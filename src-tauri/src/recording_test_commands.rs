use super::*;
use crate::recording_test_store::{validate_trial_id, TrialDirectory, CANCELLED};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::time::{Duration, Instant};

const TEST_LENGTH: Duration = Duration::from_secs(5);

struct HiddenSetup {
    window: tauri::WebviewWindow,
    restore: bool,
}

impl HiddenSetup {
    fn enter(app: &tauri::AppHandle) -> Result<Self, String> {
        let window = app
            .get_webview_window("main")
            .ok_or("Recording setup is unavailable.")?;
        let restore = !window.is_minimized().map_err(|e| e.to_string())?;
        let guard = Self { window, restore };
        recording_controls::protect(&guard.window, true)?;
        if restore {
            guard.window.minimize().map_err(|e| e.to_string())?;
        }
        Ok(guard)
    }
}

impl Drop for HiddenSetup {
    fn drop(&mut self) {
        let _ = recording_controls::protect(&self.window, false);
        if self.restore {
            let _ = self.window.unminimize();
            let _ = self.window.show();
            let _ = self.window.set_focus();
        }
    }
}

#[derive(Serialize)]
pub(super) struct SourceImage {
    width: u32,
    height: u32,
    data: String,
}

#[tauri::command]
pub(super) async fn get_source_thumbnail(
    app: tauri::AppHandle,
    source_id: String,
) -> Result<SourceImage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "Recording setup is busy. Try the frame again.")?;
        if busy(&state) || screenrecord_capture::cleanup_in_progress() {
            return Err("Wait for the current media operation before choosing a frame.".into());
        }
        state.recording_busy.store(true, Ordering::Release);
        let _busy = BusyReset(&state.recording_busy);
        screenrecord_capture::validate_source(&source_id)?;
        let _setup = if source_id.starts_with("monitor:") {
            Some(HiddenSetup::enter(&app)?)
        } else {
            None
        };
        let image = screenrecord_capture::source_thumbnail(&source_id)?;
        Ok(SourceImage {
            width: image.width,
            height: image.height,
            data: STANDARD.encode(image.jpeg),
        })
    })
    .await
    .map_err(|_| "Could not read the selected source.".to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TrialMedia {
    id: String,
    duration: f64,
    width: u32,
    height: u32,
    screen_path: String,
    camera_path: Option<String>,
    camera_offset_seconds: f64,
    microphone_path: Option<String>,
    system_audio_path: Option<String>,
}

fn checked_media_path(part: &Path, path: &Path, allowed_names: &[&str]) -> Result<String, String> {
    let actual = dunce::canonicalize(path)
        .map_err(|e| format!("Cannot open the recorded test track: {e}"))?;
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    use std::os::windows::fs::MetadataExt;
    if !meta.is_file()
        || meta.file_attributes() & 0x400 != 0
        || meta.len() == 0
        || actual.parent() != Some(part)
        || !actual
            .file_name()
            .and_then(|v| v.to_str())
            .is_some_and(|v| allowed_names.contains(&v))
    {
        return Err("A test track is missing or outside the temporary recording.".into());
    }
    Ok(actual.to_string_lossy().into_owned())
}

impl TrialMedia {
    fn from_capture(
        id: &str,
        directory: &Path,
        options: &RecordingOptions,
        result: CaptureResult,
    ) -> Result<Self, String> {
        let directory = dunce::canonicalize(directory).map_err(|e| e.to_string())?;
        let part = directory.join("part-00000");
        if !((TEST_LENGTH.as_secs_f64() - 0.15)..=(TEST_LENGTH.as_secs_f64() + 0.50))
            .contains(&result.duration)
        {
            return Err("The test could not produce a five-second clip. Check the selected source and retry.".into());
        }
        if dunce::canonicalize(&part).map_err(|e| e.to_string())? != part
            || result.width < 2
            || result.height < 2
            || result.fps != options.fps
            || result.camera_path.is_some() != options.camera_id.is_some()
            || result.microphone_path.is_some() != options.microphone_id.is_some()
            || result.system_audio_path.is_some() != options.system_audio
            || !result.camera_offset_seconds.is_finite()
            || options
                .region
                .is_some_and(|r| (r.width, r.height) != (result.width, result.height))
        {
            return Err("The test did not produce the selected frame and tracks.".into());
        }
        let track = |path: Option<&Path>, names: &[&str]| {
            path.map(|p| checked_media_path(&part, p, names))
                .transpose()
        };
        Ok(Self {
            id: id.into(),
            duration: result.duration,
            width: result.width,
            height: result.height,
            screen_path: checked_media_path(&part, &result.screen_path, &["screen.mp4"])?,
            camera_path: track(result.camera_path.as_deref(), &["camera-source.mp4"])?,
            camera_offset_seconds: result.camera_offset_seconds,
            microphone_path: track(result.microphone_path.as_deref(), &["microphone.wav"])?,
            system_audio_path: track(result.system_audio_path.as_deref(), &["system.wav"])?,
        })
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

fn capture_interval(
    cancel: &AtomicBool,
    mut elapsed: impl FnMut() -> f64,
    mut health: impl FnMut() -> Result<(), String>,
    mut wait: impl FnMut(Duration),
) -> Result<(), String> {
    let start = Instant::now();
    loop {
        check_cancel(cancel)?;
        health()?;
        let seconds = elapsed();
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("The recording test clock stopped.".into());
        }
        let remaining = TEST_LENGTH.as_secs_f64() - seconds;
        if remaining <= 0.0 {
            return Ok(());
        }
        if start.elapsed() > TEST_LENGTH + Duration::from_millis(300) {
            return Err("The selected source did not advance for five seconds.".into());
        }
        wait(Duration::from_secs_f64(remaining).min(Duration::from_millis(35)));
    }
}

struct TrialLease {
    app: tauri::AppHandle,
    id: String,
}

impl Drop for TrialLease {
    fn drop(&mut self) {
        let state = self.app.state::<AppState>();
        if let Ok(mut trials) = state.trials.lock() {
            trials.finish(&self.id);
        }
        state.recording_busy.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub(super) async fn run_recording_test(
    app: tauri::AppHandle,
    id: String,
    options: RecordingOptions,
) -> Result<TrialMedia, String> {
    validate_trial_id(&id)?;
    options.validate()?;
    let cancel = {
        let state = app.state::<AppState>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "Wait for the current media operation before testing.")?;
        if busy(&state) || screenrecord_capture::cleanup_in_progress() {
            return Err("Wait for the current media operation before testing.".into());
        }
        let token = state
            .trials
            .lock()
            .map_err(|e| e.to_string())?
            .reserve(&id)?;
        state.recording_busy.store(true, Ordering::Release);
        token
    };
    let lease = TrialLease {
        app: app.clone(),
        id: id.clone(),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let state = app.state::<AppState>();
        check_cancel(&cancel)?;
        screenrecord_capture::validate_recording_source(&options)?;
        let engine = ffmpeg(&state)?;
        camera_commands::stop_all(&state)?;
        audio_commands::stop_meter(&state)?;
        check_cancel(&cancel)?;
        let directory = TrialDirectory::create(&state.test_cache, &id)?;
        let setup = HiddenSetup::enter(&app)?;
        check_cancel(&cancel)?;
        let recorder = RecordingSession::start_cancellable(
            &directory.path(),
            &options,
            engine,
            cancel.clone(),
        )?;
        let phase = |name: &str| {
            let _ = app.emit(
                "recording-test-phase",
                serde_json::json!({"id": id, "phase": name}),
            );
        };
        phase("recording");
        capture_interval(
            &cancel,
            || recorder.elapsed(),
            || recorder.health(),
            std::thread::sleep,
        )?;
        phase("finalizing");
        let mut parts = recorder.stop()?;
        drop(setup);
        check_cancel(&cancel)?;
        let result = parts
            .pop()
            .filter(|_| parts.is_empty())
            .ok_or("The test must contain one screen recording.")?;
        let preview = TrialMedia::from_capture(&id, &directory.path(), &options, result)?;
        state
            .trials
            .lock()
            .map_err(|e| e.to_string())?
            .publish(directory)?;
        Ok(preview)
    })
    .await
    .map_err(|_| "The recording test worker stopped.".to_string())?
}

async fn release_test(app: tauri::AppHandle, id: String) -> Result<(), String> {
    validate_trial_id(&id)?;
    let media = app
        .state::<AppState>()
        .trials
        .lock()
        .map_err(|e| e.to_string())?
        .release(&id);
    if let Some(media) = media {
        tauri::async_runtime::spawn_blocking(move || drop(media))
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn cancel_recording_test(app: tauri::AppHandle, id: String) -> Result<(), String> {
    release_test(app, id).await
}

#[tauri::command]
pub(super) async fn discard_recording_test(
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    release_test(app, id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_limit_uses_the_screen_epoch_including_time_already_recorded() {
        let cancel = AtomicBool::new(false);
        let now = std::cell::Cell::new(Duration::from_millis(4700));
        let mut sleeps = 0;
        capture_interval(
            &cancel,
            || now.get().as_secs_f64(),
            || Ok(()),
            |delay| {
                assert!(delay <= Duration::from_millis(35));
                now.set(now.get() + delay);
                sleeps += 1;
            },
        )
        .unwrap();
        assert_eq!(now.get(), TEST_LENGTH);
        assert!((8..=10).contains(&sleeps));
    }

    #[test]
    fn cancellation_and_health_faults_do_not_publish_a_partial_recording() {
        let cancel = AtomicBool::new(false);
        let result = capture_interval(
            &cancel,
            || 1.0,
            || Ok(()),
            |_| cancel.store(true, Ordering::Release),
        );
        assert_eq!(result.unwrap_err(), CANCELLED);
        let cancel = AtomicBool::new(false);
        assert_eq!(
            capture_interval(&cancel, || 5.0, || Err("source lost".into()), |_| panic!())
                .unwrap_err(),
            "source lost"
        );
        assert!(capture_interval(&cancel, || f64::NAN, || Ok(()), |_| panic!()).is_err());
    }

    #[test]
    fn only_finalized_named_tracks_in_the_current_test_can_be_played() {
        let root = tempfile::tempdir().unwrap();
        let part = root.path().join("part-00000");
        fs::create_dir(&part).unwrap();
        let part = dunce::canonicalize(part).unwrap();
        fs::write(part.join("screen.mp4"), b"synthetic").unwrap();
        fs::write(root.path().join("screen.mp4"), b"other").unwrap();
        fs::write(part.join("camera.raw.mp4"), b"private").unwrap();
        fs::write(part.join("system.wav"), b"").unwrap();
        assert!(checked_media_path(&part, &part.join("screen.mp4"), &["screen.mp4"]).is_ok());
        assert!(
            checked_media_path(&part, &root.path().join("screen.mp4"), &["screen.mp4"]).is_err()
        );
        assert!(
            checked_media_path(&part, &part.join("camera.raw.mp4"), &["camera-source.mp4"])
                .is_err()
        );
        assert!(checked_media_path(&part, &part.join("system.wav"), &["system.wav"]).is_err());
    }

    #[test]
    fn test_result_requires_selected_tracks_cropped_dimensions_and_full_interval() {
        let cache = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let directory = TrialDirectory::create(cache.path(), &id).unwrap();
        let part = directory.path().join("part-00000");
        fs::create_dir(&part).unwrap();
        for file in [
            "screen.mp4",
            "camera-source.mp4",
            "system.wav",
            "microphone.wav",
        ] {
            fs::write(part.join(file), b"fixture").unwrap();
        }
        let options: RecordingOptions = serde_json::from_value(serde_json::json!({
            "sourceId":"app:21:32","fps":60,"systemAudio":true,"systemAudioScope":"application",
            "cameraId":"test-camera","microphoneId":"test-mic","region":{"x":10,"y":8,"width":320,"height":240}
        })).unwrap();
        let captured = || CaptureResult {
            screen_path: part.join("screen.mp4"),
            camera_path: Some(part.join("camera-source.mp4")),
            microphone_path: Some(part.join("microphone.wav")),
            system_audio_path: Some(part.join("system.wav")),
            duration: 5.012,
            width: 320,
            height: 240,
            fps: 60,
            camera_offset_seconds: 0.4,
            cursor: vec![],
        };
        let publish = |result| TrialMedia::from_capture(&id, &directory.path(), &options, result);
        let preview = publish(captured()).unwrap();
        assert_eq!(preview.camera_offset_seconds, 0.4);
        assert!(preview.system_audio_path.unwrap().ends_with("system.wav"));
        assert!(!directory.path().join("project.json").exists());
        for duration in [0.3, 7.0, f64::NAN] {
            assert!(publish(CaptureResult {
                duration,
                ..captured()
            })
            .is_err());
        }
        assert!(publish(CaptureResult {
            width: 1920,
            ..captured()
        })
        .is_err());
        assert!(publish(CaptureResult {
            microphone_path: None,
            ..captured()
        })
        .is_err());
    }

    #[test]
    fn webview_scope_exposes_only_the_four_final_test_media_names() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let allow = config
            .pointer("/app/security/assetProtocol/scope/allow")
            .unwrap()
            .as_array()
            .unwrap();
        let prefix = "$APPLOCALDATA/recording-tests/";
        let actual: std::collections::BTreeSet<_> = allow
            .iter()
            .filter_map(|path| path.as_str()?.strip_prefix(prefix))
            .collect();
        assert_eq!(
            actual,
            [
                "*/part-00000/camera-source.mp4",
                "*/part-00000/microphone.wav",
                "*/part-00000/screen.mp4",
                "*/part-00000/system.wav"
            ]
            .into_iter()
            .collect()
        );
    }
}
