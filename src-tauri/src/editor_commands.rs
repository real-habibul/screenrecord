use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TranscriptionProgress {
    job_id: String,
    state: String,
    project_id: String,
    progress: f64,
    error: Option<String>,
    cues: Option<Vec<SubtitleCue>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TranscriptionOptions {
    #[serde(default = "local_provider")]
    provider: String,
    #[serde(default)]
    api_key: String,
    source: String,
    language: String,
}
fn local_provider() -> String {
    "local".into()
}

fn subtitle_appearance_key(
    width: u32,
    height: u32,
    cue: &SubtitleCue,
    style: &SubtitleStyle,
    aspect_ratio: &str,
    crop: &Option<CropSettings>,
    word_index: Option<usize>,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(
        width,
        height,
        &cue.text,
        cue.words.iter().map(|word| &word.text).collect::<Vec<_>>(),
        style,
        aspect_ratio,
        crop,
        word_index,
    ))
    .map_err(|e| e.to_string())?;
    Ok(format!("caption-{:x}.png", Sha256::digest(bytes)))
}

impl Drop for TranscriptionOptions {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.api_key.zeroize();
    }
}

impl TranscriptionOptions {
    fn validate(&self) -> Result<(), String> {
        if !["id", "en", "auto"].contains(&self.language.as_str())
            || !["microphone", "system"].contains(&self.source.as_str())
            || !["local", "cloud"].contains(&self.provider.as_str())
            || self.provider == "cloud"
                && (!(16..=512).contains(&self.api_key.len())
                    || !self.api_key.bytes().all(|b| b.is_ascii_graphic()))
        {
            return Err("Choose an audio source, language and provider; cloud transcription requires a valid Deepgram API key.".into());
        }
        Ok(())
    }
}

#[tauri::command]
pub(super) async fn import_music(
    app: tauri::AppHandle,
    id: String,
    path: String,
    start: f64,
) -> Result<MusicClip, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let operation = state.operation.lock().map_err(|e| e.to_string())?;
        if busy(&state) {
            return Err("Wait for the current media operation before importing music.".into());
        }
        state.editor_busy.store(true, Ordering::Release);
        let _busy = BusyReset(&state.editor_busy);
        drop(operation);
        let project = load_project_metadata(&state.root, &id)?;
        crate::editor_media::import_music(&project, Path::new(&path), start, ffmpeg(&state)?)
    })
    .await
    .map_err(|_| "Music import worker failed. The source is unchanged.".to_string())?
}

#[tauri::command]
pub(super) async fn import_subtitles(
    app: tauri::AppHandle,
    id: String,
    path: String,
) -> Result<Vec<SubtitleCue>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let operation = state
            .operation
            .try_lock()
            .map_err(|_| "A media operation is starting. Try again shortly.")?;
        if busy(&state) {
            return Err("Wait for the current media operation before importing subtitles.".into());
        }
        state.editor_busy.store(true, Ordering::Release);
        let _busy = BusyReset(&state.editor_busy);
        drop(operation);
        let project = load_project_metadata(&state.root, &id)?;
        crate::editor_media::import_srt(&project, Path::new(&path))
    })
    .await
    .map_err(|_| "Subtitle import worker failed. Existing captions are unchanged.".to_string())?
}

#[tauri::command]
pub(super) async fn export_subtitles(
    app: tauri::AppHandle,
    id: String,
    output_path: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let project = load_project_metadata(&state.root, &id)?;
        crate::editor_media::export_srt(&project, Path::new(&output_path))
    })
    .await
    .map_err(|_| "Subtitle export worker failed.".to_string())?
}

#[tauri::command]
pub(super) async fn render_subtitle_preview(
    app: tauri::AppHandle,
    id: String,
    cue: SubtitleCue,
    style: SubtitleStyle,
    aspect_ratio: String,
    crop: Option<CropSettings>,
    word_index: Option<usize>,
) -> Result<String, String> {
    if word_index.is_some_and(|index| index >= cue.words.len()) {
        return Err("Choose a valid timed subtitle word.".into());
    }
    let cancel = {
        let state = app.state::<AppState>();
        let mut previous = state.preview_cancel.lock().map_err(|e| e.to_string())?;
        previous.store(true, Ordering::Release);
        let cancel = Arc::new(AtomicBool::new(false));
        *previous = cancel.clone();
        cancel
    };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.preview_write.lock().map_err(|e| e.to_string())?;
        if cancel.load(Ordering::Acquire) {
            return Err("Subtitle preview was superseded.".into());
        }
        let mut project = load_project_metadata(&state.root, &id)?;
        let indices: Vec<_> = match word_index {
            Some(first) => (first..first.saturating_add(8).min(cue.words.len()))
                .map(Some)
                .collect(),
            None => vec![None],
        };
        let keys = indices
            .iter()
            .map(|index| {
                subtitle_appearance_key(
                    project.width,
                    project.height,
                    &cue,
                    &style,
                    &aspect_ratio,
                    &crop,
                    *index,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let key = keys[0].clone();
        project.edits.subtitles = vec![cue.clone()];
        project.edits.subtitle_style = style;
        project.edits.aspect_ratio = aspect_ratio;
        project.edits.crop = crop;
        project.edits.validate(project.duration)?;
        let directory = crate::editor_media::preview_directory(&project)?;
        let output = directory.join(&key);
        if output.exists() {
            let resolved = dunce::canonicalize(&output).map_err(|e| e.to_string())?;
            if resolved.parent() != Some(directory.as_path()) || !resolved.is_file() {
                return Err("Invalid subtitle preview cache entry.".into());
            }
            return Ok(output.to_string_lossy().into_owned());
        }
        let missing: Vec<_> = indices
            .iter()
            .zip(&keys)
            .filter(|(_, key)| !directory.join(key).exists())
            .collect();
        let requested: Vec<_> = missing.iter().map(|(index, _)| **index).collect();
        let images = screenrecord_render::subtitle_previews(
            &project,
            &cue.id,
            &requested,
            ffmpeg(&state)?,
            &cancel,
        )?;
        if images.len() != missing.len() {
            return Err("Subtitle preview returned an incomplete appearance batch.".into());
        }
        for (png, (_, key)) in images.into_iter().zip(missing) {
            if cancel.load(Ordering::Acquire) {
                return Err("Subtitle preview was superseded.".into());
            }
            if png.len() > 16 * 1024 * 1024 || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err("Subtitle renderer returned an invalid or oversized preview.".into());
            }
            let mut temporary =
                tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
            temporary.write_all(&png).map_err(|e| e.to_string())?;
            temporary
                .persist_noclobber(directory.join(key))
                .map_err(|e| e.to_string())?;
        }
        let mut owned: Vec<_> = fs::read_dir(&directory)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.len() != 76
                    || !name.starts_with("caption-")
                    || !name.ends_with(".png")
                    || !name[8..72].bytes().all(|b| b.is_ascii_hexdigit())
                    || entry.path() == output
                {
                    return None;
                }
                let metadata = fs::symlink_metadata(entry.path()).ok()?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return None;
                }
                Some((metadata.modified().ok(), entry.path(), metadata.len()))
            })
            .collect();
        owned.sort_by_key(|entry| entry.0);
        let mut count = owned.len() + 1;
        let mut bytes = owned.iter().map(|entry| entry.2).sum::<u64>()
            + fs::metadata(&output).map_err(|e| e.to_string())?.len();
        for (_, path, length) in owned {
            if count <= 128 && bytes <= 64 * 1024 * 1024 {
                break;
            }
            if fs::remove_file(path).is_ok() {
                count -= 1;
                bytes = bytes.saturating_sub(length);
            }
        }
        Ok(output.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "Subtitle preview worker failed.".to_string())?
}

fn publish(app: &tauri::AppHandle, progress: TranscriptionProgress) {
    if let Ok(mut current) = app.state::<AppState>().transcription_progress.lock() {
        *current = Some(progress.clone());
    }
    let _ = app.emit("transcription-progress", progress);
}

#[tauri::command]
pub(super) async fn start_transcription(
    app: tauri::AppHandle,
    id: String,
    job_id: String,
    mut options: TranscriptionOptions,
) -> Result<(), String> {
    validate_id(&id)?;
    validate_id(&job_id)?;
    options.validate()?;
    if options.provider == "local" {
        use zeroize::Zeroize;
        options.api_key.zeroize();
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let state = app.state::<AppState>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "A media operation is starting. Try again shortly.")?;
        if busy(&state) {
            return Err("Wait for the current media operation before generating subtitles.".into());
        }
        let mut slot = state
            .transcription_cancel
            .lock()
            .map_err(|e| e.to_string())?;
        *slot = Some((job_id.clone(), cancel.clone()));
        state.editor_busy.store(true, Ordering::Release);
        publish(
            &app,
            TranscriptionProgress {
                job_id: job_id.clone(),
                state: "running".into(),
                project_id: id.clone(),
                progress: 0.0,
                error: None,
                cues: None,
            },
        );
    }
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let _busy = BusyReset(&state.editor_busy);
        let prepare_app = app.clone();
        let prepare_id = id.clone();
        let prepare_cancel = cancel.clone();
        let provider = options.provider.clone();
        let local_directory = state.local_transcription_root.clone();
        let prepared = tauri::async_runtime::spawn_blocking(move || {
            let state = prepare_app.state::<AppState>();
            prepare_cancellable(&prepare_cancel, || {
                let project = load_project_metadata(&state.root, &prepare_id)?;
                let audio = match options.source.as_str() {
                    "microphone" => project.microphone_path.as_ref(),
                    "system" => project.system_audio_path.as_ref(),
                    _ => None,
                }
                .ok_or("The selected recording has no audio for this source.")?;
                let input = screenrecord_transcribe::TranscriptionInput {
                    audio_path: PathBuf::from(audio),
                    ffmpeg_path: ffmpeg(&state)?.to_path_buf(),
                    start: project.edits.trim_start,
                    end: project.edits.trim_end,
                    ranges: kept_intervals(&project.edits, project.duration)
                        .iter()
                        .map(|range| (range.start, range.end))
                        .collect(),
                    language: options.language.clone(),
                    api_key: std::mem::take(&mut options.api_key),
                };
                Ok((project, input))
            })
        })
        .await
        .unwrap_or_else(|_| Err("Cannot prepare subtitle generation.".into()));
        let outcome = match prepared {
            Err(error) => Err(error),
            Ok((project, input)) => {
                let reporter = app.clone();
                let reporter_id = id.clone();
                let reporter_job = job_id.clone();
                let report = move |progress| {
                    publish(
                        &reporter,
                        TranscriptionProgress {
                            job_id: reporter_job.clone(),
                            state: "running".into(),
                            project_id: reporter_id.clone(),
                            progress,
                            error: None,
                            cues: None,
                        },
                    );
                };
                let transcribe_cancel = cancel.clone();
                let task = tauri::async_runtime::spawn(async move {
                    if provider == "local" {
                        screenrecord_transcribe::local::transcribe(
                            input,
                            local_directory,
                            transcribe_cancel,
                            report,
                        )
                        .await
                    } else {
                        screenrecord_transcribe::transcribe(input, transcribe_cancel, report).await
                    }
                });
                task.await
                    .unwrap_or_else(|_| {
                        Err(
                            "Subtitle generation worker failed. Existing subtitles are unchanged."
                                .into(),
                        )
                    })
                    .and_then(|cues| {
                        if cues.is_empty() {
                            return Err(
                                "No speech was detected. Existing subtitles are unchanged."
                                    .to_string(),
                            );
                        }
                        let mut edits = project.edits;
                        edits.subtitles = cues.clone();
                        edits.validate(project.duration)?;
                        Ok(cues)
                    })
            }
        };
        let (status, cues, error) = if cancel.load(Ordering::Acquire) {
            ("cancelled", None, None)
        } else {
            match outcome {
                Ok(cues) => ("completed", Some(cues), None),
                Err(error) => ("failed", None, Some(error)),
            }
        };
        drop(_busy);
        publish(
            &app,
            TranscriptionProgress {
                job_id,
                state: status.into(),
                project_id: id,
                progress: if status == "completed" { 1.0 } else { 0.0 },
                error,
                cues,
            },
        );
    });
    Ok(())
}
#[tauri::command]
pub(super) fn cancel_transcription(state: tauri::State<'_, AppState>, job_id: String) {
    cancel_job(&state.transcription_cancel, &job_id);
}

fn cancel_job(slot: &Mutex<Option<(String, Arc<AtomicBool>)>>, job_id: &str) {
    if let Ok(current) = slot.lock() {
        if let Some((_, cancel)) = current.as_ref().filter(|(id, _)| id == job_id) {
            cancel.store(true, Ordering::Release);
        }
    }
}

fn prepare_cancellable<T>(
    cancel: &AtomicBool,
    prepare: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Subtitle generation cancelled.".into());
    }
    let value = prepare()?;
    if cancel.load(Ordering::Acquire) {
        return Err("Subtitle generation cancelled.".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_provider_is_local_and_cloud_requires_explicit_valid_credentials() {
        let mut options: TranscriptionOptions =
            serde_json::from_value(serde_json::json!({"source":"microphone","language":"id"}))
                .unwrap();
        assert_eq!(options.provider, "local");
        assert!(options.api_key.is_empty());
        options.validate().unwrap();
        options.provider = "cloud".into();
        assert!(options.validate().is_err());
        options.api_key = "a".repeat(16);
        options.validate().unwrap();
        options.provider = "unknown".into();
        assert!(options.validate().is_err());
    }

    #[test]
    fn glyph_cache_keys_ignore_timing_but_include_word_segmentation_and_active_word() {
        let mut style = SubtitleStyle {
            highlight_words: true,
            ..SubtitleStyle::default()
        };
        let mut cue = SubtitleCue {
            id: "original".into(),
            start: 0.0,
            end: 1.0,
            text: "one two three".into(),
            words: vec![
                SubtitleWord {
                    start: 0.0,
                    end: 0.2,
                    text: "one".into(),
                },
                SubtitleWord {
                    start: 0.2,
                    end: 0.5,
                    text: "two three".into(),
                },
            ],
            source_ranges: vec![],
        };
        let key =
            subtitle_appearance_key(1920, 1080, &cue, &style, "source", &None, Some(1)).unwrap();
        cue.id = "retimed".into();
        cue.start = 0.01;
        cue.end = 0.9;
        cue.words[0].start = 0.01;
        cue.source_ranges = vec![SubtitleRange {
            start: 0.01,
            end: 0.9,
        }];
        assert_eq!(
            key,
            subtitle_appearance_key(1920, 1080, &cue, &style, "source", &None, Some(1)).unwrap()
        );
        assert_ne!(
            key,
            subtitle_appearance_key(1920, 1080, &cue, &style, "source", &None, None).unwrap()
        );
        cue.words[0].text = "one two".into();
        cue.words[1].text = "three".into();
        assert_ne!(
            key,
            subtitle_appearance_key(1920, 1080, &cue, &style, "source", &None, Some(1)).unwrap()
        );
        style.highlight_color = "#FF0000".into();
        assert_ne!(
            key,
            subtitle_appearance_key(1920, 1080, &cue, &style, "source", &None, Some(1)).unwrap()
        );
    }

    #[test]
    fn cancel_during_preparation_prevents_dispatch_and_stale_cancel_is_ignored() {
        let cancel = Arc::new(AtomicBool::new(false));
        let slot = Mutex::new(Some(("new-job".into(), cancel.clone())));
        cancel_job(&slot, "old-job");
        assert!(!cancel.load(Ordering::Acquire));
        let (started, ready) = std::sync::mpsc::sync_channel(0);
        let (release, wait) = std::sync::mpsc::sync_channel(0);
        let dispatched = Arc::new(AtomicBool::new(false));
        let observed = dispatched.clone();
        let worker = std::thread::spawn(move || {
            prepare_cancellable(&cancel, || {
                started.send(()).unwrap();
                wait.recv().unwrap();
                Ok(())
            })
            .map(|_| {
                observed.store(true, Ordering::Release);
            })
        });
        ready.recv().unwrap();
        cancel_job(&slot, "new-job");
        release.send(()).unwrap();
        assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
        assert!(!dispatched.load(Ordering::Acquire));
    }
}

#[tauri::command]
pub(super) fn get_transcription_progress(
    state: tauri::State<'_, AppState>,
) -> Option<TranscriptionProgress> {
    state
        .transcription_progress
        .lock()
        .ok()
        .and_then(|p| p.clone())
}
