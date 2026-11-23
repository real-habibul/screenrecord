use super::*;
use screenrecord_capture::{
    analysis::{analyze_cached, AnalysisCachePolicy, AudioAnalysis, AudioAnalysisInput},
    meter::{MeterSnapshot, MicrophoneMeter},
};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct AnalysisState {
    request: Option<(String, Arc<AtomicBool>)>,
    pending_cancels: VecDeque<String>,
}

impl AnalysisState {
    fn reserve(&mut self, id: String) -> Result<Arc<AtomicBool>, String> {
        if self.request.is_some() {
            return Err("Audio analysis is still finishing. Try again shortly.".into());
        }
        if let Some(index) = self
            .pending_cancels
            .iter()
            .position(|cancelled| cancelled == &id)
        {
            self.pending_cancels.remove(index);
            return Err("Audio analysis cancelled.".into());
        }
        let token = Arc::new(AtomicBool::new(false));
        self.request = Some((id, token.clone()));
        Ok(token)
    }
    fn cancel(&mut self, id: &str) {
        if let Some((current, token)) = &self.request {
            if current == id {
                token.store(true, Ordering::Release);
                return;
            }
        }
        if !self.pending_cancels.iter().any(|cancelled| cancelled == id) {
            if self.pending_cancels.len() >= 128 {
                self.pending_cancels.pop_front();
            }
            self.pending_cancels.push_back(id.to_owned());
        }
    }
    fn finish(&mut self, id: &str) {
        if self
            .request
            .as_ref()
            .is_some_and(|(current, _)| current == id)
        {
            self.request = None;
        }
    }
}

struct AnalysisReset<'a> {
    state: &'a AppState,
    job_id: &'a str,
}
impl Drop for AnalysisReset<'_> {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.state.audio_analysis.lock() {
            slot.finish(self.job_id);
        }
        self.state.editor_busy.store(false, Ordering::Release);
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisProgress {
    job_id: String,
    project_id: String,
    source: String,
    progress: f64,
}

fn analysis_input(
    project: &Project,
    source: &str,
    engine: &Path,
) -> Result<AudioAnalysisInput, String> {
    let kept = kept_intervals(&project.edits, project.duration);
    let (path, duration, ranges) = match source {
        "microphone" => (
            project.microphone_path.as_ref(),
            project.duration,
            kept.iter().map(|range| (range.start, range.end)).collect(),
        ),
        "system" => (
            project.system_audio_path.as_ref(),
            project.duration,
            kept.iter().map(|range| (range.start, range.end)).collect(),
        ),
        _ => {
            let clip = source
                .strip_prefix("music:")
                .and_then(|id| project.edits.music.iter().find(|clip| clip.id == id))
                .ok_or("Select a recorded audio source or an imported music clip.")?;
            let ranges: Vec<_> = kept
                .iter()
                .filter_map(|range| {
                    let start = range.start.max(clip.start);
                    let end = range.end.min(clip.start + clip.duration);
                    if end <= start {
                        return None;
                    }
                    let from =
                        (clip.source_start + start - clip.start).clamp(0.0, clip.source_duration);
                    let to =
                        (clip.source_start + end - clip.start).clamp(0.0, clip.source_duration);
                    (to > from).then_some((from, to))
                })
                .collect();
            if ranges.is_empty() {
                return Err("This music clip is outside the edited video.".into());
            }
            (Some(&clip.path), clip.source_duration, ranges)
        }
    };
    Ok(AudioAnalysisInput {
        audio_path: PathBuf::from(path.ok_or("This source was not recorded.")?),
        ffmpeg_path: engine.to_owned(),
        source_duration: duration,
        retained_ranges: ranges,
        bins: 4096,
    })
}

#[tauri::command]
pub(super) async fn analyze_audio(
    app: tauri::AppHandle,
    id: String,
    source: String,
    job_id: String,
    force_refresh: Option<bool>,
) -> Result<AudioAnalysis, String> {
    validate_id(&id)?;
    validate_id(&job_id)?;
    let token = {
        let state = app.state::<AppState>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "A media operation is starting. Try again shortly.")?;
        if busy(&state) {
            return Err("Wait for the current media operation before analyzing audio.".into());
        }
        let token = state
            .audio_analysis
            .lock()
            .map_err(|e| e.to_string())?
            .reserve(job_id.clone())?;
        state.editor_busy.store(true, Ordering::Release);
        token
    };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _reset = AnalysisReset {
            state: &state,
            job_id: &job_id,
        };
        if token.load(Ordering::Acquire) {
            return Err("Audio analysis cancelled.".into());
        }
        let project = load_project_metadata(&state.root, &id)?;
        let input = analysis_input(&project, &source, ffmpeg(&state)?)?;
        if token.load(Ordering::Acquire) {
            return Err("Audio analysis cancelled.".into());
        }
        let reporter = app.clone();
        let reporter_job = job_id.clone();
        let directory = Path::new(&project.directory).join(".audio-analysis");
        let policy = if force_refresh.unwrap_or(false) {
            AnalysisCachePolicy::Refresh
        } else {
            AnalysisCachePolicy::Reuse
        };
        let result = analyze_cached(input, &directory, policy, token.clone(), move |progress| {
            if progress.is_finite() {
                let _ = reporter.emit(
                    "audio-analysis-progress",
                    AnalysisProgress {
                        job_id: reporter_job.clone(),
                        project_id: id.clone(),
                        source: source.clone(),
                        progress: progress.clamp(0.0, 1.0),
                    },
                );
            }
        });
        if token.load(Ordering::Acquire) {
            return Err("Audio analysis cancelled.".into());
        }
        result
    })
    .await
    .map_err(|_| "Audio analysis worker failed. The recordings are unchanged.".to_string())?
}

#[tauri::command]
pub(super) fn cancel_audio_analysis(
    state: tauri::State<'_, AppState>,
    job_id: String,
) -> Result<(), String> {
    validate_id(&job_id)?;
    state
        .audio_analysis
        .lock()
        .map_err(|e| e.to_string())?
        .cancel(&job_id);
    Ok(())
}

#[derive(Default)]
pub(super) struct MicrophoneMeterState {
    request: Option<(String, Arc<AtomicBool>)>,
    resource: Option<(String, MicrophoneMeter)>,
}

impl MicrophoneMeterState {
    fn reserve(&mut self, id: String) -> Arc<AtomicBool> {
        if let Some((_, token)) = &self.request {
            token.store(true, Ordering::Release);
        }
        let token = Arc::new(AtomicBool::new(false));
        self.request = Some((id, token.clone()));
        token
    }
    fn revoke(&mut self, id: Option<&str>) {
        if let Some((current, token)) = &self.request {
            if id.is_none_or(|id| id == current) {
                token.store(true, Ordering::Release);
            }
        }
    }
}

pub(super) fn stop_meter(state: &AppState) -> Result<(), String> {
    let resource = {
        let mut slot = state.microphone_meter.lock().map_err(|e| e.to_string())?;
        slot.revoke(None);
        slot.request = None;
        slot.resource.take()
    };
    if let Some((_, mut resource)) = resource {
        resource.stop()?;
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn start_microphone_meter(
    app: tauri::AppHandle,
    microphone_id: String,
    session_id: String,
) -> Result<(), String> {
    validate_id(&session_id)?;
    let token = {
        let state = app.state::<AppState>();
        if busy(&state) {
            return Err("Stop the current media operation before testing a microphone.".into());
        }
        if !state
            .known_devices
            .lock()
            .map_err(|e| e.to_string())?
            .microphone_available(&microphone_id)
        {
            return Err(
                "Refresh devices and select an available microphone before testing.".into(),
            );
        }
        let token = state
            .microphone_meter
            .lock()
            .map_err(|e| e.to_string())?
            .reserve(session_id.clone());
        token
    };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state.operation.lock().map_err(|e| e.to_string())?;
        if token.load(Ordering::Acquire) {
            return Err("Microphone test cancelled.".into());
        }
        let previous = state
            .microphone_meter
            .lock()
            .map_err(|e| e.to_string())?
            .resource
            .take();
        if let Some((_, mut previous)) = previous {
            previous.stop()?;
        }
        if busy(&state) {
            return Err("Stop the current media operation before testing a microphone.".into());
        }
        if token.load(Ordering::Acquire) {
            return Err("Microphone test cancelled.".into());
        }
        let mut resource = MicrophoneMeter::start(&microphone_id)?;
        let mut slot = state.microphone_meter.lock().map_err(|e| e.to_string())?;
        if token.load(Ordering::Acquire) {
            drop(slot);
            resource.stop()?;
            return Err("Microphone test cancelled.".into());
        }
        slot.resource = Some((session_id, resource));
        Ok(())
    })
    .await
    .map_err(|_| "Microphone test worker failed.".to_string())?
}

#[tauri::command]
pub(super) async fn stop_microphone_meter(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        state
            .microphone_meter
            .lock()
            .map_err(|e| e.to_string())?
            .revoke(Some(&session_id));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state.operation.lock().map_err(|e| e.to_string())?;
        let resource = {
            let mut slot = state.microphone_meter.lock().map_err(|e| e.to_string())?;
            if slot
                .resource
                .as_ref()
                .is_some_and(|(id, _)| id == &session_id)
            {
                slot.resource.take()
            } else {
                None
            }
        };
        if let Some((_, mut resource)) = resource {
            resource.stop()?;
        }
        Ok(())
    })
    .await
    .map_err(|_| "Microphone test cleanup failed.".to_string())?
}

#[tauri::command]
pub(super) fn get_microphone_meter(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<MeterSnapshot, String> {
    let slot = state.microphone_meter.lock().map_err(|e| e.to_string())?;
    let current = slot
        .request
        .as_ref()
        .is_some_and(|(id, token)| id == &session_id && !token.load(Ordering::Acquire));
    Ok(slot
        .resource
        .as_ref()
        .filter(|(id, _)| id == &session_id && current)
        .map(|(_, resource)| resource.snapshot())
        .unwrap_or(MeterSnapshot {
            sequence: 0,
            peak: 0.0,
            rms: 0.0,
            active: false,
            ready: false,
            error: None,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_sources_use_owned_media_and_music_ranges_follow_retained_video() {
        let mut project = Project {
            version: PROJECT_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: "Analysis fixture".into(),
            created_at: "2026-11-11T00:00:00+07:00".into(),
            directory: "owned".into(),
            screen_path: "screen.mp4".into(),
            microphone_path: Some("microphone.wav".into()),
            system_audio_path: None,
            camera_path: None,
            camera_offset_seconds: 0.0,
            camera_intervals: None,
            width: 1920,
            height: 1080,
            fps: 30,
            duration: 10.0,
            cursor: vec![],
            edits: EditSettings::new(10.0),
            status: "ready".into(),
        };
        project.edits.video_clips = vec![
            VideoClip {
                id: "a".into(),
                start: 2.0,
                end: 4.0,
            },
            VideoClip {
                id: "b".into(),
                start: 6.0,
                end: 8.0,
            },
        ];
        project.edits.music.push(MusicClip {
            id: "music".into(),
            name: "Music fixture".into(),
            path: "owned-music.wav".into(),
            source_duration: 8.0,
            start: 3.0,
            source_start: 1.0,
            duration: 4.0,
            volume: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
        });
        let recording = analysis_input(&project, "microphone", Path::new("ffmpeg.exe")).unwrap();
        assert_eq!(recording.retained_ranges, vec![(2.0, 4.0), (6.0, 8.0)]);
        assert_eq!(recording.audio_path, Path::new("microphone.wav"));
        let music = analysis_input(&project, "music:music", Path::new("ffmpeg.exe")).unwrap();
        assert_eq!(music.retained_ranges, vec![(1.0, 2.0), (4.0, 5.0)]);
        assert_eq!(music.source_duration, 8.0);
        assert!(analysis_input(&project, "system", Path::new("ffmpeg.exe")).is_err());
        assert!(analysis_input(&project, "C:\\outside.wav", Path::new("ffmpeg.exe")).is_err());
        project.edits.music[0].source_duration = 5.0 - f64::EPSILON * 4.0;
        let music = analysis_input(&project, "music:music", Path::new("ffmpeg.exe")).unwrap();
        assert!(music.retained_ranges.last().unwrap().1 <= music.source_duration);
        project.edits.music[0].start = 8.0;
        assert!(analysis_input(&project, "music:music", Path::new("ffmpeg.exe")).is_err());
    }
    #[test]
    fn analysis_cancellation_is_registered_and_old_jobs_cannot_cancel_or_clear_new_jobs() {
        let mut state = AnalysisState::default();
        let first = state.reserve("first".into()).unwrap();
        assert!(state.reserve("second".into()).is_err());
        state.cancel("other");
        assert!(!first.load(Ordering::Acquire));
        state.cancel("first");
        assert!(first.load(Ordering::Acquire));
        state.finish("first");
        let second = state.reserve("second".into()).unwrap();
        state.cancel("first");
        state.finish("first");
        assert!(!second.load(Ordering::Acquire));
        assert!(state.request.is_some());
    }

    #[test]
    fn cancellation_before_async_reservation_is_consumed_without_starting_work_and_is_bounded() {
        let mut state = AnalysisState::default();
        state.cancel("early");
        assert!(state
            .reserve("early".into())
            .unwrap_err()
            .contains("cancelled"));
        assert!(state.request.is_none());
        for index in 0..300 {
            state.cancel(&format!("cancelled-{index}"));
        }
        assert_eq!(state.pending_cancels.len(), 128);
        assert!(state.reserve("cancelled-299".into()).is_err());
        assert!(state.reserve("fresh".into()).is_ok());
    }
    #[test]
    fn microphone_sessions_revoke_pending_start_and_ignore_late_old_stop() {
        let mut state = MicrophoneMeterState::default();
        let first = state.reserve("first".into());
        let second = state.reserve("second".into());
        assert!(first.load(Ordering::Acquire));
        state.revoke(Some("first"));
        assert!(!second.load(Ordering::Acquire));
        state.revoke(None);
        assert!(second.load(Ordering::Acquire));
    }
}
