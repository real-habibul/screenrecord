mod matcher;

use crate::{cleanup, process};
use screenrecord_core::{MaskKeyframe, MAX_MASK_TRACKING_SECONDS};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::{ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

const FPS: f64 = 10.0;

#[derive(Debug, Clone, Copy)]
pub struct TrackingRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub struct TrackingInput {
    pub source_path: PathBuf,
    pub ffmpeg_path: PathBuf,
    pub source_width: u32,
    pub source_height: u32,
    pub start: f64,
    pub end: f64,
    pub rect: TrackingRect,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackingResult {
    pub keyframes: Vec<MaskKeyframe>,
    pub lost_at: Option<f64>,
}

/// Frames and keyframes use original recording coordinates, before crop or zoom.
pub fn track(
    mut input: TrackingInput,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(f64) + Send + 'static,
) -> Result<TrackingResult, String> {
    validate(&input)?;
    input.rect.x = input.rect.x.min(1.0 - input.rect.width);
    input.rect.y = input.rect.y.min(1.0 - input.rect.height);
    cancelled(&cancel)?;
    let info = probe(&input, cancel.clone())?;
    if info.width != input.source_width || info.height != input.source_height {
        return Err("The recording dimensions changed. Reload the project before tracking.".into());
    }
    if input.start < info.start || input.end > info.end + 0.000_001 {
        return Err("The tracking range extends outside the available video.".into());
    }
    let (width, height) = dimensions(info.width, info.height);
    let frames = frame_count((input.end - input.start).min(MAX_MASK_TRACKING_SECONDS));
    let mut command = decode_command(&input, width, height, frames);
    let reader_cancel = cancel.clone();
    let output = run(
        &mut command,
        cancel,
        Duration::from_secs(900),
        Duration::from_secs(15),
        |count| progress((count as f64 / frames as f64).min(0.99)),
        move |pipe, count| scan(pipe, input, width, height, frames, reader_cancel, count),
    )?;
    progress(1.0);
    Ok(output)
}

fn validate(input: &TrackingInput) -> Result<(), String> {
    let r = input.rect;
    if !input.start.is_finite()
        || !input.end.is_finite()
        || input.start < 0.0
        || input.end - input.start < 0.01
        || input.end - input.start > MAX_MASK_TRACKING_SECONDS + 1e-9
        || input.end > screenrecord_core::MAX_RECORDING_RECOVERY_SECONDS
    {
        return Err("Track a source range between 0.01 and 300 seconds.".into());
    }
    if [r.x, r.y, r.width, r.height]
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0 || *v > 1.0)
        || r.width < 0.001
        || r.height < 0.001
        || r.x + r.width > 1.0 + 1e-9
        || r.y + r.height > 1.0 + 1e-9
    {
        return Err("The tracking rectangle must be inside the original video.".into());
    }
    if input.source_width < 2
        || input.source_height < 2
        || input.source_width > 8192
        || input.source_height > 8192
        || u64::from(input.source_width) * u64::from(input.source_height) > 33_554_432
    {
        return Err("The source dimensions exceed the supported tracking limit.".into());
    }
    if !input.source_path.is_file() || !input.ffmpeg_path.is_file() {
        return Err("The local recording or bundled media runtime is unavailable.".into());
    }
    Ok(())
}

fn dimensions(width: u32, height: u32) -> (usize, usize) {
    let scale = (640.0 / f64::from(width.max(height))).min(1.0);
    (
        (f64::from(width) * scale).round().max(2.0) as usize,
        (f64::from(height) * scale).round().max(2.0) as usize,
    )
}

fn frame_count(duration: f64) -> usize {
    let count = (duration * FPS).floor() as usize;
    count + usize::from(count as f64 / FPS < duration)
}

fn decode_command(input: &TrackingInput, width: usize, height: usize, frames: usize) -> Command {
    let mut command = process::command(&input.ffmpeg_path);
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-threads",
        "2",
        "-filter_threads",
        "1",
        "-protocol_whitelist",
        "file,pipe",
        "-copyts",
        "-ss",
    ]);
    command
        .arg(input.start.floor().to_string())
        .args(["-noaccurate_seek", "-i"]);
    command
        .arg(&input.source_path)
        .args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf"]);
    // Ceil frame timestamps hold the source frame active at each requested time.
    command.arg(format!(
        "trim=end={:.17},scale={width}:{height}:flags=area,format=gray,settb=AVTB,setpts=PTS-{:.17}/TB,fps=10:start_time=0:round=up,trim=end_frame={frames}",
        input.end, input.start
    ));
    command.args([
        "-threads", "1", "-f", "rawvideo", "-pix_fmt", "gray", "pipe:1",
    ]);
    command
}

#[derive(Deserialize)]
struct ProbeOutput {
    streams: Vec<ProbeStream>,
}
#[derive(Deserialize)]
struct ProbeStream {
    width: u32,
    height: u32,
    start_time: Option<String>,
    duration: Option<String>,
}
struct VideoInfo {
    width: u32,
    height: u32,
    start: f64,
    end: f64,
}

fn probe(input: &TrackingInput, cancel: Arc<AtomicBool>) -> Result<VideoInfo, String> {
    let mut command = process::command(&input.ffmpeg_path.with_file_name("ffprobe.exe"));
    command.args([
        "-v",
        "error",
        "-protocol_whitelist",
        "file,pipe",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height,start_time,duration",
        "-of",
        "json",
    ]);
    command.arg(&input.source_path);
    let bytes = run(
        &mut command,
        cancel,
        Duration::from_secs(15),
        Duration::from_secs(15),
        |_| {},
        |pipe, _| {
            let mut bytes = Vec::new();
            pipe.take(16_385)
                .read_to_end(&mut bytes)
                .map_err(|_| "Cannot inspect the recording.".to_owned())?;
            if bytes.len() > 16_384 {
                return Err("The video metadata exceeds its size limit.".into());
            }
            Ok((bytes, false))
        },
    )?;
    let data: ProbeOutput =
        serde_json::from_slice(&bytes).map_err(|_| "The recording has invalid video metadata.")?;
    let stream = data
        .streams
        .first()
        .ok_or("The recording has no video stream.")?;
    let start = stream
        .start_time
        .as_deref()
        .unwrap_or("0")
        .parse::<f64>()
        .map_err(|_| "The recording has no valid start time.")?;
    let duration = stream
        .duration
        .as_deref()
        .ok_or("The recording has no video duration.")?
        .parse::<f64>()
        .map_err(|_| "The recording has no valid duration.")?;
    if !start.is_finite() || !duration.is_finite() || start < 0.0 || duration <= 0.0 {
        return Err("The recording has invalid timing metadata.".into());
    }
    Ok(VideoInfo {
        width: stream.width,
        height: stream.height,
        start,
        end: start + duration,
    })
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Mask tracking cancelled.".into())
    } else {
        Ok(())
    }
}

fn run<T: Send + 'static>(
    command: &mut Command,
    cancel: Arc<AtomicBool>,
    deadline: Duration,
    stall: Duration,
    progress: impl Fn(u64),
    read: impl FnOnce(ChildStdout, Arc<AtomicU64>) -> Result<(T, bool), String> + Send + 'static,
) -> Result<T, String> {
    cancelled(&cancel)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Cannot start the local mask tracking process.".to_owned())?;
    let job = match process::ChildJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = cleanup::run(
                Path::new("mask-tracking"),
                Duration::from_secs(2),
                move || child.wait().map(|_| ()).map_err(|e| e.to_string()),
            );
            return Err(error);
        }
    };
    let pipe = child.stdout.take().expect("Tracking stdout is piped.");
    let count = Arc::new(AtomicU64::new(0));
    let reader_count = count.clone();
    let (send, receive) = mpsc::sync_channel(1);
    let reader = match thread::Builder::new()
        .name("mask-tracking-reader".into())
        .spawn(move || {
            let _ = send.send(read(pipe, reader_count));
        }) {
        Ok(reader) => reader,
        Err(_) => {
            let _ = job.terminate();
            let _ = child.kill();
            let _ = cleanup::run(
                Path::new("mask-tracking"),
                Duration::from_secs(2),
                move || child.wait().map(|_| ()).map_err(|e| e.to_string()),
            );
            return Err("Cannot start the mask tracking reader.".into());
        }
    };
    let started = Instant::now();
    let mut last_progress = Instant::now();
    let mut reported = Instant::now();
    let mut last = 0;
    let mut value = None;
    progress(0);
    let outcome = loop {
        if let Err(error) = cancelled(&cancel) {
            break Err(error);
        }
        if started.elapsed() >= deadline {
            break Err("Mask tracking exceeded its processing deadline.".into());
        }
        let current = count.load(Ordering::Relaxed);
        if current != last {
            last = current;
            last_progress = Instant::now();
        }
        if last_progress.elapsed() >= stall {
            break Err("The tracking decoder stopped making progress.".into());
        }
        if reported.elapsed() >= Duration::from_millis(100) {
            progress(current);
            reported = Instant::now();
        }
        if value.is_none() {
            match receive.recv_timeout(Duration::from_millis(25)) {
                Ok(Ok((output, true))) => break Ok(output),
                Ok(Ok((output, false))) => value = Some(output),
                Ok(Err(error)) => break Err(error),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err("The tracking reader stopped unexpectedly.".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        } else {
            thread::sleep(Duration::from_millis(5));
        }
        match child.try_wait() {
            Ok(Some(status)) if value.is_some() => {
                break if status.success() {
                    Ok(value.take().unwrap())
                } else {
                    Err("The recording could not be decoded completely.".into())
                };
            }
            Err(_) => break Err("Cannot inspect the tracking process.".into()),
            _ => {}
        }
    };
    let _ = job.terminate();
    let _ = child.kill();
    cleanup::run(
        Path::new("mask-tracking"),
        Duration::from_secs(2),
        move || {
            child
                .wait()
                .map_err(|_| "Cannot release the tracking process.".to_owned())?;
            reader
                .join()
                .map_err(|_| "The tracking reader stopped unexpectedly.".to_owned())
        },
    )?;
    if outcome.is_ok() {
        progress(count.load(Ordering::Relaxed));
    }
    outcome
}

fn scan(
    pipe: impl Read,
    input: TrackingInput,
    width: usize,
    height: usize,
    frames: usize,
    cancel: Arc<AtomicBool>,
    count: Arc<AtomicU64>,
) -> Result<(TrackingResult, bool), String> {
    let mut pipe = BufReader::with_capacity(65_536, pipe);
    let mut frame = vec![0; width * height];
    let mut tracker = None;
    let mut keyframes = Vec::with_capacity(frames);
    for index in 0..frames {
        cancelled(&cancel)?;
        pipe.read_exact(&mut frame)
            .map_err(|_| "The recording ended before the tracking range was decoded.".to_owned())?;
        let time = input.start + index as f64 / FPS;
        let detection = if index == 0 {
            tracker = matcher::Tracker::new(&frame, width, height, input.rect);
            tracker
                .as_mut()
                .and_then(|tracker| tracker.locate(&frame, &cancel))
        } else {
            tracker
                .as_mut()
                .and_then(|tracker| tracker.locate(&frame, &cancel))
        };
        cancelled(&cancel)?;
        count.store(index as u64 + 1, Ordering::Relaxed);
        let Some((x, y, confidence)) = detection else {
            keyframes.push(MaskKeyframe {
                time,
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                confidence: 0.0,
                kind: "lost".into(),
            });
            return Ok((
                TrackingResult {
                    keyframes,
                    lost_at: Some(time),
                },
                true,
            ));
        };
        keyframes.push(MaskKeyframe {
            time,
            x,
            y,
            width: input.rect.width,
            height: input.rect.height,
            confidence,
            kind: if index == 0 { "manual" } else { "auto" }.into(),
        });
    }
    let mut extra = [0];
    if pipe
        .read(&mut extra)
        .map_err(|_| "Cannot finish decoding the recording.".to_owned())?
        != 0
    {
        return Err("The tracking decoder returned unexpected extra frames.".into());
    }
    Ok((
        TrackingResult {
            keyframes,
            lost_at: None,
        },
        false,
    ))
}

#[cfg(test)]
mod tests;
