use crate::{cleanup, process};
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_FRAME_BYTES: usize = 512 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct PreviewSnapshot {
    pub sequence: u64,
    pub jpeg: Option<Vec<u8>>,
    pub active: bool,
    pub error: Option<String>,
}

struct State {
    snapshot: PreviewSnapshot,
    last_frame: Instant,
}

struct Resources {
    child: Child,
    job: Arc<process::ChildJob>,
    reader: Option<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
}

pub struct CameraPreview {
    state: Arc<Mutex<State>>,
    stopping: Arc<AtomicBool>,
    resources: Option<Resources>,
}

impl CameraPreview {
    /// The caller validates the ID against native camera enumeration and stops this before recording.
    pub fn start(camera_id: &str, ffmpeg: &Path) -> Result<Self, String> {
        cleanup::ensure_idle()?;
        if camera_id.is_empty() || camera_id.len() > 4096 || camera_id.chars().any(char::is_control)
        {
            return Err("Select a valid camera before enabling preview.".into());
        }
        let mut command = process::command(ffmpeg);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "dshow",
                "-rtbufsize",
                "16M",
                "-threads",
                "1",
                "-i",
            ])
            .arg(format!("video={camera_id}"));
        Self::spawn(command, STARTUP_TIMEOUT, FRAME_TIMEOUT)
    }

    fn spawn(mut command: Command, startup: Duration, stall: Duration) -> Result<Self, String> {
        command
            .args([
                "-an",
                "-sn",
                "-dn",
                "-filter_threads",
                "1",
                "-vf",
                "fps=4,scale=480:270:force_original_aspect_ratio=decrease:force_divisible_by=2",
                "-c:v",
                "mjpeg",
                "-q:v",
                "6",
                "-threads",
                "1",
                "-f",
                "image2pipe",
                "-flush_packets",
                "1",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|_| {
            "Cannot start camera preview. Check the bundled media runtime.".to_owned()
        })?;
        let job = match process::ChildJob::attach(&child) {
            Ok(job) => Arc::new(job),
            Err(error) => {
                let _ = child.kill();
                let _ = cleanup::run(
                    Path::new("camera-preview"),
                    Duration::from_secs(2),
                    move || child.wait().map(|_| ()).map_err(|error| error.to_string()),
                );
                return Err(error);
            }
        };
        let stdout = child
            .stdout
            .take()
            .ok_or("Camera preview pipe is unavailable.")?;
        let state = Arc::new(Mutex::new(State {
            snapshot: PreviewSnapshot {
                sequence: 0,
                jpeg: None,
                active: true,
                error: None,
            },
            last_frame: Instant::now(),
        }));
        let stopping = Arc::new(AtomicBool::new(false));
        let mut preview = Self {
            state: state.clone(),
            stopping: stopping.clone(),
            resources: Some(Resources {
                child,
                job: job.clone(),
                reader: None,
                watchdog: None,
            }),
        };
        let reader_state = state.clone();
        let reader_stop = stopping.clone();
        let reader = thread::Builder::new().name("camera-preview-frames".into()).spawn(move || {
            let result = read_frames(stdout, |jpeg| {
                if reader_stop.load(Ordering::Acquire) { return false; }
                let mut state = reader_state.lock().unwrap_or_else(|error| error.into_inner());
                state.snapshot.sequence += 1;
                state.snapshot.jpeg = Some(jpeg);
                state.last_frame = Instant::now();
                true
            });
            let mut state = reader_state.lock().unwrap_or_else(|error| error.into_inner());
            state.snapshot.active = false;
            if !reader_stop.load(Ordering::Acquire) && state.snapshot.error.is_none() {
                state.snapshot.error = Some(result.err().unwrap_or_else(|| "Camera preview ended. Check camera access in Windows privacy settings, close other camera apps, and try again.".into()));
            }
        }).map_err(|_| "Cannot start the camera preview reader.".to_owned())?;
        preview.resources.as_mut().unwrap().reader = Some(reader);
        let watchdog = thread::Builder::new().name("camera-preview-watchdog".into()).spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                {
                    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                    if !state.snapshot.active { let _ = job.terminate(); break; }
                    let timeout = if state.snapshot.sequence == 0 { startup } else { stall };
                    if state.last_frame.elapsed() >= timeout {
                        state.snapshot.active = false;
                        state.snapshot.jpeg = None;
                        state.snapshot.error = Some("Camera preview timed out. Check the camera connection and try again.".into());
                        let _ = job.terminate();
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(50));
            }
        }).map_err(|_| "Cannot start the camera preview watchdog.".to_owned())?;
        preview.resources.as_mut().unwrap().watchdog = Some(watchdog);
        Ok(preview)
    }

    pub fn snapshot(&self) -> PreviewSnapshot {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot
            .clone()
    }

    /// A deadline error retains child ownership in capture cleanup and blocks the next recording.
    pub fn stop(&mut self) -> Result<(), String> {
        self.stopping.store(true, Ordering::Release);
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.snapshot.active = false;
            state.snapshot.jpeg = None;
        }
        let Some(mut resources) = self.resources.take() else {
            return Ok(());
        };
        let _ = resources.job.terminate();
        let _ = resources.child.kill();
        cleanup::run(
            Path::new("camera-preview"),
            Duration::from_secs(2),
            move || {
                resources
                    .child
                    .wait()
                    .map_err(|_| "Camera preview process could not be released.".to_owned())?;
                if let Some(reader) = resources.reader {
                    reader
                        .join()
                        .map_err(|_| "Camera preview reader stopped unexpectedly.".to_owned())?;
                }
                if let Some(watchdog) = resources.watchdog {
                    watchdog
                        .join()
                        .map_err(|_| "Camera preview watchdog stopped unexpectedly.".to_owned())?;
                }
                Ok(())
            },
        )
    }
}

impl Drop for CameraPreview {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub(crate) fn read_frames(
    mut input: impl Read,
    mut publish: impl FnMut(Vec<u8>) -> bool,
) -> Result<(), String> {
    let mut pending = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let count = input
            .read(&mut chunk)
            .map_err(|_| "Camera preview stream failed.".to_owned())?;
        if count == 0 {
            return if pending.is_empty() {
                Ok(())
            } else {
                Err("Camera returned an incomplete preview frame.".into())
            };
        }
        pending.extend_from_slice(&chunk[..count]);
        while let Some(end) = jpeg_end(&pending)? {
            let trailing = pending.split_off(end);
            if !publish(std::mem::replace(&mut pending, trailing)) {
                return Ok(());
            }
        }
        if pending.len() > MAX_FRAME_BYTES {
            return Err("Camera preview exceeded its frame size limit.".into());
        }
    }
}

fn jpeg_end(bytes: &[u8]) -> Result<Option<usize>, String> {
    if bytes.len() < 2 {
        return Ok(None);
    }
    if bytes[..2] != [0xff, 0xd8] {
        return Err("Camera returned an invalid JPEG preview.".into());
    }
    let mut position = 2;
    let mut entropy = false;
    while position < bytes.len() {
        if position >= MAX_FRAME_BYTES {
            return Err("Camera preview exceeded its frame size limit.".into());
        }
        if entropy && bytes[position] != 0xff {
            position += 1;
            continue;
        }
        if bytes[position] != 0xff {
            return Err("Camera returned an invalid JPEG marker.".into());
        }
        let marker_start = position;
        while position < bytes.len() && bytes[position] == 0xff {
            position += 1;
        }
        let Some(&marker) = bytes.get(position) else {
            return Ok(None);
        };
        position += 1;
        if entropy && (marker == 0 || (0xd0..=0xd7).contains(&marker)) {
            continue;
        }
        if marker == 0xd9 {
            if position > MAX_FRAME_BYTES {
                return Err("Camera preview exceeded its frame size limit.".into());
            }
            return Ok(Some(position));
        }
        if marker == 0xd8 || marker == 0 || marker == 1 || (0xd0..=0xd7).contains(&marker) {
            return Err("Camera returned an invalid JPEG segment.".into());
        }
        if position + 2 > bytes.len() {
            return Ok(None);
        }
        let length = u16::from_be_bytes([bytes[position], bytes[position + 1]]) as usize;
        if length < 2 || position + length > MAX_FRAME_BYTES || position <= marker_start {
            return Err("Camera preview segment exceeded its size limit.".into());
        }
        if position + length > bytes.len() {
            return Ok(None);
        }
        position += length;
        entropy = marker == 0xda;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn preview_parser_respects_segment_lengths_and_entropy_escapes() {
        let jpeg = vec![
            0xff, 0xd8, 0xff, 0xe0, 0, 6, 0xff, 0xd9, 0xff, 0xd8, 0xff, 0xda, 0, 2, 9, 0xff, 0, 8,
            0xff, 0xd0, 7, 0xff, 0xd9,
        ];
        for split in 1..jpeg.len() {
            assert_eq!(jpeg_end(&jpeg[..split]).unwrap(), None);
        }
        let mut joined = jpeg.clone();
        joined.extend(&jpeg);
        let mut frames = Vec::new();
        read_frames(Cursor::new(joined), |frame| {
            frames.push(frame);
            true
        })
        .unwrap();
        assert_eq!(frames, [jpeg.clone(), jpeg]);
    }

    #[test]
    fn preview_parser_rejects_oversize_truncated_and_invalid_input() {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xda, 0, 2];
        bytes.resize(MAX_FRAME_BYTES + 1, 3);
        assert!(read_frames(Cursor::new(bytes), |_| true)
            .unwrap_err()
            .contains("size limit"));
        assert!(read_frames(Cursor::new([0xff, 0xd8, 0xff]), |_| true).is_err());
        assert!(jpeg_end(&[0xff, 0xd8, 0xff, 0xe0, 0, 1]).is_err());
        assert!(jpeg_end(b"invalid").is_err());
    }

    #[test]
    #[ignore = "Synthetic FFmpeg only; requires SCREENRECORD_FFMPEG. No camera is opened."]
    fn media_contract_preview_frames_stop_drop_and_timeout() {
        let ffmpeg = std::env::var_os("SCREENRECORD_FFMPEG").expect("SCREENRECORD_FFMPEG");
        let mut command = process::command(Path::new(&ffmpeg));
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=480x270:rate=8",
        ]);
        let mut preview =
            CameraPreview::spawn(command, Duration::from_secs(5), Duration::from_secs(2)).unwrap();
        let pid = preview.resources.as_ref().unwrap().child.id();
        let started = Instant::now();
        while preview.snapshot().sequence < 5 && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(30));
        }
        let snapshot = preview.snapshot();
        assert!(snapshot.active, "{:?}", snapshot.error);
        assert!(snapshot.sequence >= 5);
        let jpeg = snapshot.jpeg.unwrap();
        assert_eq!(jpeg_end(&jpeg).unwrap(), Some(jpeg.len()));
        assert!(jpeg.len() < MAX_FRAME_BYTES);
        let stopping = Instant::now();
        preview.stop().unwrap();
        assert!(stopping.elapsed() < Duration::from_millis(2500));
        preview.stop().unwrap();
        assert!(!preview.snapshot().active);
        assert!(preview.snapshot().jpeg.is_none());
        assert_exited(pid);
        let mut command = process::command(Path::new(&ffmpeg));
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=480x270:rate=1",
        ]);
        let preview =
            CameraPreview::spawn(command, Duration::from_secs(5), Duration::from_secs(2)).unwrap();
        let pid = preview.resources.as_ref().unwrap().child.id();
        drop(preview);
        assert_exited(pid);
        let mut command = process::command(Path::new(&ffmpeg));
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=480x270:rate=1/100",
        ]);
        let mut preview = CameraPreview::spawn(
            command,
            Duration::from_millis(200),
            Duration::from_millis(200),
        )
        .unwrap();
        let started = Instant::now();
        while preview.snapshot().active && started.elapsed() < Duration::from_secs(3) {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(preview.snapshot().error.unwrap().contains("timed out"));
        preview.stop().unwrap();
        let mut command = process::command(Path::new(&ffmpeg));
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "not_a_real_filter",
        ]);
        let mut preview =
            CameraPreview::spawn(command, Duration::from_secs(5), Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        while preview.snapshot().active && started.elapsed() < Duration::from_secs(3) {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!preview.snapshot().active);
        assert!(preview.snapshot().error.unwrap().contains("ended"));
        preview.stop().unwrap();
    }

    fn assert_exited(pid: u32) {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        unsafe {
            if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut code = 259;
                GetExitCodeProcess(handle, &mut code).unwrap();
                let _ = CloseHandle(handle);
                assert_ne!(code, 259);
            }
        }
    }
}
