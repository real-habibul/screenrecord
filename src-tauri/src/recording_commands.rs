use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};

pub(super) fn state_for_slot(slot: &RecordingSlot) -> RecordingState {
    let paused = slot.recorder.is_paused();
    RecordingState {
        active: true,
        paused,
        camera_available: slot.pending.options.camera_id.is_some(),
        phase: if paused { "paused" } else { "recording" }.into(),
        elapsed: slot.recorder.elapsed(),
        project_id: Some(slot.pending.id.clone()),
    }
}

pub(super) fn remember_state(app: &tauri::AppHandle, value: RecordingState) {
    if let Ok(mut state) = app.state::<AppState>().recording_status.lock() {
        *state = value.clone();
    }
    let _ = app.emit("recording-state-changed", value);
}

pub(super) fn watch_health(app: tauri::AppHandle, project_id: String) {
    tauri::async_runtime::spawn_blocking(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let fault = {
            let state = app.state::<AppState>();
            let Ok(slot) = state.recording.try_lock() else {
                continue;
            };
            let Some(slot) = slot.as_ref().filter(|slot| slot.pending.id == project_id) else {
                break;
            };
            slot.recorder.health().err()
        };
        if let Some(error) = fault {
            if let Err(reason) = crate::recording_failure::write(
                &app.state::<AppState>().root,
                &project_id,
                "health",
                &error,
                None,
            ) {
                log::warn!("Cannot save recording health failure: {reason}");
            }
            report_recovery_issue(
                &app,
                format!("Recording is stopping to preserve available media: {error}"),
            );
            stop_from_system(&app, Some(project_id.clone()));
            break;
        }
    });
}

fn take_for_project<T>(
    slot: &mut Option<T>,
    expected: Option<&str>,
    identity: impl Fn(&T) -> &str,
) -> Result<T, String> {
    let current = slot.as_ref().ok_or("There is no active recording.")?;
    if expected.is_some_and(|expected| expected != identity(current)) {
        return Err("This recording has changed. Refresh the recording controls.".into());
    }
    slot.take()
        .ok_or_else(|| "There is no active recording.".into())
}

pub(super) fn take_slot(
    slot: &mut Option<RecordingSlot>,
    expected: Option<&str>,
) -> Result<RecordingSlot, String> {
    take_for_project(slot, expected, |slot| &slot.pending.id)
}

fn change_pause(
    app: &tauri::AppHandle,
    project_id: &str,
    pause: bool,
) -> Result<RecordingState, String> {
    let state = app.state::<AppState>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "A recording operation is already in progress.")?;
    if state.recording_busy.swap(true, Ordering::AcqRel) {
        return Err("A recording operation is already in progress.".into());
    }
    let _busy = BusyReset(&state.recording_busy);
    let mut slot = state.recording.lock().map_err(|e| e.to_string())?;
    let slot = slot.as_mut().ok_or("There is no active recording.")?;
    if slot.pending.id != project_id {
        return Err("This recording has changed. Refresh the recording controls.".into());
    }
    let result = if pause {
        slot.recorder.pause()
    } else {
        slot.recorder.resume()
    };
    let current = state_for_slot(slot);
    remember_state(app, current.clone());
    result?;
    Ok(current)
}

#[tauri::command]
pub(super) async fn pause_recording(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<RecordingState, String> {
    validate_id(&project_id)?;
    tauri::async_runtime::spawn_blocking(move || change_pause(&app, &project_id, true))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(super) async fn resume_recording(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<RecordingState, String> {
    validate_id(&project_id)?;
    tauri::async_runtime::spawn_blocking(move || change_pause(&app, &project_id, false))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecordingCameraFrame {
    generation: u32,
    sequence: u64,
    mime_type: &'static str,
    data: Option<String>,
    active: bool,
    error: Option<String>,
}

#[tauri::command]
pub(super) async fn get_recording_camera_frame(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<RecordingCameraFrame, String> {
    validate_id(&project_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let source = {
            let state = app.state::<AppState>();
            let slot = state.recording.lock().map_err(|e| e.to_string())?;
            slot.as_ref()
                .filter(|slot| slot.pending.id == project_id)
                .map(|slot| (slot.recorder.generation(), slot.recorder.preview_snapshot()))
        };
        match source {
            Some((generation, frame)) => Ok(RecordingCameraFrame {
                generation,
                sequence: frame.sequence,
                mime_type: "image/jpeg",
                data: frame.jpeg.map(|jpeg| STANDARD.encode(jpeg)),
                active: frame.active,
                error: frame.error,
            }),
            None => Ok(RecordingCameraFrame {
                generation: 0,
                sequence: 0,
                mime_type: "image/jpeg",
                data: None,
                active: false,
                error: None,
            }),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn joined_cursor(parts: &[CaptureResult], offsets: &[f64]) -> Result<Vec<CursorSample>, String> {
    if offsets.len() != parts.len() {
        return Err("Recording part offsets are incomplete.".into());
    }
    let maximum = parts
        .iter()
        .try_fold(parts.len().saturating_sub(1), |count, part| {
            count.checked_add(part.cursor.len())
        })
        .filter(|count| *count <= 16_000_000)
        .ok_or("Cursor metadata exceeds the supported recording length.")?;
    let mut cursor = Vec::with_capacity(maximum);
    for (index, (part, offset)) in parts.iter().zip(offsets).enumerate() {
        let mut samples = part
            .cursor
            .iter()
            .filter(|sample| sample.time >= 0.0 && sample.time < part.duration)
            .peekable();
        if index > 0 {
            let first = samples.peek();
            cursor.push(CursorSample {
                time: *offset,
                x: 0.0,
                y: 0.0,
                pressed: first.is_some_and(|s| s.pressed),
                buttons: first.map_or(0, |s| s.buttons),
                visible: false,
            });
        }
        cursor.extend(samples.map(|sample| CursorSample {
            time: offset + sample.time,
            ..*sample
        }));
    }
    Ok(cursor)
}

pub(super) fn finish_session(
    app: &tauri::AppHandle,
    pending: &PendingProject,
    mut parts: Vec<CaptureResult>,
    status: &str,
) -> Result<Project, String> {
    let state = app.state::<AppState>();
    if parts.len() == 1 {
        return finish_project(&state.root, pending, parts.remove(0), status);
    }
    if parts.is_empty() {
        return Err("There is no complete recording part to save.".into());
    }
    let input: Vec<_> = parts
        .iter()
        .map(|part| screenrecord_render::RecordingPart {
            screen_path: part.screen_path.clone(),
            microphone_path: part.microphone_path.clone(),
            system_audio_path: part.system_audio_path.clone(),
            camera_path: part.camera_path.clone(),
            camera_offset_seconds: part.camera_offset_seconds,
            width: part.width,
            height: part.height,
            fps: part.fps,
            duration: part.duration,
        })
        .collect();
    let directory = project_directory(&state.root, &pending.id)?;
    let reporter = app.clone();
    let project_id = pending.id.clone();
    let assembled = screenrecord_render::assemble_recording(
        &input,
        &directory,
        ffmpeg(&state)?,
        Arc::new(AtomicBool::new(false)),
        move |progress| {
            let _ = reporter.emit(
                "recording-finalization-progress",
                serde_json::json!({ "projectId": project_id, "progress": progress }),
            );
        },
    )?;
    let cursor = joined_cursor(&parts, &assembled.part_offsets)?;
    let captured = CaptureResult {
        screen_path: assembled.screen_path,
        microphone_path: assembled.microphone_path,
        system_audio_path: assembled.system_audio_path,
        camera_path: assembled.camera_path,
        camera_offset_seconds: assembled.camera_offset_seconds,
        width: assembled.width,
        height: assembled.height,
        fps: assembled.fps,
        duration: assembled.duration,
        cursor,
    };
    finish_project_with_intervals(
        &state.root,
        pending,
        captured,
        assembled.camera_intervals,
        status,
    )
}

pub(super) fn recover_recording(
    app: &tauri::AppHandle,
    pending: &PendingProject,
    directory: &Path,
) -> Result<Project, String> {
    let state = app.state::<AppState>();
    if directory.join("session-journal.json").is_file() {
        let parts = screenrecord_capture::session::recover_session(directory, ffmpeg(&state)?)?;
        finish_session(app, pending, parts, "interrupted")
    } else {
        let captured = screenrecord_capture::recover(directory, ffmpeg(&state)?)?;
        finish_project(&state.root, pending, captured, "interrupted")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_stop_cannot_take_a_new_recording() {
        let mut slot = Some("new-project".to_string());
        assert!(take_for_project(&mut slot, Some("old-project"), |value| value.as_str()).is_err());
        assert_eq!(slot.as_deref(), Some("new-project"));
        assert_eq!(
            take_for_project(&mut slot, Some("new-project"), |value| value.as_str()).unwrap(),
            "new-project"
        );
        assert!(slot.is_none());
    }
    #[test]
    fn part_cursor_tails_are_removed_and_resume_resets_visibility_before_the_new_sample() {
        let part = |times: &[f64]| CaptureResult {
            screen_path: PathBuf::new(),
            microphone_path: None,
            system_audio_path: None,
            camera_path: None,
            camera_offset_seconds: 0.0,
            width: 2,
            height: 2,
            fps: 30,
            duration: 1.0,
            cursor: times
                .iter()
                .map(|time| CursorSample {
                    time: *time,
                    x: 0.5,
                    y: 0.5,
                    pressed: true,
                    buttons: 2,
                    visible: true,
                })
                .collect(),
        };
        let result = joined_cursor(
            &[part(&[0.0, 0.9, 1.0, 1.02]), part(&[0.0, 0.2])],
            &[0.0, 1.0],
        )
        .unwrap();
        assert_eq!(
            result.iter().map(|sample| sample.time).collect::<Vec<_>>(),
            vec![0.0, 0.9, 1.0, 1.0, 1.2]
        );
        assert!(!result[2].visible && result[2].pressed);
        assert!(result[3].visible);
        assert_eq!(
            result.iter().map(|s| s.buttons).collect::<Vec<_>>(),
            [2, 2, 2, 2, 2]
        );
        assert!(cursor_clicks(&result).next().is_none());
    }
}
