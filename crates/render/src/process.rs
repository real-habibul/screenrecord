use screenrecord_core::ExportProgress;
use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub(crate) type Progress = Arc<Mutex<Box<dyn Fn(ExportProgress) + Send>>>;

pub(crate) fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

pub(crate) fn emit(
    callback: &Progress,
    state: &str,
    value: f64,
    path: &str,
    error: Option<String>,
) {
    emit_with_bytes(callback, state, value, path, error, None);
}

pub(crate) fn emit_with_bytes(
    callback: &Progress,
    state: &str,
    value: f64,
    path: &str,
    error: Option<String>,
    output_bytes: Option<u64>,
) {
    if let Ok(callback) = callback.lock() {
        callback(ExportProgress {
            state: state.into(),
            progress: value,
            output_path: path.into(),
            error,
            output_bytes,
        });
    }
}

struct ChildGuard(Child, #[allow(dead_code)] ChildJob);
impl ChildGuard {
    fn spawn(command: &mut Command) -> Result<Self, String> {
        let mut child = command
            .spawn()
            .map_err(|error| format!("Cannot start media tool: {error}"))?;
        match ChildJob::attach(&mut child) {
            Ok(job) => Ok(Self(child, job)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct ChildJob(#[allow(dead_code)] Option<usize>);
impl ChildJob {
    fn attach(child: &mut Child) -> Result<Self, String> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows::Win32::{
                Foundation::{CloseHandle, HANDLE},
                System::JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                },
            };
            unsafe {
                let handle = CreateJobObjectW(None, None).map_err(|error| error.to_string())?;
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let configured = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
                .and_then(|_| AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle())));
                if let Err(error) = configured {
                    let _ = CloseHandle(handle);
                    if child.try_wait().ok().flatten().is_some() {
                        return Ok(Self(None));
                    }
                    return Err(format!("Cannot secure the media process lifetime: {error}"));
                }
                Ok(Self(Some(handle.0 as usize)))
            }
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Ok(Self(None))
        }
    }
}
impl Drop for ChildJob {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(handle) = self.0 {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(
                    windows::Win32::Foundation::HANDLE(handle as *mut _),
                );
            }
        }
    }
}

fn read_bounded(mut reader: impl Read, capacity: usize, tail: bool) -> Vec<u8> {
    let mut result = Vec::new();
    let mut chunk = [0u8; 4096];
    while let Ok(count) = reader.read(&mut chunk) {
        if count == 0 {
            break;
        }
        if tail {
            result.extend_from_slice(&chunk[..count]);
            if result.len() > capacity {
                result.drain(..result.len() - capacity);
            }
        } else {
            let keep = count.min(capacity.saturating_sub(result.len()));
            result.extend_from_slice(&chunk[..keep]);
        }
    }
    result
}

pub(crate) fn capture(
    command: &mut Command,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<(bool, Vec<u8>, String), String> {
    hidden(command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ChildGuard::spawn(command)?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or("Media tool stdout is unavailable.")?;
    let stderr = child
        .0
        .stderr
        .take()
        .ok_or("Media tool stderr is unavailable.")?;
    let output = thread::spawn(move || read_bounded(stdout, 1024 * 1024, false));
    let errors = thread::spawn(move || read_bounded(stderr, 16 * 1024, true));
    let start = Instant::now();
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err("Export cancelled.".to_string());
        }
        if start.elapsed() > timeout {
            break Err("Media tool did not respond within its time limit.".to_string());
        }
        match child.0.try_wait() {
            Ok(Some(status)) => break Ok(status.success()),
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => break Err(error.to_string()),
        }
    };
    if result.is_err() {
        let _ = child.0.kill();
        let _ = child.0.wait();
    }
    let output = output.join().unwrap_or_default();
    let errors = String::from_utf8_lossy(&errors.join().unwrap_or_default()).into_owned();
    result.map(|success| (success, output, errors))
}

pub(crate) fn encode(
    command: &mut Command,
    cancel: &Arc<AtomicBool>,
    callback: &Progress,
    duration: f64,
    path: &str,
    progress_range: std::ops::Range<f64>,
) -> Result<(), EncodeError> {
    encode_with_timeout(
        command,
        cancel,
        callback,
        duration,
        path,
        progress_range,
        Duration::from_secs(180),
    )
}

pub(crate) fn stream_audio(
    command: &mut Command,
    cancel: &AtomicBool,
    callback: &Progress,
    samples: u64,
    path: &str,
    range: std::ops::Range<f64>,
    consume: impl FnOnce(std::process::ChildStdout, &AtomicU64) -> Result<(), String> + Send,
) -> Result<(), String> {
    hidden(command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ChildGuard::spawn(command)?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or("The audio decoder stream is unavailable.")?;
    let stderr = child
        .0
        .stderr
        .take()
        .ok_or("The audio decoder diagnostics are unavailable.")?;
    let completed = AtomicU64::new(0);
    thread::scope(|scope| {
        let mut consumer = Some(scope.spawn(|| consume(stdout, &completed)));
        let mut consumed = None;
        let errors = scope.spawn(|| read_bounded(stderr, 16 * 1024, true));
        let mut last_samples = 0;
        let mut last_work = Instant::now();
        let result = loop {
            if consumer
                .as_ref()
                .is_some_and(|consumer| consumer.is_finished())
            {
                consumed =
                    Some(
                        consumer.take().unwrap().join().unwrap_or_else(|_| {
                            Err("Audio processing stopped unexpectedly.".into())
                        }),
                    );
                if let Some(Err(error)) = &consumed {
                    break Err(error.clone());
                }
            }
            if cancel.load(Ordering::Relaxed) {
                break Err("Export cancelled.".to_string());
            }
            let count = completed.load(Ordering::Relaxed);
            if count > last_samples {
                last_samples = count;
                last_work = Instant::now();
                emit(
                    callback,
                    "running",
                    range.start
                        + (count as f64 / samples as f64).min(1.0) * (range.end - range.start),
                    path,
                    None,
                );
            }
            if last_work.elapsed() > Duration::from_secs(180) {
                break Err("The audio decoder made no sample progress for 180 seconds.".to_string());
            }
            match child.0.try_wait() {
                Ok(Some(status)) => {
                    break if status.success() {
                        Ok(())
                    } else {
                        Err(format!("The audio decoder exited with {status}."))
                    }
                }
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(error) => break Err(error.to_string()),
            }
        };
        if result.is_err() {
            let _ = child.0.kill();
            let _ = child.0.wait();
        }
        let consumed = consumed.unwrap_or_else(|| {
            consumer
                .take()
                .unwrap()
                .join()
                .unwrap_or_else(|_| Err("Audio processing stopped unexpectedly.".into()))
        });
        let diagnostics = String::from_utf8_lossy(&errors.join().unwrap_or_default()).into_owned();
        if cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled.".into());
        }
        result.map_err(|error| format!("{error}\n{diagnostics}"))?;
        consumed?;
        emit(callback, "running", range.end, path, None);
        Ok(())
    })
}

#[derive(Debug)]
pub(crate) struct EncodeError {
    message: String,
    pub retry_with_software: bool,
}
impl From<String> for EncodeError {
    fn from(message: String) -> Self {
        Self {
            message,
            retry_with_software: false,
        }
    }
}
impl From<&str> for EncodeError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl From<EncodeError> for String {
    fn from(error: EncodeError) -> Self {
        error.message
    }
}

struct CountedReader<T> {
    reader: T,
    bytes: Arc<AtomicU64>,
}
impl<T: Read> Read for CountedReader<T> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.reader.read(buffer)?;
        self.bytes.fetch_add(count as u64, Ordering::Relaxed);
        Ok(count)
    }
}

struct ProgressClock {
    last_work: Instant,
    time: u64,
    frames: u64,
    size: u64,
    io_bytes: u64,
}
impl ProgressClock {
    fn new(now: Instant) -> Self {
        Self {
            last_work: now,
            time: 0,
            frames: 0,
            size: 0,
            io_bytes: 0,
        }
    }
    fn report(&mut self, line: &str, now: Instant) -> Option<u64> {
        let (key, value) = line.trim().split_once('=')?;
        let value = value.parse::<u64>().ok()?;
        let current = match key {
            "out_time_us" => &mut self.time,
            "frame" => &mut self.frames,
            "total_size" => &mut self.size,
            _ => return None,
        };
        if value > *current {
            *current = value;
            self.last_work = now;
        }
        (key == "out_time_us").then_some(self.time)
    }
    fn writes(&mut self, process_bytes: u64, pipe_bytes: u64, now: Instant) {
        // Faststart rewrites existing file bytes after timestamp progress has stopped.
        let useful = process_bytes.saturating_sub(pipe_bytes);
        if useful.saturating_sub(self.io_bytes) >= 64 * 1024 {
            self.io_bytes = useful;
            self.last_work = now;
        }
    }
    fn expired(&self, now: Instant, timeout: Duration) -> bool {
        now.duration_since(self.last_work) > timeout
    }
}

fn process_write_bytes(child: &Child) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            System::Threading::{GetProcessIoCounters, IO_COUNTERS},
        };
        let mut counters = IO_COUNTERS::default();
        unsafe {
            GetProcessIoCounters(HANDLE(child.as_raw_handle()), &mut counters).ok()?;
        }
        Some(counters.WriteTransferCount)
    }
    #[cfg(not(windows))]
    {
        let _ = child;
        None
    }
}

fn recoverable_encoder_error(diagnostics: &str) -> bool {
    let lower = diagnostics.to_ascii_lowercase();
    if [
        "no space left",
        "disk full",
        "permission denied",
        "input/output error",
        "error opening input",
        "invalid data found",
        "error while filtering",
        "error parsing",
        "no such file",
        "could not write",
        "error writing",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        return false;
    }
    lower.lines().any(|line| {
        line.starts_with('[')
            && ["h264_nvenc", "h264_qsv", "h264_amf", "h264_mf"]
                .iter()
                .any(|name| {
                    line.split(']')
                        .next()
                        .is_some_and(|prefix| prefix.contains(name))
                })
            && [
                "failed",
                "error",
                "unsupported",
                "not supported",
                "cannot",
                "could not",
                "no capable",
            ]
            .iter()
            .any(|pattern| line.contains(pattern))
    })
}

#[cfg(windows)]
pub(crate) fn available_bytes(directory: &Path) -> Option<u64> {
    use windows::{core::HSTRING, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};
    let mut free = 0;
    unsafe {
        GetDiskFreeSpaceExW(
            &HSTRING::from(directory.as_os_str()),
            Some(&mut free),
            None,
            None,
        )
    }
    .ok()?;
    Some(free)
}

fn output_write_error(directory: Option<&Path>, status: &str, diagnostics: &str) -> String {
    let detail = format!("{status}\n{}", diagnostics.trim());
    let lower = diagnostics.to_ascii_lowercase();
    if [
        "no space left",
        "disk full",
        "disk is full",
        "not enough space on the disk",
    ]
    .iter()
    .any(|reason| lower.contains(reason))
    {
        return format!(
            "The output folder is out of disk space. Free space and try again.\n{detail}"
        );
    }
    #[cfg(windows)]
    if [
        "could not write",
        "error writing",
        "error submitting a packet to the muxer",
        "error muxing",
        "error closing file",
    ]
    .iter()
    .any(|reason| lower.contains(reason))
    {
        if let Some(free) = directory
            .and_then(available_bytes)
            .filter(|bytes| *bytes < 1024 * 1024)
        {
            return format!(
                "The output folder has only {} KiB of disk space available.\n{detail}",
                free / 1024
            );
        }
    }
    #[cfg(not(windows))]
    let _ = directory;
    detail
}

fn encode_with_timeout(
    command: &mut Command,
    cancel: &Arc<AtomicBool>,
    callback: &Progress,
    duration: f64,
    path: &str,
    progress_range: std::ops::Range<f64>,
    timeout: Duration,
) -> Result<(), EncodeError> {
    hidden(command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ChildGuard::spawn(command)?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or("Encoder progress stream is unavailable.")?;
    let stderr = child
        .0
        .stderr
        .take()
        .ok_or("Encoder diagnostics are unavailable.")?;
    let pipe_bytes = Arc::new(AtomicU64::new(0));
    let stdout = CountedReader {
        reader: stdout,
        bytes: pipe_bytes.clone(),
    };
    let stderr = CountedReader {
        reader: stderr,
        bytes: pipe_bytes.clone(),
    };
    let last_progress = Arc::new(Mutex::new(ProgressClock::new(Instant::now())));
    let progress_time = last_progress.clone();
    let callback = callback.clone();
    let path = path.to_string();
    let progress_reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let time = progress_time
                .lock()
                .ok()
                .and_then(|mut progress| progress.report(&line, Instant::now()));
            if let Some(time) = time {
                emit(
                    &callback,
                    "running",
                    progress_range.start
                        + (time as f64 / 1_000_000.0 / duration).clamp(0.0, 1.0)
                            * (progress_range.end - progress_range.start),
                    &path,
                    None,
                );
            }
        }
    });
    let error_reader = thread::spawn(move || read_bounded(stderr, 16 * 1024, true));
    let mut exited_unsuccessfully = false;
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err("Export cancelled.".to_string());
        }
        if let Ok(mut progress) = last_progress.lock() {
            if let Some(written) = process_write_bytes(&child.0) {
                progress.writes(written, pipe_bytes.load(Ordering::Relaxed), Instant::now());
            }
            if progress.expired(Instant::now(), timeout) {
                break Err(format!("The media process made no timestamp, frame, or file-writing progress for {:.1} seconds.", timeout.as_secs_f64()));
            }
        }
        match child.0.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    exited_unsuccessfully = true;
                    Err(format!("FFmpeg exited with {status}."))
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => break Err(error.to_string()),
        }
    };
    if result.is_err() {
        let _ = child.0.kill();
        let _ = child.0.wait();
    }
    let _ = progress_reader.join();
    let diagnostics =
        String::from_utf8_lossy(&error_reader.join().unwrap_or_default()).into_owned();
    result.map_err(|error| {
        if cancel.load(Ordering::Relaxed) {
            EncodeError::from(error)
        } else {
            EncodeError {
                message: output_write_error(command.get_current_dir(), &error, &diagnostics),
                retry_with_software: exited_unsuccessfully
                    && recoverable_encoder_error(&diagnostics),
            }
        }
    })
}

pub(crate) fn media_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(&path).to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoder {
    Nvidia,
    Intel,
    Amd,
    MediaFoundation,
    SoftwareMf,
    SoftwareX264,
}

#[derive(Clone, Copy)]
pub(crate) struct VideoSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Encoder {
    pub(crate) fn args_for_bitrate(self, bitrate: Option<u32>) -> Vec<String> {
        let Some(bitrate) = bitrate else {
            return self.args().into_iter().map(str::to_owned).collect();
        };
        let base = match self {
            Self::Nvidia => vec!["-c:v", "h264_nvenc", "-preset", "p4", "-rc", "cbr"],
            Self::Intel => vec!["-c:v", "h264_qsv", "-preset", "medium", "-look_ahead", "0"],
            Self::Amd => vec![
                "-c:v",
                "h264_amf",
                "-quality",
                "balanced",
                "-rc",
                "cbr",
                "-enforce_hrd",
                "1",
                "-filler_data",
                "0",
            ],
            Self::MediaFoundation => vec![
                "-c:v",
                "h264_mf",
                "-hw_encoding",
                "1",
                "-rate_control",
                "cbr",
            ],
            Self::SoftwareMf => vec![
                "-c:v",
                "h264_mf",
                "-hw_encoding",
                "0",
                "-rate_control",
                "cbr",
            ],
            Self::SoftwareX264 => vec!["-c:v", "libx264", "-preset", "medium", "-threads", "4"],
        };
        let mut args: Vec<String> = base.into_iter().map(str::to_owned).collect();
        args.extend([
            "-b:v".into(),
            bitrate.to_string(),
            "-maxrate".into(),
            bitrate.to_string(),
            "-bufsize".into(),
            (u64::from(bitrate) * 2).to_string(),
            "-g".into(),
            "60".into(),
        ]);
        args
    }
    pub(crate) fn args(self) -> Vec<&'static str> {
        match self {
            Self::Nvidia => vec![
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p4",
                "-rc",
                "vbr",
                "-cq",
                "20",
                "-b:v",
                "0",
            ],
            Self::Intel => vec![
                "-c:v",
                "h264_qsv",
                "-preset",
                "medium",
                "-global_quality",
                "22",
            ],
            Self::Amd => vec![
                "-c:v", "h264_amf", "-quality", "balanced", "-rc", "cqp", "-qp_i", "20", "-qp_p",
                "20",
            ],
            Self::MediaFoundation => vec!["-c:v", "h264_mf", "-hw_encoding", "1", "-b:v", "16M"],
            Self::SoftwareMf => vec![
                "-c:v",
                "h264_mf",
                "-hw_encoding",
                "0",
                "-rate_control",
                "quality",
                "-quality",
                "85",
                "-b:v",
                "16M",
            ],
            Self::SoftwareX264 => vec![
                "-c:v", "libx264", "-preset", "medium", "-crf", "20", "-threads", "4",
            ],
        }
    }
    pub(crate) fn is_software(self) -> bool {
        matches!(self, Self::SoftwareMf | Self::SoftwareX264)
    }
}

pub(crate) fn choose_encoder(
    ffmpeg: &Path,
    spec: VideoSpec,
    cancel: &AtomicBool,
) -> Result<Encoder, String> {
    choose_encoder_for_bitrate(ffmpeg, spec, cancel, None)
}

pub(crate) fn choose_encoder_for_bitrate(
    ffmpeg: &Path,
    spec: VideoSpec,
    cancel: &AtomicBool,
    bitrate: Option<u32>,
) -> Result<Encoder, String> {
    for encoder in [
        Encoder::Nvidia,
        Encoder::Intel,
        Encoder::Amd,
        Encoder::MediaFoundation,
    ] {
        if cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled.".into());
        }
        let mut command = Command::new(ffmpeg);
        command
            .args([
                "-hide_banner",
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                &format!(
                    "color=c=black:s={}x{}:r={}:d=0.2",
                    spec.width, spec.height, spec.fps
                ),
                "-frames:v",
                "3",
                "-an",
            ])
            .args(encoder.args_for_bitrate(bitrate))
            .args(["-f", "null", "-"]);
        if capture(&mut command, Duration::from_secs(8), cancel)
            .is_ok_and(|(success, _, _)| success)
        {
            return Ok(encoder);
        }
    }
    software_encoder_for_bitrate(ffmpeg, spec, cancel, bitrate)
}

pub(crate) fn software_encoder(
    ffmpeg: &Path,
    spec: VideoSpec,
    cancel: &AtomicBool,
) -> Result<Encoder, String> {
    software_encoder_for_bitrate(ffmpeg, spec, cancel, None)
}

pub(crate) fn software_encoder_for_bitrate(
    ffmpeg: &Path,
    spec: VideoSpec,
    cancel: &AtomicBool,
    bitrate: Option<u32>,
) -> Result<Encoder, String> {
    for encoder in [Encoder::SoftwareMf, Encoder::SoftwareX264] {
        if cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled.".into());
        }
        let mut command = Command::new(ffmpeg);
        command
            .args([
                "-hide_banner",
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                &format!(
                    "color=c=black:s={}x{}:r={}:d=0.2",
                    spec.width, spec.height, spec.fps
                ),
                "-frames:v",
                "3",
                "-an",
            ])
            .args(encoder.args_for_bitrate(bitrate))
            .args(["-f", "null", "-"]);
        if capture(&mut command, Duration::from_secs(8), cancel)
            .is_ok_and(|(success, _, _)| success)
        {
            return Ok(encoder);
        }
    }
    Err(format!("No H.264 encoder accepted {}x{} at {} fps. Choose a smaller output or check the Windows Media Feature Pack.", spec.width, spec.height, spec.fps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_reports_do_not_extend_the_work_deadline() {
        let start = Instant::now();
        let timeout = Duration::from_secs(180);
        let mut clock = ProgressClock::new(start);
        assert_eq!(clock.report("out_time_us=1000000", start), Some(1_000_000));
        for second in 1..=181 {
            let now = start + Duration::from_secs(second);
            clock.report("out_time_us=1000000", now);
            clock.report("out_time_us=500000", now);
            clock.writes(second * 4096, second * 4096, now);
        }
        assert!(clock.expired(start + Duration::from_secs(181), timeout));
        clock.report("frame=61", start + Duration::from_secs(181));
        assert!(!clock.expired(start + Duration::from_secs(182), timeout));
    }

    #[test]
    fn finalization_file_writes_keep_the_job_alive_after_timestamps_stop() {
        let start = Instant::now();
        let mut clock = ProgressClock::new(start);
        clock.report("out_time_us=60000000", start);
        for step in 1..=60 {
            let now = start + Duration::from_secs(step * 30);
            let pipe_bytes = step * 10_000;
            clock.writes(pipe_bytes + step * 1024 * 1024, pipe_bytes, now);
            assert!(!clock.expired(now, Duration::from_secs(180)));
        }
    }

    #[test]
    fn only_encoder_failures_are_eligible_for_software_retry() {
        assert!(recoverable_encoder_error(
            "[h264_nvenc @ 123] OpenEncodeSessionEx failed: out of memory (10)"
        ));
        assert!(recoverable_encoder_error(
            "[vost#0:0/h264_qsv @ 123] Error while opening encoder: unsupported (-3)"
        ));
        for diagnostics in [
            "[h264_amf @ 123] AMF encoder failed\nError writing packet: No space left on device",
            "[h264_amf @ 123] Error initializing encoder\nError while filtering: Invalid argument",
            "Error opening input: Invalid data found when processing input",
            "Could not write header: Permission denied",
            "[Parsed_crop_0 @ 123] Invalid too big width",
            "FFmpeg exited unexpectedly",
        ] {
            assert!(!recoverable_encoder_error(diagnostics), "{diagnostics}");
        }
    }

    #[test]
    fn progress_fixture_child() {
        let Ok(mode) = std::env::var("SCREENRECORD_RENDER_PROCESS_FIXTURE") else {
            return;
        };
        if mode == "quiet" {
            thread::sleep(Duration::from_secs(3));
            return;
        }
        use std::io::Write;
        let mut file = (mode == "writes").then(|| {
            std::fs::File::create(std::env::var_os("SCREENRECORD_RENDER_PROCESS_FILE").unwrap())
                .unwrap()
        });
        let bytes = vec![0_u8; 256 * 1024];
        for _ in 0..60 {
            println!("out_time_us=1000000");
            if let Some(file) = &mut file {
                file.write_all(&bytes).unwrap();
                file.flush().unwrap();
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(windows)]
    #[test]
    fn process_watchdog_stops_heartbeat_only_children_but_allows_active_file_writes() {
        let directory = tempfile::tempdir().unwrap();
        let callback: Progress = Arc::new(Mutex::new(Box::new(|_| {})));
        for mode in ["stall", "writes"] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "process::tests::progress_fixture_child",
                    "--nocapture",
                ])
                .env("SCREENRECORD_RENDER_PROCESS_FIXTURE", mode)
                .env(
                    "SCREENRECORD_RENDER_PROCESS_FILE",
                    directory.path().join("active-file.bin"),
                );
            let start = Instant::now();
            let result = encode_with_timeout(
                &mut command,
                &Arc::new(AtomicBool::new(false)),
                &callback,
                1.0,
                "test-output",
                0.0..1.0,
                Duration::from_secs(1),
            );
            if mode == "stall" {
                let error = result.unwrap_err();
                assert!(error.message.contains("made no"));
                assert!(!error.retry_with_software);
                assert!(start.elapsed() < Duration::from_secs(3));
            } else {
                result.unwrap();
                assert_eq!(
                    std::fs::metadata(directory.path().join("active-file.bin"))
                        .unwrap()
                        .len(),
                    60 * 256 * 1024
                );
            }
        }
    }

    #[test]
    fn audio_stream_cancellation_interrupts_a_blocked_consumer_and_reaps_the_child() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "process::tests::progress_fixture_child",
                "--nocapture",
            ])
            .env("SCREENRECORD_RENDER_PROCESS_FIXTURE", "stall");
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let timer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(120));
            signal.store(true, Ordering::Relaxed);
        });
        let callback: Progress = Arc::new(Mutex::new(Box::new(|_| {})));
        let started = Instant::now();
        let result = stream_audio(
            &mut command,
            &cancel,
            &callback,
            48_000,
            "test-output",
            0.0..1.0,
            |mut input, _| {
                std::io::copy(&mut input, &mut std::io::sink())
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            },
        );
        timer.join().unwrap();
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn audio_stream_consumer_failure_stops_a_decoder_that_is_still_waiting() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "process::tests::progress_fixture_child",
                "--nocapture",
            ])
            .env("SCREENRECORD_RENDER_PROCESS_FIXTURE", "quiet");
        let callback: Progress = Arc::new(Mutex::new(Box::new(|_| {})));
        let started = Instant::now();
        let error = stream_audio(
            &mut command,
            &AtomicBool::new(false),
            &callback,
            48_000,
            "test",
            0.0..1.0,
            |_, _| Err("Injected audio write failure".into()),
        )
        .unwrap_err();
        assert!(error.contains("Injected audio write failure"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
