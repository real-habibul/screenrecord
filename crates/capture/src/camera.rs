use crate::{media, process};
use screenrecord_core::Device;
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub fn enumerate(ffmpeg: &Path) -> Result<Vec<Device>, String> {
    let log_path = std::env::temp_dir().join(format!(
        "screenrecord-devices-{}-{}.log",
        std::process::id(),
        crate::qpc_now()
    ));
    let log = File::create(&log_path).map_err(|error| error.to_string())?;
    let mut child = process::command(ffmpeg)
        .args([
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .map_err(|error| format!("Cannot enumerate cameras: {error}"))?;
    let _job = match process::ChildJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let result = process::wait_bounded(&mut child, Duration::from_secs(10));
    let output = std::fs::read_to_string(&log_path).unwrap_or_default();
    let _ = std::fs::remove_file(&log_path);
    result?;
    Ok(parse_devices(&output))
}

fn parse_devices(output: &str) -> Vec<Device> {
    let mut devices: Vec<Device> = Vec::new();
    let mut pending = None;
    for line in output.lines() {
        if line.trim_end().ends_with("(video)") {
            if let Some(name) = quoted(line) {
                devices.push(Device {
                    id: name.to_owned(),
                    name: name.to_owned(),
                });
                pending = Some(devices.len() - 1);
            }
        } else if line.contains("Alternative name") {
            if let (Some(index), Some(id)) = (pending.take(), quoted(line)) {
                devices[index].id = id.to_owned();
            }
        } else if line.trim_end().ends_with("(audio)") || line.trim_end().ends_with("(none)") {
            pending = None;
        }
    }
    devices
}

fn quoted(text: &str) -> Option<&str> {
    let first = text.find('"')? + 1;
    let last = text.rfind('"')?;
    (last >= first).then_some(&text[first..last])
}

pub struct CameraWorker {
    child: Option<Child>,
    progress_reader: Option<JoinHandle<()>>,
    source_reader: Option<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
    pub epoch: i64,
    pub raw_path: PathBuf,
    pub log_path: PathBuf,
    exited: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    preview: Arc<Mutex<RecordingPreview>>,
    _job: Arc<process::ChildJob>,
    output: Arc<Mutex<CameraOutputState>>,
    requires_full_validation: bool,
}

#[derive(Default)]
struct CameraOutputState {
    progress: process::CameraStopProgress,
    failed: bool,
}
impl CameraOutputState {
    fn observe(&mut self, line: &str) {
        if let Some((key, value)) = line.split_once('=') {
            if let Ok(value) = value.parse::<u64>() {
                match key {
                    "frame" => self.progress.frame = self.progress.frame.max(value),
                    "out_time_us" => self.progress.output_us = self.progress.output_us.max(value),
                    "total_size" => {
                        self.progress.total_bytes = self.progress.total_bytes.max(value)
                    }
                    _ => {}
                }
            }
        }
        self.progress.complete |= line == "progress=end";
        self.failed |= ["[error]", "[fatal]", "[panic]"]
            .iter()
            .any(|level| line.contains(level));
    }
}

struct RecordingPreview {
    snapshot: crate::preview::PreviewSnapshot,
    last_frame: Instant,
}

impl CameraWorker {
    pub fn start(directory: &Path, id: &str, ffmpeg: &Path) -> Result<Self, String> {
        let mut command = process::command(ffmpeg);
        command
            .args([
                "-f",
                "dshow",
                "-use_video_device_timestamps",
                "false",
                "-rtbufsize",
                "128M",
                "-i",
            ])
            .arg(format!("video={id}"));
        Self::spawn(directory, command, None)
    }

    fn spawn(
        directory: &Path,
        mut command: std::process::Command,
        synthetic_epoch: Option<i64>,
    ) -> Result<Self, String> {
        let raw_path = directory.join("camera.raw.mp4");
        let log_path = directory.join("camera.log");
        let mut log = File::create(&log_path).map_err(|error| error.to_string())?;
        let mut child = command
            .args(["-hide_banner", "-loglevel", "level+verbose", "-nostats", "-n", "-stats_period", "0.1", "-progress", "pipe:2", "-filter_threads", "1", "-map", "0:v:0", "-an", "-vf", "scale=1280:720:force_original_aspect_ratio=decrease:force_divisible_by=2,format=nv12", "-c:v", "h264_mf", "-hw_encoding", "0", "-b:v", "6000k", "-r", "30", "-g", "30", "-movflags", "+frag_keyframe+empty_moov+default_base_moof", "-f", "mp4"])
            .arg(&raw_path)
            .args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf", "fps=4,scale=480:270:force_original_aspect_ratio=decrease:force_divisible_by=2", "-c:v", "mjpeg", "-q:v", "6", "-threads", "1", "-f", "image2pipe", "-flush_packets", "1", "pipe:1"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().map_err(|error| format!("Cannot start webcam: {error}"))?;
        let job = match process::ChildJob::attach(&child) {
            Ok(job) => Arc::new(job),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdout = child
            .stdout
            .take()
            .ok_or("Camera thumbnail stream is missing.")?;
        let exited = Arc::new(AtomicBool::new(false));
        let stderr = child
            .stderr
            .take()
            .ok_or("Camera source clock stream is missing.")?;
        let preview = Arc::new(Mutex::new(RecordingPreview {
            snapshot: crate::preview::PreviewSnapshot {
                active: true,
                ..crate::session::inactive_snapshot()
            },
            last_frame: Instant::now(),
        }));
        let stopping = Arc::new(AtomicBool::new(false));
        let output = Arc::new(Mutex::new(CameraOutputState::default()));
        let mut worker = Self {
            child: Some(child),
            progress_reader: None,
            source_reader: None,
            watchdog: None,
            epoch: 0,
            raw_path,
            log_path,
            exited: exited.clone(),
            stopping: stopping.clone(),
            preview: preview.clone(),
            _job: job.clone(),
            output: output.clone(),
            requires_full_validation: false,
        };
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let source_reader = thread::Builder::new()
            .name("recording-camera-clock".into())
            .spawn(move || {
                let mut clock = SourceClock::default();
                let mut log_bytes = 0;
                let mut announced = false;
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    output
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .observe(&line);
                    clock.observe(&line, crate::qpc_now());
                    if !announced
                        && line
                            .strip_prefix("out_time_us=")
                            .and_then(|value| value.parse::<i64>().ok())
                            .is_some_and(|us| us > 0)
                    {
                        if let Some(epoch) = synthetic_epoch.or_else(|| clock.ready_epoch()) {
                            let _ = ready_tx.send(epoch);
                            announced = true;
                        }
                    }
                    // Per-frame DirectShow diagnostics are consumed for timing, not retained on disk.
                    if !line.contains("passing through packet of type") && log_bytes < 1024 * 1024 {
                        log_bytes += line.len() + 1;
                        let _ = writeln!(log, "{line}");
                    }
                }
            })
            .map_err(|error| format!("Cannot start the camera clock reader: {error}"))?;
        worker.source_reader = Some(source_reader);
        let reader_preview = preview.clone();
        let reader_stop = stopping.clone();
        let reader_exited = exited.clone();
        let reader = thread::Builder::new().name("recording-camera-thumbnail".into()).spawn(move || {
            let result = crate::preview::read_frames(stdout, |jpeg| {
                // Drain encoder shutdown output so closing the thumbnail pipe cannot truncate MP4.
                if reader_stop.load(Ordering::Acquire) { return true; }
                let mut preview = reader_preview.lock().unwrap_or_else(|error| error.into_inner());
                preview.snapshot.sequence += 1;
                preview.snapshot.jpeg = Some(jpeg);
                preview.last_frame = Instant::now();
                true
            });
            let mut preview = reader_preview.lock().unwrap_or_else(|error| error.into_inner());
            preview.snapshot.active = false;
            if !reader_stop.load(Ordering::Acquire) && preview.snapshot.error.is_none() {
                preview.snapshot.error = Some(result.err().unwrap_or_else(|| "The recording camera stopped. Stop recording to preserve the available media.".into()));
            }
            reader_exited.store(true, Ordering::Release);
        }).map_err(|error| format!("Cannot start the camera thumbnail reader: {error}"))?;
        worker.progress_reader = Some(reader);
        worker.watchdog = Some(
            thread::Builder::new()
                .name("recording-camera-watchdog".into())
                .spawn(move || {
                    watch_thumbnail(preview, exited, stopping, job);
                })
                .map_err(|error| format!("Cannot start the camera watchdog: {error}"))?,
        );
        match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(epoch) => {
                worker.epoch = epoch;
                Ok(worker)
            }
            Err(_) => {
                let _ = worker.stop();
                let details = std::fs::read_to_string(&worker.log_path).unwrap_or_default();
                Err(format!("Webcam did not produce video with usable source timestamps. Check camera permissions and whether another app is using it. {}", process::tail(&details, 1200)))
            }
        }
    }

    pub fn is_finished(&self) -> bool {
        self.exited.load(Ordering::Acquire)
    }

    pub fn snapshot(&self) -> crate::preview::PreviewSnapshot {
        self.preview
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot
            .clone()
    }

    pub fn stop(&mut self) -> Result<(), String> {
        let started = Instant::now();
        self.stopping.store(true, Ordering::Release);
        {
            let mut preview = self
                .preview
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            preview.snapshot.active = false;
            preview.snapshot.jpeg = None;
        }
        let result = if let Some(mut child) = self.child.take() {
            let result = process::stop_camera_child_with_progress(&mut child, || {
                self.output
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .progress
            });
            let _ = self._job.terminate();
            let reaped = crate::cleanup::run(
                self.raw_path.parent().unwrap_or(&self.raw_path),
                Duration::from_secs(15).saturating_sub(started.elapsed()),
                move || child.wait().map(|_| ()).map_err(|error| error.to_string()),
            );
            result.and_then(|outcome| reaped.map(|_| Some(outcome)))
        } else {
            Ok(None)
        };
        let readers: Vec<_> = [
            self.progress_reader.take(),
            self.source_reader.take(),
            self.watchdog.take(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let readers_result = if readers.is_empty() {
            Ok(())
        } else {
            crate::cleanup::run(
                self.raw_path.parent().unwrap_or(&self.raw_path),
                Duration::from_secs(15).saturating_sub(started.elapsed()),
                move || {
                    let mut failed = false;
                    for reader in readers {
                        failed |= reader.join().is_err();
                    }
                    if failed {
                        Err("A camera reader stopped unexpectedly.".into())
                    } else {
                        Ok(())
                    }
                },
            )
        };
        let outcome = result?;
        readers_result?;
        if let Some(outcome) = outcome {
            let output = self
                .output
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !output.progress.complete || output.failed {
                return Err("The webcam did not finish writing cleanly. Available media was preserved for recovery.".into());
            }
            self.requires_full_validation |=
                outcome == process::CameraStopOutcome::DeadlineTerminated;
        }
        Ok(())
    }

    pub fn requires_full_validation(&self) -> bool {
        self.requires_full_validation
    }

    #[cfg(test)]
    pub(crate) fn fixture(directory: &Path) -> Self {
        let child = process::fixture_child(false);
        let job = process::ChildJob::attach(&child).unwrap();
        Self {
            child: Some(child),
            progress_reader: None,
            source_reader: None,
            watchdog: None,
            epoch: 1,
            raw_path: directory.join("camera.raw.mp4"),
            log_path: directory.join("camera.log"),
            exited: Arc::new(AtomicBool::new(false)),
            stopping: Arc::new(AtomicBool::new(false)),
            preview: Arc::new(Mutex::new(RecordingPreview {
                snapshot: crate::session::inactive_snapshot(),
                last_frame: Instant::now(),
            })),
            _job: Arc::new(job),
            output: Arc::new(Mutex::new(CameraOutputState {
                progress: process::CameraStopProgress {
                    complete: true,
                    ..Default::default()
                },
                failed: false,
            })),
            requires_full_validation: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn process_id(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }
}

fn watch_thumbnail(
    preview: Arc<Mutex<RecordingPreview>>,
    exited: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    job: Arc<process::ChildJob>,
) {
    while !stopping.load(Ordering::Acquire) {
        if thumbnail_failed(&preview) {
            exited.store(true, Ordering::Release);
            let _ = job.terminate();
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn thumbnail_failed(preview: &Mutex<RecordingPreview>) -> bool {
    let mut preview = preview.lock().unwrap_or_else(|error| error.into_inner());
    if !preview.snapshot.active {
        return true;
    }
    let timeout = if preview.snapshot.sequence == 0 {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(5)
    };
    if preview.last_frame.elapsed() >= timeout {
        preview.snapshot.active = false;
        preview.snapshot.jpeg = None;
        preview.snapshot.error = Some("The recording camera stopped delivering frames. Stop recording to preserve the available media, then check its connection.".into());
        return true;
    }
    false
}

#[derive(Default)]
struct SourceClock {
    first_pts: Option<i64>,
    earliest_epoch: Option<i64>,
    observations: usize,
}

impl SourceClock {
    fn ready_epoch(&self) -> Option<i64> {
        (self.observations == 8)
            .then_some(self.earliest_epoch)
            .flatten()
    }

    fn observe(&mut self, line: &str, received_qpc: i64) -> Option<i64> {
        if self.observations >= 8 || !line.contains("passing through packet of type video") {
            return None;
        }
        let value = |label: &str| {
            line.split_once(label)?
                .1
                .split_whitespace()
                .next()?
                .parse::<i64>()
                .ok()
        };
        let pts = value(" timestamp ")?;
        let graph = value(" graph timestamp ")?;
        if pts != graph || graph <= 0 {
            return None;
        }
        let first = self.first_pts.unwrap_or(pts);
        let elapsed = graph.checked_sub(first)?;
        if !(-10_000_000..=600_000_000).contains(&elapsed) {
            return None;
        }
        let estimate = received_qpc.checked_sub(elapsed)?;
        if estimate <= 0 {
            return None;
        }
        self.first_pts = Some(first);
        // The graph clock removes device delivery lag; minimum receipt delay limits pipe jitter.
        let epoch = self
            .earliest_epoch
            .map_or(estimate, |earliest| earliest.min(estimate));
        self.earliest_epoch = Some(epoch);
        self.observations += 1;
        Some(epoch)
    }
}

pub struct CameraTiming {
    pub camera_epoch: i64,
    pub screen_epoch: i64,
    pub duration: f64,
    pub existing_offset: Option<f64>,
    pub recovering: bool,
    pub require_full_validation: bool,
}

pub struct CameraTrack {
    pub path: PathBuf,
    pub offset: f64,
    pub warning: Option<String>,
}

pub fn finalize_recording(
    raw_path: &Path,
    ffmpeg: &Path,
    timing: CameraTiming,
    publish_offset: impl FnOnce(f64) -> Result<(), String>,
) -> Result<CameraTrack, media::RecoveryError> {
    let output = raw_path.with_file_name("camera-source.mp4");
    let validation_log = raw_path.with_file_name("camera-validate.log");
    if output.exists() {
        if let Some(offset) = timing.existing_offset {
            if let Some(info) =
                media::usable(media::validate_video(ffmpeg, &output, &validation_log))?
            {
                return camera_track(output, offset, &info, timing.duration)
                    .map_err(media::RecoveryError::Unusable);
            }
        }
        media::preserve_invalid(&output)?;
    }
    let legacy = raw_path.with_file_name("camera.mp4");
    if legacy.exists() {
        if let Some(info) = media::usable(media::validate_video(ffmpeg, &legacy, &validation_log))?
        {
            return camera_track(legacy, 0.0, &info, timing.duration)
                .map_err(media::RecoveryError::Unusable);
        }
        media::preserve_invalid(&legacy)?;
    }
    let info = media::probe_video(ffmpeg, raw_path)?;
    let offset =
        ((timing.screen_epoch - timing.camera_epoch) as f64 / 10_000_000.0 - info.start).max(0.0);
    let temporary = raw_path.with_file_name("camera-source-finalizing.mp4");
    if temporary.exists() {
        std::fs::remove_file(&temporary).map_err(|error| error.to_string())?;
    }
    let args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-n".into(),
        "-i".into(),
        raw_path.to_string_lossy().into_owned(),
        "-map".into(),
        "0:v:0".into(),
        "-an".into(),
        "-c:v".into(),
        "copy".into(),
        "-movflags".into(),
        "+faststart".into(),
        temporary.to_string_lossy().into_owned(),
    ];
    let remuxed = process::run_logged(ffmpeg, &args, &raw_path.with_file_name("camera-align.log"))
        .and_then(|()| {
            if timing.recovering || timing.require_full_validation {
                media::validate_video(ffmpeg, &temporary, &validation_log)
            } else {
                media::probe_video(ffmpeg, &temporary)
            }
            .map_err(String::from)
        });
    let remuxed = match remuxed {
        Ok(info) => info,
        Err(error) => {
            if timing.recovering {
                media::validate_video(ffmpeg, raw_path, &validation_log)?;
            }
            return Err(media::RecoveryError::Retry(error));
        }
    };
    let track = camera_track(output.clone(), offset, &remuxed, timing.duration)
        .map_err(media::RecoveryError::Unusable)?;
    // The offset applies only to camera-source.mp4, never to an older aligned camera.mp4.
    publish_offset(offset)?;
    std::fs::rename(temporary, &output).map_err(|error| error.to_string())?;
    if timing.require_full_validation {
        log::warn!("Webcam video was fully validated after terminating a delayed camera process.");
    }
    Ok(track)
}

#[cfg(test)]
#[path = "camera_regression_tests.rs"]
mod regression_tests;

fn camera_track(
    path: PathBuf,
    offset: f64,
    info: &media::VideoInfo,
    duration: f64,
) -> Result<CameraTrack, String> {
    if !offset.is_finite() || !(0.0..=3600.0).contains(&offset) || info.duration <= offset {
        return Err("Webcam has no usable frames on the screen recording timeline.".into());
    }
    let available = info.duration - offset;
    let warning = (available + 0.1 < duration).then(|| format!(
        "The webcam ends at {available:.3} seconds; the screen recording continues to {duration:.3} seconds. Available camera frames were preserved."
    ));
    Ok(CameraTrack {
        path,
        offset,
        warning,
    })
}

impl Drop for CameraWorker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stalled_thumbnail_terminates_and_reaps_only_the_owned_camera_process() {
        let directory = tempfile::tempdir().unwrap();
        let mut camera = CameraWorker::fixture(directory.path());
        {
            let mut preview = camera.preview.lock().unwrap();
            preview.snapshot.active = true;
            preview.snapshot.sequence = 1;
            preview.last_frame = Instant::now() - Duration::from_secs(6);
        }
        watch_thumbnail(
            camera.preview.clone(),
            camera.exited.clone(),
            camera.stopping.clone(),
            camera._job.clone(),
        );
        assert!(camera.is_finished());
        assert!(camera.snapshot().error.is_some());
        assert!(camera.stop().is_err());
        assert!(camera.child.is_none());
    }

    #[test]
    fn thumbnail_stall_clears_frame_and_marks_recording_unhealthy() {
        let state = Mutex::new(RecordingPreview {
            snapshot: crate::preview::PreviewSnapshot {
                sequence: 1,
                jpeg: Some(vec![1, 2, 3]),
                active: true,
                error: None,
            },
            last_frame: Instant::now() - Duration::from_secs(6),
        });
        assert!(thumbnail_failed(&state));
        let state = state.lock().unwrap();
        assert!(state.snapshot.jpeg.is_none());
        assert!(!state.snapshot.active);
        assert!(state
            .snapshot
            .error
            .as_ref()
            .unwrap()
            .contains("stopped delivering"));
    }

    #[test]
    #[ignore = "Requires the bundled FFmpeg; uses synthetic video only."]
    fn media_contract_recording_camera_encodes_and_streams_bounded_latest_thumbnail() {
        let ffmpeg = PathBuf::from(
            std::env::var_os("SCREENRECORD_FFMPEG").expect("Set SCREENRECORD_FFMPEG."),
        );
        let directory = tempfile::tempdir().unwrap();
        let mut command = process::command(&ffmpeg);
        command.args(["-re", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30"]);
        let mut camera =
            CameraWorker::spawn(directory.path(), command, Some(crate::qpc_now())).unwrap();
        let first = camera.snapshot().sequence;
        thread::sleep(Duration::from_millis(1300));
        let snapshot = camera.snapshot();
        assert!(snapshot.active);
        assert!(snapshot.sequence >= first + 4);
        let jpeg = snapshot.jpeg.unwrap();
        assert!(jpeg.len() < 512 * 1024);
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xff, 0xd9]);
        let stopped = Instant::now();
        let stop = camera.stop();
        assert!(
            stop.is_ok(),
            "Synthetic camera stop failed: {stop:?}. FFmpeg stderr: {}",
            process::tail(
                &std::fs::read_to_string(&camera.log_path).unwrap_or_default(),
                12000
            )
        );
        assert!(stopped.elapsed() < Duration::from_secs(4));
        assert!(camera.child.is_none());
        assert!(
            camera.progress_reader.is_none()
                && camera.source_reader.is_none()
                && camera.watchdog.is_none()
        );
        assert!(!camera.snapshot().active);
        assert!(camera.snapshot().jpeg.is_none());
        let info = media::validate_video(
            &ffmpeg,
            &camera.raw_path,
            &directory.path().join("validate.log"),
        )
        .unwrap();
        assert!(info.duration >= 1.0);
        assert_eq!((info.width, info.height), (1280, 720));
    }

    #[test]
    fn source_clock_waits_for_eight_observations_then_freezes() {
        let mut clock = SourceClock::default();
        for index in 0..8 {
            let graph = 100_000_000 + index * 333_333;
            let device = graph - 400_000;
            let line = format!("[dshow] passing through packet of type video size 100 timestamp {graph} orig timestamp {device} graph timestamp {graph} diff 400000 Camera");
            let pipe_delay = (7 - index) * 10_000;
            clock.observe(&line, 10_000_000 + graph - 100_000_000 + pipe_delay);
            assert_eq!(clock.ready_epoch(), (index == 7).then_some(10_000_000));
        }
        assert!(clock.observe("[dshow] passing through packet of type video size 100 timestamp 103000000 orig timestamp 103000000 graph timestamp 103400000 diff 400000 Camera", 10_000_000).is_none());
        assert_eq!(clock.ready_epoch(), Some(10_000_000));
    }
    #[test]
    fn source_clock_anchors_before_encoding_and_rejects_unrelated_logs() {
        let mut clock = SourceClock::default();
        assert!(clock.observe("out_time_us=800000", 20_000_000).is_none());
        let line = "[dshow] passing through packet of type video size 100 timestamp 100000000 orig timestamp 99600000 graph timestamp 100000000 diff 400000 Camera";
        assert_eq!(clock.observe(line, 10_100_000), Some(10_100_000));
        let next = "[dshow] passing through packet of type video size 100 timestamp 100333333 orig timestamp 99933333 graph timestamp 100333333 diff 400000 Camera";
        assert_eq!(clock.observe(next, 10_383_333), Some(10_050_000));
        assert_eq!(clock.observe(next, 12_783_333), Some(10_050_000));
        assert!(clock
            .observe("[dshow] video timestamp broken", 10_000_000)
            .is_none());
    }
    #[test]
    fn device_names_preserve_unicode_and_alternative_identity() {
        let devices = parse_devices("[dshow] \"Camera café\" (video)\n[dshow]   Alternative name \"@device_unique\"\n[dshow] \"Microphone\" (audio)\n[dshow]   Alternative name \"mic\"\n");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "@device_unique");
        assert_eq!(devices[0].name, "Camera café");
    }
}
