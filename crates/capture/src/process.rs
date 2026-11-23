use crate::media::RecoveryError;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub struct ChildJob(usize);

impl ChildJob {
    pub fn terminate(&self) -> Result<(), String> {
        unsafe {
            windows::Win32::System::JobObjects::TerminateJobObject(
                windows::Win32::Foundation::HANDLE(self.0 as *mut _),
                1,
            )
            .map_err(|error| error.to_string())
        }
    }

    pub fn attach(child: &Child) -> Result<Self, String> {
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
            let configure = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
            .and_then(|_| AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle())));
            if let Err(error) = configure {
                let mut exit_code = 259u32;
                let exited = windows::Win32::System::Threading::GetExitCodeProcess(
                    HANDLE(child.as_raw_handle()),
                    &mut exit_code,
                )
                .is_ok()
                    && exit_code != 259;
                if !exited {
                    let _ = CloseHandle(handle);
                    return Err(format!("Cannot secure the media process lifetime: {error}"));
                }
            }
            Ok(Self(handle.0 as usize))
        }
    }
}

impl Drop for ChildJob {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(
                self.0 as *mut _,
            ));
        }
    }
}

pub fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(0x0800_0000);
    command
}

pub fn wait_bounded(
    child: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Ok(status);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Media process exceeded its shutdown deadline.".into());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

pub fn run_logged(program: &Path, args: &[String], log_path: &Path) -> Result<(), String> {
    run_logged_classified(program, args, log_path).map_err(String::from)
}

pub(crate) fn run_logged_classified(
    program: &Path,
    args: &[String],
    log_path: &Path,
) -> Result<(), RecoveryError> {
    let stall = Duration::from_secs(90);
    let log = File::create(log_path).map_err(|error| error.to_string())?;
    let mut child = command(program)
        .args(["-progress", "pipe:1", "-stats_period", "0.25"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|error| format!("Cannot start media processor: {error}"))?;
    let job = match ChildJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
    };
    let stdout = child
        .stdout
        .take()
        .ok_or("Media progress pipe is missing.")?;
    let (sender, receiver) = mpsc::sync_channel(16);
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some((key, value)) = line.split_once('=') {
                let kind = match key {
                    "out_time_us" => 0,
                    "total_size" => 1,
                    _ => continue,
                };
                if let Ok(value) = value.parse::<u64>() {
                    let _ = sender.try_send((kind, value));
                }
            }
        }
    });
    let started = Instant::now();
    let mut progress = ProgressWatch::default();
    let result = loop {
        for (kind, value) in receiver.try_iter() {
            progress.observe(kind, value, started.elapsed());
        }
        let mut io = windows::Win32::System::Threading::IO_COUNTERS::default();
        if unsafe {
            windows::Win32::System::Threading::GetProcessIoCounters(
                windows::Win32::Foundation::HANDLE(child.as_raw_handle()),
                &mut io,
            )
        }
        .is_ok()
        {
            // MP4 faststart copies file data after the final timestamp progress update.
            progress.observe(2, io.ReadTransferCount, started.elapsed());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(error) => break Err(error.to_string()),
            _ => {}
        }
        if progress.stalled(started.elapsed(), stall) {
            break Err(format!(
                "Media processing stopped making progress for {:.1} seconds.",
                stall.as_secs_f64()
            ));
        }
        thread::sleep(Duration::from_millis(25));
    };
    let _ = job.terminate();
    let _ = child.wait();
    let _ = reader.join();
    let status = result?;
    if !status.success() {
        let log = std::fs::read_to_string(log_path).unwrap_or_default();
        return Err(RecoveryError::Unusable(format!(
            "Media processing failed: {}",
            tail(&log, 2000)
        )));
    }
    Ok(())
}

#[derive(Default)]
struct ProgressWatch {
    values: [u64; 3],
    last: Duration,
}
impl ProgressWatch {
    fn observe(&mut self, kind: usize, value: u64, now: Duration) {
        if value > self.values[kind] {
            self.values[kind] = value;
            self.last = now;
        }
    }
    fn stalled(&self, now: Duration, limit: Duration) -> bool {
        now.saturating_sub(self.last) >= limit
    }
}

pub(crate) fn read_output(
    program: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<Vec<u8>, RecoveryError> {
    let mut child = command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let job = ChildJob::attach(&child).inspect_err(|_| {
        let _ = child.kill();
        let _ = child.wait();
    })?;
    let stdout = child.stdout.take().ok_or("Media output pipe is missing.")?;
    let stderr = child.stderr.take().ok_or("Media error pipe is missing.")?;
    let read = |pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.take(65_537)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() > 65_536 {
                return Err("Media metadata exceeds its supported size.".to_owned());
            }
            Ok(bytes)
        })
    };
    let output = read(Box::new(stdout));
    let errors = read(Box::new(stderr));
    let status = wait_bounded(&mut child, timeout);
    let _ = job.terminate();
    let output = output
        .join()
        .map_err(|_| "Media metadata reader failed.")??;
    let errors = errors.join().map_err(|_| "Media error reader failed.")??;
    if !status?.success() {
        return Err(RecoveryError::Unusable(format!(
            "Media probe failed: {}",
            tail(&String::from_utf8_lossy(&errors), 2000)
        )));
    }
    Ok(output)
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CameraStopProgress {
    pub frame: u64,
    pub output_us: u64,
    pub total_bytes: u64,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CameraStopOutcome {
    Exited,
    DeadlineTerminated,
}

#[derive(Clone, Copy)]
struct CameraStopLimits {
    stalled: Duration,
    after_end: Duration,
    total: Duration,
}

struct CameraStopWatch {
    progress: CameraStopProgress,
    advanced_at: Duration,
    completed_at: Option<Duration>,
}

impl CameraStopWatch {
    fn new(progress: CameraStopProgress) -> Self {
        Self {
            completed_at: progress.complete.then_some(Duration::ZERO),
            progress,
            advanced_at: Duration::ZERO,
        }
    }

    fn expired(
        &mut self,
        progress: CameraStopProgress,
        now: Duration,
        limits: CameraStopLimits,
    ) -> bool {
        if progress.frame > self.progress.frame
            || progress.output_us > self.progress.output_us
            || progress.total_bytes > self.progress.total_bytes
        {
            self.advanced_at = now;
        }
        self.progress.frame = self.progress.frame.max(progress.frame);
        self.progress.output_us = self.progress.output_us.max(progress.output_us);
        self.progress.total_bytes = self.progress.total_bytes.max(progress.total_bytes);
        if progress.complete && self.completed_at.is_none() {
            self.completed_at = Some(now);
        }
        now >= limits.total
            || self.completed_at.map_or_else(
                || now.saturating_sub(self.advanced_at) >= limits.stalled,
                |completed| now.saturating_sub(completed) >= limits.after_end,
            )
    }
}

pub(crate) fn stop_camera_child_with_progress(
    child: &mut Child,
    progress: impl FnMut() -> CameraStopProgress,
) -> Result<CameraStopOutcome, String> {
    stop_camera_with_limits(
        child,
        progress,
        CameraStopLimits {
            stalled: Duration::from_secs(2),
            after_end: Duration::from_secs(2),
            total: Duration::from_secs(10),
        },
    )
}

fn stop_camera_with_limits(
    child: &mut Child,
    mut progress: impl FnMut() -> CameraStopProgress,
    limits: CameraStopLimits,
) -> Result<CameraStopOutcome, String> {
    let mut watch = CameraStopWatch::new(progress());
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"q\n");
        let _ = stdin.flush();
    }
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return if status.success() {
                Ok(CameraStopOutcome::Exited)
            } else {
                Err(format!("The webcam process exited with {status}."))
            };
        }
        if watch.expired(progress(), started.elapsed(), limits) {
            // Killing is not proof that the output completed; the camera owner validates it.
            match child.kill() {
                Ok(()) => return Ok(CameraStopOutcome::DeadlineTerminated),
                Err(error) => {
                    if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                        return if status.success() {
                            Ok(CameraStopOutcome::Exited)
                        } else {
                            Err(format!("The webcam process exited with {status}."))
                        };
                    }
                    return Err(format!("Cannot terminate the webcam process: {error}"));
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn tail(text: &str, count: usize) -> &str {
    let mut start = text.len().saturating_sub(count);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

#[cfg(test)]
pub(crate) fn fixture_child(ignore_stop: bool) -> Child {
    command(&std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "process::tests::stop_child_fixture",
            "--nocapture",
        ])
        .env(
            "SCREENRECORD_CHILD_FIXTURE",
            if ignore_stop { "ignore" } else { "cooperate" },
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progressing_long_media_has_no_fixed_duration_deadline_but_repeated_stats_stall() {
        let mut progress = ProgressWatch::default();
        let limit = Duration::from_secs(90);
        for second in 1..=600 {
            progress.observe(0, second, Duration::from_secs(second));
            assert!(!progress.stalled(Duration::from_secs(second + 1), limit));
        }
        progress.observe(0, 600, Duration::from_secs(650));
        progress.observe(1, 0, Duration::from_secs(680));
        assert!(progress.stalled(Duration::from_secs(690), limit));
        progress.observe(1, 1024, Duration::from_secs(690));
        assert!(!progress.stalled(Duration::from_secs(700), limit));
        progress.observe(2, 10_000_000, Duration::from_secs(750));
        assert!(!progress.stalled(Duration::from_secs(800), limit));
    }

    #[test]
    #[ignore = "Executed only as an owned child process by the shutdown tests."]
    fn stop_child_fixture() {
        match std::env::var("SCREENRECORD_CHILD_FIXTURE").as_deref() {
            Ok("ignore") => thread::sleep(Duration::from_secs(60)),
            Ok("cooperate") => {
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).unwrap();
                assert_eq!(input.trim(), "q");
            }
            Ok(mode @ ("slow-camera" | "ended-camera" | "busy-camera" | "failed-camera")) => {
                println!("CAMERA_FIXTURE_READY");
                std::io::stdout().flush().unwrap();
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).unwrap();
                assert_eq!(input.trim(), "q");
                if mode == "failed-camera" {
                    std::process::exit(7);
                }
                if mode == "ended-camera" {
                    println!("complete");
                    std::io::stdout().flush().unwrap();
                    thread::sleep(Duration::from_secs(60));
                } else {
                    for index in 1..=if mode == "busy-camera" { 6000 } else { 8 } {
                        println!("frame={index}");
                        std::io::stdout().flush().unwrap();
                        thread::sleep(Duration::from_millis(30));
                    }
                }
            }
            _ => panic!("The child fixture requires an explicit test mode."),
        }
    }

    #[test]
    fn an_unresponsive_capture_process_is_terminated_at_the_deadline() {
        let mut child = fixture_child(true);
        let _job = ChildJob::attach(&child).unwrap();
        let started = Instant::now();
        assert_eq!(
            stop_camera_with_limits(
                &mut child,
                CameraStopProgress::default,
                CameraStopLimits {
                    stalled: Duration::from_millis(60),
                    after_end: Duration::from_millis(60),
                    total: Duration::from_secs(1),
                }
            )
            .unwrap(),
            CameraStopOutcome::DeadlineTerminated
        );
        assert!(!wait_bounded(&mut child, Duration::from_secs(1))
            .unwrap()
            .success());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn camera_stop_tracks_real_progress_but_bounds_stalls_teardown_and_total_time() {
        let limits = CameraStopLimits {
            stalled: Duration::from_secs(2),
            after_end: Duration::from_secs(2),
            total: Duration::from_secs(10),
        };
        let mut watch = CameraStopWatch::new(CameraStopProgress::default());
        for second in 1..10 {
            assert!(!watch.expired(
                CameraStopProgress {
                    frame: second,
                    ..Default::default()
                },
                Duration::from_secs(second),
                limits
            ));
        }
        assert!(watch.expired(
            CameraStopProgress {
                frame: 10,
                ..Default::default()
            },
            Duration::from_secs(10),
            limits
        ));
        let mut stalled = CameraStopWatch::new(CameraStopProgress {
            frame: 5,
            ..Default::default()
        });
        assert!(!stalled.expired(
            CameraStopProgress {
                frame: 5,
                ..Default::default()
            },
            Duration::from_secs(1),
            limits
        ));
        assert!(stalled.expired(
            CameraStopProgress {
                frame: 5,
                ..Default::default()
            },
            Duration::from_secs(2),
            limits
        ));
        let mut ended = CameraStopWatch::new(CameraStopProgress::default());
        assert!(!ended.expired(
            CameraStopProgress {
                complete: true,
                ..Default::default()
            },
            Duration::from_secs(1),
            limits
        ));
        assert!(!ended.expired(
            CameraStopProgress {
                frame: 100,
                complete: true,
                ..Default::default()
            },
            Duration::from_secs(2),
            limits
        ));
        assert!(ended.expired(
            CameraStopProgress {
                frame: 200,
                complete: true,
                ..Default::default()
            },
            Duration::from_secs(3),
            limits
        ));
    }

    fn camera_fixture(
        mode: &str,
    ) -> (
        Child,
        ChildJob,
        std::sync::Arc<std::sync::Mutex<CameraStopProgress>>,
        thread::JoinHandle<()>,
    ) {
        let mut child = command(&std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "process::tests::stop_child_fixture",
                "--nocapture",
            ])
            .env("SCREENRECORD_CHILD_FIXTURE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let job = ChildJob::attach(&child).unwrap();
        let progress = std::sync::Arc::new(std::sync::Mutex::new(CameraStopProgress::default()));
        let observed = progress.clone();
        let (ready, started) = mpsc::sync_channel(1);
        let pipe = child.stdout.take().unwrap();
        let reader = thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if line.contains("CAMERA_FIXTURE_READY") {
                    let _ = ready.try_send(());
                }
                let mut state = observed.lock().unwrap();
                if line == "complete" {
                    state.complete = true;
                }
                if let Some(frame) = line
                    .strip_prefix("frame=")
                    .and_then(|value| value.parse().ok())
                {
                    state.frame = frame;
                }
            }
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        (child, job, progress, reader)
    }

    #[test]
    fn camera_flush_continues_while_progressing_and_failed_exit_stays_an_error() {
        for mode in ["slow-camera", "failed-camera"] {
            let (mut child, _job, progress, reader) = camera_fixture(mode);
            let result = stop_camera_with_limits(
                &mut child,
                || *progress.lock().unwrap(),
                CameraStopLimits {
                    stalled: Duration::from_millis(150),
                    after_end: Duration::from_millis(150),
                    total: Duration::from_secs(2),
                },
            );
            child.wait().unwrap();
            reader.join().unwrap();
            if mode == "slow-camera" {
                assert_eq!(result.unwrap(), CameraStopOutcome::Exited);
                assert_eq!(progress.lock().unwrap().frame, 8);
            } else {
                assert!(result.unwrap_err().contains("exited"));
            }
        }
    }

    #[test]
    fn camera_lingering_driver_and_endless_progress_are_terminated_and_reaped() {
        for mode in ["ended-camera", "busy-camera"] {
            let (mut child, _job, progress, reader) = camera_fixture(mode);
            let began = Instant::now();
            let result = stop_camera_with_limits(
                &mut child,
                || *progress.lock().unwrap(),
                CameraStopLimits {
                    stalled: Duration::from_millis(150),
                    after_end: Duration::from_millis(50),
                    total: Duration::from_millis(400),
                },
            );
            assert_eq!(result.unwrap(), CameraStopOutcome::DeadlineTerminated);
            child.wait().unwrap();
            reader.join().unwrap();
            assert!(began.elapsed() < Duration::from_secs(2));
            assert_eq!(progress.lock().unwrap().complete, mode == "ended-camera");
        }
    }
}
