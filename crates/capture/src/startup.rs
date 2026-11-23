use crate::Journal;
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::windows::fs::MetadataExt,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

#[cfg(test)]
pub(crate) fn launch<T: Send + 'static>(
    directory: &Path,
    stop: Arc<AtomicBool>,
    deadline: Duration,
    start: impl FnOnce() -> Result<T, String> + Send + 'static,
    shutdown: impl FnOnce(T) -> Result<(), String> + Send + 'static,
) -> Result<T, String> {
    launch_cancellable(directory, stop, deadline, None, start, shutdown)
}

pub(crate) const CANCELLED: &str = "Recording test cancelled.";

pub(crate) fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

pub(crate) struct CancelRelay {
    finished: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl CancelRelay {
    pub(crate) fn start(
        cancel: Option<Arc<AtomicBool>>,
        stop: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let finished = Arc::new(AtomicBool::new(false));
        let done = finished.clone();
        let worker = cancel
            .map(|cancel| {
                std::thread::Builder::new()
                    .name("recording-test-startup-cancel".into())
                    .spawn(move || {
                        while !done.load(Ordering::Acquire) {
                            if cancel.load(Ordering::Acquire) {
                                stop.store(true, Ordering::Release);
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    })
                    .map_err(|error| error.to_string())
            })
            .transpose()?;
        Ok(Self { finished, worker })
    }
}

impl Drop for CancelRelay {
    fn drop(&mut self) {
        self.finished.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(crate) enum WaitError {
    Cancelled,
    Timeout,
    Disconnected,
}

pub(crate) fn wait_cancellable<T>(
    receiver: &mpsc::Receiver<T>,
    deadline: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<T, WaitError> {
    let began = Instant::now();
    loop {
        if check_cancel(cancel).is_err() {
            return Err(WaitError::Cancelled);
        }
        let result = receiver
            .recv_timeout(Duration::from_millis(25).min(deadline.saturating_sub(began.elapsed())));
        if check_cancel(cancel).is_err() {
            return Err(WaitError::Cancelled);
        }
        match result {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(WaitError::Disconnected),
            Err(mpsc::RecvTimeoutError::Timeout) if began.elapsed() >= deadline => {
                return Err(WaitError::Timeout)
            }
            _ => (),
        }
    }
}

pub(crate) fn launch_cancellable<T: Send + 'static>(
    directory: &Path,
    stop: Arc<AtomicBool>,
    deadline: Duration,
    cancel: Option<Arc<AtomicBool>>,
    start: impl FnOnce() -> Result<T, String> + Send + 'static,
    shutdown: impl FnOnce(T) -> Result<(), String> + Send + 'static,
) -> Result<T, String> {
    check_cancel(cancel.as_deref())?;
    let (done, complete) = mpsc::sync_channel(1);
    let worker_cancel = cancel.clone();
    let worker = std::thread::Builder::new()
        .name("screen-capture-startup".into())
        .spawn(move || {
            let result = check_cancel(worker_cancel.as_deref()).and_then(|_| start());
            let _ = done.send(());
            result
        })
        .map_err(|_| "Cannot create the screen capture startup worker.".to_owned())?;
    let waited = wait_cancellable(&complete, deadline, cancel.as_deref());
    if waited.is_ok() {
        return worker
            .join()
            .map_err(|_| "The screen capture startup worker failed.".to_owned())?;
    }
    stop.store(true, Ordering::Release);
    let cleanup = crate::cleanup::run_native(directory, Duration::from_secs(2), move || {
        let result = worker
            .join()
            .unwrap_or_else(|_| panic!("Native startup ownership is uncertain after a panic."));
        if let Ok(capture) = result {
            shutdown(capture)?;
        }
        Ok(())
    });
    if matches!(waited, Err(WaitError::Cancelled)) {
        if let Err(error) = cleanup {
            log::warn!("Cancelled native startup cleanup remains pending: {error}");
        }
        return Err(CANCELLED.into());
    }
    Err(match cleanup {
        Ok(()) => "Windows capture initialization did not finish before its deadline. No screen recording was started.".into(),
        Err(error) => format!("Windows capture initialization did not finish. {error}"),
    })
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Stage {
    Waiting,
    FrameReceived,
    EncoderStarting,
    EncoderSubmission,
    Snapshot,
    Ready,
}

pub(crate) struct State(AtomicU8);
impl Default for State {
    fn default() -> Self {
        Self(AtomicU8::new(Stage::Waiting as u8))
    }
}
impl State {
    pub(crate) fn set(&self, stage: Stage) {
        self.0.store(stage as u8, Ordering::Release);
    }
    fn timeout(&self, monitor: bool) -> String {
        let stage = self.0.load(Ordering::Acquire);
        match stage {
            0 if monitor => "Screen capture did not reach its first video frame for the selected display before the startup deadline.".into(),
            0 => "Screen capture did not reach its first video frame for the selected window before the startup deadline.".into(),
            1 => "Windows delivered a frame, but its timestamp or recording journal could not finish initializing.".into(),
            2 => "Windows delivered a frame, but the video encoder did not finish initializing.".into(),
            3 => "Windows delivered a frame, but the video encoder stopped accepting it.".into(),
            4 => "Windows delivered a frame, but the GPU snapshot did not finish initializing.".into(),
            _ => "Screen capture did not complete its startup notification.".into(),
        }
    }
}

pub(crate) fn persist_diagnostic(
    directory: &Path,
    journal: &Journal,
    state: &State,
    native: windows_capture::diagnostics::CaptureDiagnosticsSnapshot,
    error: &str,
    worker_finished: Option<bool>,
) {
    let result = (|| -> Result<(), String> {
        let mut file = File::create_new(directory.join("capture-startup-diagnostic.json"))
            .map_err(|error| error.to_string())?;
        let value = serde_json::json!({
            "schemaVersion": 1, "appVersion": env!("CARGO_PKG_VERSION"),
            "timestampUnixMs": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),
            "sourceId": journal.source_id, "width": journal.width, "height": journal.height,
            "appStage": state.0.load(Ordering::Acquire), "nativeStartupStage": native.startup_stage,
            "nativeFrameStage": native.frame_stage, "frameEvents": native.frame_events,
            "deliveredFrames": native.delivered_frames, "lastHresult": native.last_hresult,
            "workerFinished": worker_finished, "error": error.chars().take(2048).collect::<String>(),
        });
        serde_json::to_writer_pretty(&mut file, &value).map_err(|error| error.to_string())?;
        file.flush().map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())
    })();
    if let Err(error) = result {
        log::warn!("Cannot persist native capture startup diagnostics: {error}");
    }
}

#[cfg(test)]
pub(crate) fn wait_for_first_frame(
    ready: &mpsc::Receiver<Result<(u32, u32), String>>,
    state: &State,
    monitor: bool,
    deadline: Duration,
    finished: impl FnMut() -> Option<Result<(), String>>,
) -> Result<(u32, u32), String> {
    wait_for_first_frame_cancellable(ready, state, monitor, deadline, None, finished)
}

pub(crate) fn wait_for_first_frame_cancellable(
    ready: &mpsc::Receiver<Result<(u32, u32), String>>,
    state: &State,
    monitor: bool,
    deadline: Duration,
    cancel: Option<&AtomicBool>,
    mut finished: impl FnMut() -> Option<Result<(), String>>,
) -> Result<(u32, u32), String> {
    let began = Instant::now();
    loop {
        check_cancel(cancel)?;
        let message = ready
            .recv_timeout(Duration::from_millis(25).min(deadline.saturating_sub(began.elapsed())));
        check_cancel(cancel)?;
        match message {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return match finished() {
                    Some(Err(error)) => {
                        Err(format!("Screen capture stopped during startup: {error}"))
                    }
                    _ => Err("Screen capture closed before publishing its first frame.".into()),
                };
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(result) = finished() {
            return Err(match result {
                Err(error) => format!("Screen capture stopped during startup: {error}"),
                Ok(()) => "The capture source closed before its first frame was ready.".into(),
            });
        }
        if began.elapsed() >= deadline {
            return Err(state.timeout(monitor));
        }
    }
}

fn plain(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "The failed-start recording metadata is unavailable.".to_owned())?;
    if metadata.file_attributes() & 0x400 != 0 || metadata.file_type().is_symlink() {
        return Err("Failed-start classification does not follow links or junctions.".into());
    }
    Ok(metadata)
}

fn missing(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            plain(path)?;
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err("Cannot inspect failed-start recording files.".into()),
    }
}

/// True means the journals and empty screen directory prove capture never started; no files are changed.
pub fn never_started_screen(directory: &Path) -> Result<bool, String> {
    crate::cleanup::ensure_idle()?;
    classify(directory)
}

fn classify(directory: &Path) -> Result<bool, String> {
    if !plain(directory)?.is_dir() {
        return Err("The recording directory is invalid.".into());
    }
    let directory = crate::canonical_media_directory(directory)?;
    let part = if missing(&directory.join("session-journal.json"))? {
        directory.clone()
    } else {
        crate::session::initial_failed_part(&directory)?
    };
    if !plain(&part)?.is_dir() {
        return Err("The recording part directory is invalid.".into());
    }
    for base in [&directory, &part] {
        for entry in fs::read_dir(base).map_err(|_| "Cannot inspect recording media.")? {
            let entry = entry.map_err(|_| "Cannot inspect a recording file.")?;
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with("screen") && !name.ends_with(".log") {
                return Ok(false);
            }
            if base == &directory && name.starts_with("part-") && name != "part-00000" {
                return Ok(false);
            }
        }
    }
    let journal_path = part.join("capture-journal.json");
    if !plain(&journal_path)?.is_file() {
        return Err("The capture journal is invalid.".into());
    }
    let mut bytes = Vec::new();
    File::open(&journal_path)
        .map_err(|_| "Cannot open the capture journal.")?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read the capture journal.")?;
    if bytes.len() > 65_536 {
        return Err("The capture journal is too large.".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "The capture journal is malformed.")?;
    let fields = [
        "version",
        "sourceId",
        "width",
        "height",
        "fps",
        "epochQpc",
        "cameraEpochQpc",
        "cameraOffsetSeconds",
        "duration",
        "microphone",
        "systemAudio",
        "segments",
        "segmentDurations",
    ];
    if value
        .as_object()
        .is_none_or(|object| object.keys().any(|key| !fields.contains(&key.as_str())))
    {
        return Err("The capture journal contains unknown fields.".into());
    }
    let journal: Journal =
        serde_json::from_value(value).map_err(|_| "The capture journal is incomplete.")?;
    if journal.version != 1 {
        return Err("The capture journal version is unsupported.".into());
    }
    if !(2..=16384).contains(&journal.width)
        || !(2..=16384).contains(&journal.height)
        || ![30, 60].contains(&journal.fps)
        || !(crate::apps::AppWindow::parse(&journal.source_id).is_ok()
            || journal.source_id.split_once(':').is_some_and(|(kind, id)| {
                ["monitor", "window"].contains(&kind) && id.parse::<usize>().is_ok_and(|id| id > 0)
            }))
    {
        return Err("The capture journal contains invalid source metadata.".into());
    }
    if journal.epoch_qpc != 0
        || journal.duration != 0.0
        || !journal.segments.is_empty()
        || !journal.segment_durations.is_empty()
    {
        return Ok(false);
    }
    let segments = part.join("segments");
    if !plain(&segments)?.is_dir() {
        return Err("The screen segment directory is unavailable.".into());
    }
    if fs::read_dir(&segments)
        .map_err(|_| "Cannot inspect screen segments.")?
        .next()
        .transpose()
        .map_err(|_| "Cannot inspect a screen segment.")?
        .is_some()
    {
        return Ok(false);
    }
    if part != directory && !missing(&directory.join("segments"))? {
        return Ok(false);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_cancellation_interrupts_first_frame_wait_and_queued_success() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            signal.store(true, Ordering::Release);
        });
        let began = Instant::now();
        assert_eq!(
            wait_for_first_frame_cancellable(
                &receiver,
                &State::default(),
                true,
                Duration::from_secs(15),
                Some(&cancel),
                || None
            )
            .unwrap_err(),
            CANCELLED
        );
        worker.join().unwrap();
        assert!(began.elapsed() < Duration::from_secs(1));
        sender.send(Ok((1920, 1080))).unwrap();
        assert_eq!(
            wait_for_first_frame_cancellable(
                &receiver,
                &State::default(),
                true,
                Duration::from_secs(15),
                Some(&cancel),
                || panic!("cancelled before dispatch")
            )
            .unwrap_err(),
            CANCELLED
        );
    }

    #[test]
    fn cancelled_native_start_reaps_late_control_without_waiting_for_frame_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let (entered, entering) = mpsc::sync_channel(1);
        let (release, blocked) = mpsc::sync_channel(1);
        let signal = cancel.clone();
        let stopped = stop.clone();
        let controller = std::thread::spawn(move || {
            entering.recv_timeout(Duration::from_secs(1)).unwrap();
            signal.store(true, Ordering::Release);
            let began = Instant::now();
            while !stopped.load(Ordering::Acquire) && began.elapsed() < Duration::from_secs(1) {
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(stopped.load(Ordering::Acquire));
            release.send(()).unwrap();
        });
        let closed = Arc::new(AtomicBool::new(false));
        let closed_by_cleanup = closed.clone();
        let began = Instant::now();
        let result = launch_cancellable(
            directory.path(),
            stop.clone(),
            Duration::from_secs(15),
            Some(cancel.clone()),
            move || {
                entered.send(()).unwrap();
                blocked.recv().unwrap();
                Ok(42)
            },
            move |control| {
                assert_eq!(control, 42);
                closed_by_cleanup.store(true, Ordering::Release);
                Ok(())
            },
        );
        controller.join().unwrap();
        assert_eq!(result.unwrap_err(), CANCELLED);
        assert!(began.elapsed() < Duration::from_secs(1));
        assert!(closed.load(Ordering::Acquire));
        assert!(crate::cleanup::ensure_recovery_ready(directory.path()).is_ok());
        assert_eq!(
            launch_cancellable(
                directory.path(),
                stop,
                Duration::from_secs(15),
                Some(cancel),
                || -> Result<(), String> { panic!("pre-cancelled launch must not open a source") },
                |_| Ok(())
            )
            .unwrap_err(),
            CANCELLED
        );
    }

    #[test]
    fn cancellation_relay_is_one_way_and_stops_when_startup_finishes() {
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(true));
        let relay = CancelRelay::start(Some(cancel.clone()), stop.clone()).unwrap();
        drop(relay);
        assert!(!cancel.load(Ordering::Acquire));
        stop.store(false, Ordering::Release);
        let relay = CancelRelay::start(Some(cancel.clone()), stop.clone()).unwrap();
        cancel.store(true, Ordering::Release);
        let began = Instant::now();
        while !stop.load(Ordering::Acquire) && began.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(stop.load(Ordering::Acquire));
        drop(relay);
        let normal = CancelRelay::start(None, stop).unwrap();
        assert!(normal.worker.is_none());
    }
    #[test]
    fn worker_error_and_encoder_stall_are_not_reported_as_missing_frames() {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let state = State::default();
        let error = wait_for_first_frame(&receiver, &state, true, Duration::from_secs(1), || {
            Some(Err("injected frame-pool HRESULT".into()))
        })
        .unwrap_err();
        assert!(error.contains("injected frame-pool HRESULT"));
        state.set(Stage::EncoderStarting);
        let error =
            wait_for_first_frame(&receiver, &state, true, Duration::from_millis(1), || None)
                .unwrap_err();
        assert!(error.contains("encoder") && !error.contains("did not deliver"));
        state.set(Stage::Waiting);
        assert!(!state.timeout(true).contains("Restore the window"));
    }
    #[test]
    fn queued_ready_and_handler_errors_are_preserved() {
        for result in [Ok((1920, 1080)), Err("GPU device removed".into())] {
            let (send, receive) = mpsc::sync_channel(1);
            send.send(result.clone()).unwrap();
            assert_eq!(
                wait_for_first_frame(
                    &receive,
                    &State::default(),
                    true,
                    Duration::from_secs(1),
                    || panic!("queued result wins")
                ),
                result
            );
        }
    }

    fn fixture(directory: &Path, session: bool) -> std::path::PathBuf {
        let part = if session {
            fs::write(directory.join("session-journal.json"), br#"{"version":1,"dimensions":null,"parts":[{"directory":"part-00000","duration":null}]}"#).unwrap();
            let path = directory.join("part-00000");
            fs::create_dir(&path).unwrap();
            path
        } else {
            directory.to_owned()
        };
        fs::create_dir(part.join("segments")).unwrap();
        fs::write(part.join("capture-journal.json"), br#"{"version":1,"sourceId":"monitor:42","width":1920,"height":1080,"fps":30,"epochQpc":0,"cameraEpochQpc":null,"duration":0,"microphone":false,"systemAudio":true,"segments":[],"segmentDurations":[]}"#).unwrap();
        fs::write(part.join("camera.raw.mp4"), b"camera media preserved").unwrap();
        fs::write(part.join("system.wav"), b"audio header preserved").unwrap();
        part
    }

    #[test]
    fn only_proven_empty_screen_attempt_is_classified_and_auxiliary_media_is_untouched() {
        for session in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let part = fixture(directory.path(), session);
            assert!(classify(directory.path()).unwrap());
            assert_eq!(
                fs::read(part.join("camera.raw.mp4")).unwrap(),
                b"camera media preserved"
            );
            assert_eq!(
                fs::read(part.join("system.wav")).unwrap(),
                b"audio header preserved"
            );
            fs::write(part.join("segments/unfinalized.mp4"), []).unwrap();
            assert!(!classify(directory.path()).unwrap());
            fs::remove_file(part.join("segments/unfinalized.mp4")).unwrap();
            fs::write(part.join("screen-finalizing.mp4"), []).unwrap();
            assert!(!classify(directory.path()).unwrap());
        }
    }

    #[test]
    fn metadata_diagnostics_preserve_failed_start_classification_and_do_not_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let part = fixture(directory.path(), true);
        let journal: Journal =
            serde_json::from_slice(&fs::read(part.join("capture-journal.json")).unwrap()).unwrap();
        let native = windows_capture::diagnostics::CaptureDiagnostics::default();
        persist_diagnostic(
            &part,
            &journal,
            &State::default(),
            native.snapshot(),
            &"failure".repeat(1000),
            Some(false),
        );
        let diagnostic = part.join("capture-startup-diagnostic.json");
        let bytes = fs::read(&diagnostic).unwrap();
        assert!(bytes.len() < 8192);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["frameEvents"], 0);
        assert_eq!(value["appVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["nativeFrameStage"], "no_event");
        assert!(classify(directory.path()).unwrap());
        persist_diagnostic(
            &part,
            &journal,
            &State::default(),
            native.snapshot(),
            "replacement",
            None,
        );
        assert_eq!(fs::read(diagnostic).unwrap(), bytes);
        assert_eq!(
            fs::read(part.join("camera.raw.mp4")).unwrap(),
            b"camera media preserved"
        );
    }

    #[test]
    fn apps_failed_start_classification_uses_stored_syntax_without_live_identity() {
        for session in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let part = fixture(directory.path(), session);
            let path = part.join("capture-journal.json");
            let mut journal: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            journal["sourceId"] = serde_json::json!("app:4294967295:18446744073709551615");
            fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
            assert!(classify(directory.path()).unwrap());
            for invalid in ["app:0:1", "app:1:0", "app:1:2:3", "app:01:2"] {
                journal["sourceId"] = serde_json::json!(invalid);
                fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
                assert!(classify(directory.path()).is_err(), "{invalid}");
            }
            assert_eq!(
                fs::read(part.join("camera.raw.mp4")).unwrap(),
                b"camera media preserved"
            );
            assert_eq!(
                fs::read(part.join("system.wav")).unwrap(),
                b"audio header preserved"
            );
        }
    }

    #[test]
    fn classification_rejects_missing_unknown_or_noninitial_journals() {
        let directory = tempfile::tempdir().unwrap();
        let part = fixture(directory.path(), true);
        let capture = part.join("capture-journal.json");
        let original = fs::read(&capture).unwrap();
        for (field, value) in [
            ("version", serde_json::json!(2)),
            ("unexpected", serde_json::json!(true)),
            ("width", serde_json::json!(0)),
        ] {
            let mut data: serde_json::Value = serde_json::from_slice(&original).unwrap();
            data[field] = value;
            fs::write(&capture, serde_json::to_vec(&data).unwrap()).unwrap();
            assert!(classify(directory.path()).is_err());
        }
        let mut data: serde_json::Value = serde_json::from_slice(&original).unwrap();
        data["epochQpc"] = serde_json::json!(1);
        fs::write(&capture, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(!classify(directory.path()).unwrap());
        fs::write(&capture, &original).unwrap();
        fs::write(directory.path().join("session-journal.json"), br#"{"version":1,"dimensions":[1920,1080,30],"parts":[{"directory":"part-00000","duration":null}]}"#).unwrap();
        assert!(classify(directory.path()).is_err());
        fs::remove_file(&capture).unwrap();
        assert!(classify(&part).is_err());
    }

    #[test]
    fn incomplete_session_and_unjournaled_parts_cannot_retire_recovery() {
        for text in [
            br#"{"version":1,"parts":[{"directory":"part-00000","duration":null}]}"#.as_slice(),
            br#"{"version":1,"dimensions":null,"parts":[{"directory":"part-00000"}]}"#,
        ] {
            let directory = tempfile::tempdir().unwrap();
            fixture(directory.path(), true);
            fs::write(directory.path().join("session-journal.json"), text).unwrap();
            assert!(classify(directory.path()).is_err());
        }
        let directory = tempfile::tempdir().unwrap();
        fixture(directory.path(), true);
        fs::create_dir(directory.path().join("part-00001")).unwrap();
        fs::write(
            directory.path().join("part-00001/screen.mp4"),
            b"preserve resume media",
        )
        .unwrap();
        assert!(!classify(directory.path()).unwrap());
    }

    #[test]
    fn late_native_start_is_stopped_and_its_ownership_is_retained() {
        let directory = tempfile::tempdir().unwrap();
        let (release, blocked) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let observer = closed.clone();
        let released = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            release.send(()).unwrap();
        });
        let error = launch(
            directory.path(),
            stop.clone(),
            Duration::from_millis(5),
            move || {
                blocked.recv().unwrap();
                Ok(())
            },
            move |_| {
                observer.store(true, Ordering::Release);
                Ok(())
            },
        )
        .unwrap_err();
        released.join().unwrap();
        assert!(error.contains("initialization"));
        assert!(stop.load(Ordering::Acquire));
        assert!(closed.load(Ordering::Acquire));
    }

    #[test]
    fn stuck_native_start_blocks_recovery_until_owned_cleanup_finishes() {
        let directory = tempfile::tempdir().unwrap();
        let (release, blocked) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let observer = stopped.clone();
        let begin = Instant::now();
        assert!(launch(
            directory.path(),
            Arc::new(AtomicBool::new(false)),
            Duration::from_millis(5),
            move || {
                blocked.recv().unwrap();
                Ok(())
            },
            move |_| {
                observer.store(true, Ordering::Release);
                Ok(())
            }
        )
        .is_err());
        assert!(begin.elapsed() < Duration::from_secs(3));
        assert!(crate::cleanup::ensure_recovery_ready(directory.path()).is_err());
        release.send(()).unwrap();
        for _ in 0..100 {
            if crate::cleanup::ensure_recovery_ready(directory.path()).is_ok() {
                assert!(stopped.load(Ordering::Acquire));
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("Owned startup cleanup did not finish");
    }
}
