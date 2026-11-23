use crate::{audio, probe, process, timeline};
use screenrecord_core::CameraInterval;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone, Debug)]
pub struct RecordingPart {
    pub screen_path: PathBuf,
    pub microphone_path: Option<PathBuf>,
    pub system_audio_path: Option<PathBuf>,
    pub camera_path: Option<PathBuf>,
    pub camera_offset_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration: f64,
}

#[derive(Clone, Debug)]
pub struct AssembledRecording {
    pub screen_path: PathBuf,
    pub microphone_path: Option<PathBuf>,
    pub system_audio_path: Option<PathBuf>,
    pub camera_path: Option<PathBuf>,
    pub camera_offset_seconds: f64,
    pub camera_intervals: Option<Vec<CameraInterval>>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration: f64,
    pub part_offsets: Vec<f64>,
}

struct Camera {
    path: PathBuf,
    width: u32,
    height: u32,
    start: f64,
    end: f64,
}
struct Audio {
    path: PathBuf,
    rate: u32,
    mono: bool,
}
struct Part {
    source: RecordingPart,
    screen_stream: probe::MediaStream,
    offset: f64,
    first_frame: u64,
    frames: u64,
    first_sample: u64,
    samples: u64,
    microphone: Option<Audio>,
    system: Option<Audio>,
    camera: Option<Camera>,
}

struct VideoTarget {
    camera: bool,
    geometry: (u32, u32),
}

pub fn assemble_recording(
    parts: &[RecordingPart],
    project_directory: &Path,
    ffmpeg: &Path,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(f64) + Send + 'static,
) -> Result<AssembledRecording, String> {
    reject_link(project_directory)?;
    let directory = project_directory
        .canonicalize()
        .map_err(|error| format!("Cannot open the recording project: {error}"))?;
    let parts = validate_parts(parts, &directory, ffmpeg, &cancel)?;
    let first = &parts[0].source;
    let duration = parts
        .last()
        .map(|part| part.offset + part.source.duration)
        .unwrap();
    let temporary = tempfile::Builder::new()
        .prefix(".screenrecord-assembly-")
        .tempdir_in(&directory)
        .map_err(|error| format!("Cannot prepare recording assembly: {error}"))?;
    let callback: process::Progress =
        Arc::new(Mutex::new(Box::new(move |value| progress(value.progress))));
    let camera_intervals = availability(&parts);
    let camera_geometry = (!camera_intervals.is_empty()).then(|| {
        parts
            .iter()
            .find_map(|part| {
                part.camera
                    .as_ref()
                    .map(|camera| (camera.width, camera.height))
            })
            .unwrap()
    });
    if !copy_screens(&parts, ffmpeg, temporary.path(), &cancel, &callback)? {
        render_video(
            &parts,
            VideoTarget {
                camera: false,
                geometry: (first.width, first.height),
            },
            ffmpeg,
            temporary.path(),
            &cancel,
            &callback,
            0.0..0.35,
        )?;
    }
    if let Some(geometry) = camera_geometry {
        render_video(
            &parts,
            VideoTarget {
                camera: true,
                geometry,
            },
            ffmpeg,
            temporary.path(),
            &cancel,
            &callback,
            0.35..0.65,
        )?;
    }
    let microphone = parts.iter().any(|part| part.microphone.is_some());
    let system = parts.iter().any(|part| part.system.is_some());
    if microphone {
        render_audio(
            &parts,
            false,
            ffmpeg,
            temporary.path(),
            &cancel,
            &callback,
            0.65..0.8,
        )?;
    }
    if system {
        render_audio(
            &parts,
            true,
            ffmpeg,
            temporary.path(),
            &cancel,
            &callback,
            0.8..0.95,
        )?;
    }
    let frames = timeline::unit_at(duration, first.fps);
    for (name, geometry) in [
        Some(("screen.mp4", (first.width, first.height))),
        camera_geometry.map(|geometry| ("camera.mp4", geometry)),
    ]
    .into_iter()
    .flatten()
    {
        let info = probe::probe_with_cancel(ffmpeg, &temporary.path().join(name), &cancel)?;
        let video = info
            .video()
            .ok_or("The assembled recording has no video stream.")?;
        if video.width != Some(geometry.0)
            || video.height != Some(geometry.1)
            || video
                .nb_frames
                .as_deref()
                .and_then(|value| value.parse::<u64>().ok())
                != Some(frames)
        {
            return Err("The assembled recording did not preserve its frame grid.".into());
        }
    }
    for name in [
        Some("screen.mp4"),
        camera_geometry.map(|_| "camera.mp4"),
        microphone.then_some("microphone.wav"),
        system.then_some("system.wav"),
    ]
    .into_iter()
    .flatten()
    {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(temporary.path().join(name))
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("Cannot flush assembled media: {error}"))?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Recording assembly cancelled.".into());
    }
    let suffix = temporary
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(".screenrecord-assembly-"))
        .ok_or("Invalid assembly staging name.")?;
    let published = directory.join(format!("assembled-{suffix}"));
    publish_directory(temporary.path(), &published)?;
    process::emit(&callback, "running", 1.0, "recording assembly", None);
    Ok(AssembledRecording {
        screen_path: published.join("screen.mp4"),
        microphone_path: microphone.then(|| published.join("microphone.wav")),
        system_audio_path: system.then(|| published.join("system.wav")),
        camera_path: camera_geometry.map(|_| published.join("camera.mp4")),
        camera_offset_seconds: 0.0,
        camera_intervals: Some(camera_intervals),
        width: first.width,
        height: first.height,
        fps: first.fps,
        duration,
        part_offsets: parts.iter().map(|part| part.offset).collect(),
    })
}

fn confined(path: &Path, directory: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Recording part paths must be absolute.".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Cannot read recording part {}: {error}", path.display()))?;
    if !canonical.starts_with(directory) || !canonical.is_file() {
        return Err("Recording part media must remain inside its project folder.".into());
    }
    let mut current = path.to_path_buf();
    loop {
        reject_link(&current)?;
        if current.canonicalize().map_err(|error| error.to_string())? == directory {
            break;
        }
        if !current.pop() {
            return Err("Cannot resolve the recording part's project parent.".into());
        }
    }
    Ok(canonical)
}

fn reject_link(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if metadata.file_type().is_symlink() || reparse {
        return Err("Recording parts cannot use symbolic links or reparse points.".into());
    }
    Ok(())
}

fn audio_info(
    path: &Option<PathBuf>,
    directory: &Path,
    ffmpeg: &Path,
    cancel: &AtomicBool,
) -> Result<Option<Audio>, String> {
    path.as_ref()
        .map(|path| {
            let path = confined(path, directory)?;
            let info = probe::probe_with_cancel(ffmpeg, &path, cancel)?;
            let stream = info
                .streams
                .iter()
                .find(|stream| stream.codec_type.as_deref() == Some("audio"))
                .ok_or("Recording part audio has no audio stream.")?;
            let rate = stream
                .sample_rate
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|rate| *rate > 0)
                .ok_or("Cannot read the recording part sample rate.")?;
            Ok(Audio {
                path,
                rate,
                mono: stream.channels == Some(1),
            })
        })
        .transpose()
}

fn validate_parts(
    inputs: &[RecordingPart],
    directory: &Path,
    ffmpeg: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<Part>, String> {
    if inputs.is_empty() || inputs.len() > 512 {
        return Err("A recording session requires between 1 and 512 parts.".into());
    }
    let first = &inputs[0];
    if ![30, 60].contains(&first.fps)
        || first.width < 2
        || first.height < 2
        || first.width > 16384
        || first.height > 16384
        || !first.width.is_multiple_of(2)
        || !first.height.is_multiple_of(2)
    {
        return Err("Recording part geometry or frame rate is unsupported.".into());
    }
    let mut offset = 0.0;
    let mut parts = Vec::with_capacity(inputs.len());
    for source in inputs {
        if cancel.load(Ordering::Relaxed) {
            return Err("Recording assembly cancelled.".into());
        }
        if (source.width, source.height, source.fps) != (first.width, first.height, first.fps) {
            return Err("The screen dimensions or frame rate changed while paused. Restore the original source size before resuming.".into());
        }
        if !source.duration.is_finite()
            || source.duration <= 0.0
            || offset + source.duration > screenrecord_core::MAX_RECORDING_RECOVERY_SECONDS
            || !source.camera_offset_seconds.is_finite()
            || !(0.0..=3600.0).contains(&source.camera_offset_seconds)
        {
            return Err("Recording part timing is invalid or exceeds eight hours.".into());
        }
        let mut source = source.clone();
        source.screen_path = confined(&source.screen_path, directory)?;
        let screen = probe::probe_with_cancel(ffmpeg, &source.screen_path, cancel)?;
        let video = screen
            .video()
            .ok_or("A recording part has no screen video.")?;
        if video.width != Some(source.width)
            || video.height != Some(source.height)
            || screen
                .duration()
                .is_none_or(|duration| duration + 0.15 < source.duration)
        {
            return Err(
                "A recording part's screen media does not match its geometry or duration.".into(),
            );
        }
        let microphone = audio_info(&source.microphone_path, directory, ffmpeg, cancel)?;
        let system = audio_info(&source.system_audio_path, directory, ffmpeg, cancel)?;
        let camera = source
            .camera_path
            .as_ref()
            .map(|path| {
                let path = confined(path, directory)?;
                let info = probe::probe_with_cancel(ffmpeg, &path, cancel)?;
                let video = info.video().ok_or("A webcam part has no video stream.")?;
                let start = video
                    .start_time
                    .as_deref()
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite())
                    .unwrap_or(0.0);
                let end = video
                    .duration
                    .as_deref()
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite())
                    .map(|duration| start + duration)
                    .or_else(|| info.duration())
                    .ok_or("Cannot read webcam part duration.")?;
                Ok::<_, String>(Camera {
                    path,
                    width: video
                        .width
                        .filter(|v| *v >= 2)
                        .ok_or("Cannot read webcam part width.")?,
                    height: video
                        .height
                        .filter(|v| *v >= 2)
                        .ok_or("Cannot read webcam part height.")?,
                    start,
                    end,
                })
            })
            .transpose()?;
        let first_frame = timeline::unit_at(offset, source.fps);
        let first_sample = timeline::unit_at(offset, 48000);
        let frames = timeline::unit_at(offset + source.duration, source.fps) - first_frame;
        let samples = timeline::unit_at(offset + source.duration, 48000) - first_sample;
        let duration = source.duration;
        parts.push(Part {
            source,
            screen_stream: video.clone(),
            offset,
            first_frame,
            frames,
            first_sample,
            samples,
            microphone,
            system,
            camera,
        });
        offset += duration;
    }
    if offset < 0.1 {
        return Err("Record at least 100 milliseconds before stopping.".into());
    }
    Ok(parts)
}

fn availability(parts: &[Part]) -> Vec<CameraInterval> {
    let mut intervals: Vec<CameraInterval> = Vec::new();
    for part in parts {
        let Some(camera) = &part.camera else { continue };
        let start = (camera.start - part.source.camera_offset_seconds).max(0.0);
        let end = (camera.end - part.source.camera_offset_seconds).min(part.source.duration);
        if end <= start {
            continue;
        }
        let start = part.offset + start;
        let end = part.offset + end;
        if let Some(previous) = intervals.last_mut() {
            if previous.end == start {
                previous.end = end;
                continue;
            }
        }
        intervals.push(CameraInterval { start, end });
    }
    intervals
}

fn command(ffmpeg: &Path, directory: &Path) -> Command {
    let mut command = Command::new(ffmpeg);
    command.current_dir(directory).args([
        "-hide_banner",
        "-nostdin",
        "-v",
        "warning",
        "-y",
        "-filter_complex_threads",
        "2",
    ]);
    command
}

fn copy_screens(
    parts: &[Part],
    ffmpeg: &Path,
    directory: &Path,
    cancel: &Arc<AtomicBool>,
    callback: &process::Progress,
) -> Result<bool, String> {
    let first = &parts[0].screen_stream;
    if first.codec_name.as_deref() != Some("h264") || first.extradata_hash.is_none() {
        return Ok(false);
    }
    for part in parts {
        let stream = &part.screen_stream;
        if stream.codec_name != first.codec_name
            || stream.extradata_hash != first.extradata_hash
            || stream.profile != first.profile
            || stream.level != first.level
            || stream.pix_fmt != first.pix_fmt
            || stream.time_base != first.time_base
            || stream
                .nb_frames
                .as_deref()
                .and_then(|value| value.parse::<u64>().ok())
                != Some(part.frames)
            || stream
                .duration
                .as_deref()
                .and_then(|value| value.parse::<f64>().ok())
                .is_none_or(|duration| (duration - part.source.duration).abs() > 1e-7)
            || (part.source.duration - part.frames as f64 / part.source.fps as f64).abs() > 1e-9
            || (part.first_frame as f64 / part.source.fps as f64 - part.offset).abs() > 1e-9
        {
            return Ok(false);
        }
        if !packet_grid_matches(part, ffmpeg, cancel)? {
            return Ok(false);
        }
    }
    let mut items = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let name = format!("screen-copy-{index:05}.mp4");
        if fs::hard_link(&part.source.screen_path, directory.join(&name)).is_err() {
            for (name, _, _) in &items {
                fs::remove_file(directory.join(name)).map_err(|error| error.to_string())?;
            }
            return Ok(false);
        }
        items.push((name, part.first_frame, part.frames));
    }
    concat_list(directory, "screen-copy.txt", &items, parts[0].source.fps)?;
    let mut mux = command(ffmpeg, directory);
    mux.args([
        "-f",
        "concat",
        "-safe",
        "1",
        "-protocol_whitelist",
        "file,pipe",
        "-i",
        "screen-copy.txt",
        "-map",
        "0:v:0",
        "-c:v",
        "copy",
        "-an",
        "-movflags",
        "+faststart",
        "-progress",
        "pipe:1",
        "-f",
        "mp4",
        "screen.mp4",
    ]);
    let duration = parts
        .last()
        .map(|part| part.offset + part.source.duration)
        .unwrap();
    process::encode(
        &mut mux,
        cancel,
        callback,
        duration,
        "recording assembly",
        0.0..0.35,
    )?;
    for (name, _, _) in items {
        fs::remove_file(directory.join(name)).map_err(|error| error.to_string())?;
    }
    fs::remove_file(directory.join("screen-copy.txt")).map_err(|error| error.to_string())?;
    Ok(true)
}

fn packet_grid_matches(part: &Part, ffmpeg: &Path, cancel: &AtomicBool) -> Result<bool, String> {
    let Some((numerator, denominator)) = part
        .screen_stream
        .time_base
        .as_deref()
        .and_then(|value| value.split_once('/'))
        .and_then(|(n, d)| Some((n.parse::<u64>().ok()?, d.parse::<u64>().ok()?)))
    else {
        return Ok(false);
    };
    let Some(divisor) = numerator
        .checked_mul(part.source.fps as u64)
        .filter(|value| *value > 0)
    else {
        return Ok(false);
    };
    if denominator == 0 || !denominator.is_multiple_of(divisor) {
        return Ok(false);
    }
    let step = denominator / divisor;
    let mut command = Command::new(probe::ffprobe_path(ffmpeg));
    command
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts,dts,duration,flags",
            "-of",
            "compact=p=0:nk=0",
        ])
        .arg(process::media_path(&part.source.screen_path));
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    let result = process::stream_audio(
        &mut command,
        cancel,
        &callback,
        part.frames,
        "screen compatibility",
        0.0..1.0,
        |reader, progress| {
            let mut count = 0_u64;
            for line in BufReader::new(reader).lines() {
                let line = line.map_err(|error| error.to_string())?;
                if line.is_empty() {
                    continue;
                }
                let fields: std::collections::HashMap<_, _> = line
                    .split('|')
                    .filter_map(|field| field.split_once('='))
                    .collect();
                let timestamp = count
                    .checked_mul(step)
                    .ok_or("Screen timestamp overflow.")?;
                let value = |key| fields.get(key).and_then(|value| value.parse::<u64>().ok());
                if value("pts") != Some(timestamp)
                    || value("dts") != Some(timestamp)
                    || value("duration") != Some(step)
                    || count == 0 && !fields.get("flags").is_some_and(|flags| flags.contains('K'))
                    || count >= part.frames
                {
                    return Err("Screen packets need frame-grid normalization.".into());
                }
                count += 1;
                progress.store(count, Ordering::Relaxed);
            }
            if count != part.frames {
                return Err("Screen packet count does not match the frame grid.".into());
            }
            Ok(())
        },
    );
    if cancel.load(Ordering::Relaxed) {
        return Err("Recording assembly cancelled.".into());
    }
    Ok(result.is_ok())
}

fn render_video(
    parts: &[Part],
    target: VideoTarget,
    ffmpeg: &Path,
    directory: &Path,
    cancel: &Arc<AtomicBool>,
    callback: &process::Progress,
    progress: std::ops::Range<f64>,
) -> Result<(), String> {
    let VideoTarget { camera, geometry } = target;
    let spec = process::VideoSpec {
        width: geometry.0,
        height: geometry.1,
        fps: parts[0].source.fps,
    };
    let encoder = process::choose_encoder(ffmpeg, spec, cancel)?;
    let render = |encoder| -> Result<(), process::EncodeError> {
        for (index, part) in parts.iter().enumerate().filter(|(_, part)| part.frames > 0) {
            let start = progress.start
                + (index as f64 / parts.len() as f64) * (progress.end - progress.start) * 0.9;
            let end = progress.start
                + ((index + 1) as f64 / parts.len() as f64) * (progress.end - progress.start) * 0.9;
            process::encode(
                &mut video_command(part, index, camera, geometry, ffmpeg, directory, encoder),
                cancel,
                callback,
                part.frames as f64 / spec.fps as f64,
                "recording assembly",
                start..end,
            )?;
        }
        Ok(())
    };
    if let Err(error) = render(encoder) {
        if error.retry_with_software && !encoder.is_software() && !cancel.load(Ordering::Relaxed) {
            render(process::software_encoder(ffmpeg, spec, cancel)?)?;
        } else {
            return Err(error.into());
        }
    }
    let kind = if camera { "camera" } else { "screen" };
    let items = parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.frames > 0)
        .map(|(index, part)| {
            (
                format!("{kind}-{index:05}.mp4"),
                part.first_frame,
                part.frames,
            )
        })
        .collect::<Vec<_>>();
    concat_list(directory, &format!("{kind}.txt"), &items, spec.fps)?;
    let mut mux = command(ffmpeg, directory);
    mux.args([
        "-f",
        "concat",
        "-safe",
        "1",
        "-protocol_whitelist",
        "file,pipe",
        "-i",
        &format!("{kind}.txt"),
        "-map",
        "0:v:0",
        "-c:v",
        "copy",
        "-an",
        "-movflags",
        "+faststart",
        "-progress",
        "pipe:1",
        "-f",
        "mp4",
    ])
    .arg(format!("{kind}.mp4"));
    let duration = parts
        .last()
        .map(|part| part.offset + part.source.duration)
        .unwrap();
    process::encode(
        &mut mux,
        cancel,
        callback,
        duration,
        "recording assembly",
        progress.start + (progress.end - progress.start) * 0.9..progress.end,
    )?;
    for (name, _, _) in items {
        fs::remove_file(directory.join(name)).map_err(|error| error.to_string())?;
    }
    fs::remove_file(directory.join(format!("{kind}.txt"))).map_err(|error| error.to_string())?;
    Ok(())
}

fn video_command(
    part: &Part,
    index: usize,
    camera: bool,
    geometry: (u32, u32),
    ffmpeg: &Path,
    directory: &Path,
    encoder: process::Encoder,
) -> Command {
    let mut result = command(ffmpeg, directory);
    let fps = part.source.fps;
    let shift = grid_shift(part.first_frame, part.offset, fps);
    let duration = part.frames as f64 / fps as f64;
    let kind = if camera { "camera" } else { "screen" };
    result.arg("-copyts");
    let filter = if camera {
        if let Some(source) = &part.camera {
            result
                .args(["-protocol_whitelist", "file,pipe", "-i"])
                .arg(process::media_path(&source.path));
            let begin = shift + part.source.camera_offset_seconds;
            let end = part.source.duration + part.source.camera_offset_seconds;
            let visible_start = (source.start - begin).max(0.0);
            let visible_end = (source.end - begin).min(duration);
            format!("color=c=black:s={}x{}:r={fps}:d={duration:.12}[base];[0:v]trim=end={end:.12},settb=AVTB,setpts=PTS-{begin:.12}/TB,fps={fps}:start_time=0:round=up,scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1[cam];[base][cam]overlay=eof_action=pass:repeatlast=0:enable='gte(t,{visible_start:.12})*lt(t,{visible_end:.12})',format=yuv420p[outv]",geometry.0,geometry.1,geometry.0,geometry.1,geometry.0,geometry.1)
        } else {
            format!(
                "color=c=black:s={}x{}:r={fps}:d={duration:.12},format=yuv420p[outv]",
                geometry.0, geometry.1
            )
        }
    } else {
        result
            .args(["-protocol_whitelist", "file,pipe", "-i"])
            .arg(process::media_path(&part.source.screen_path));
        format!("[0:v]trim=end={:.12},tpad=stop_mode=clone:stop_duration={duration:.12},settb=AVTB,setpts=PTS-{shift:.12}/TB,fps={fps}:start_time=0:round=up,trim=end_frame={},setsar=1,format=yuv420p[outv]",part.source.duration,part.frames)
    };
    result
        .args(["-filter_complex", &filter, "-map", "[outv]", "-an"])
        .args(encoder.args())
        .args([
            "-r",
            &fps.to_string(),
            "-frames:v",
            &part.frames.to_string(),
            "-pix_fmt",
            "yuv420p",
            "-progress",
            "pipe:1",
            "-f",
            "mp4",
        ])
        .arg(format!("{kind}-{index:05}.mp4"));
    result
}

fn render_audio(
    parts: &[Part],
    system: bool,
    ffmpeg: &Path,
    directory: &Path,
    cancel: &Arc<AtomicBool>,
    callback: &process::Progress,
    progress: std::ops::Range<f64>,
) -> Result<(), String> {
    let kind = if system { "system" } else { "microphone" };
    let mut items = Vec::new();
    for (index, part) in parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.samples > 0)
    {
        let mut command = command(ffmpeg, directory);
        let audio = if system {
            &part.system
        } else {
            &part.microphone
        };
        let filter = if let Some(audio) = audio {
            command
                .arg("-copyts")
                .args(["-protocol_whitelist", "file,pipe", "-i"])
                .arg(process::media_path(&audio.path));
            let shift = grid_shift(part.first_sample, part.offset, 48000);
            let first = audio::native_sample_at(shift, audio.rate);
            let end = audio::native_sample_at(part.source.duration, audio.rate);
            let pan = if audio.mono {
                "pan=stereo|c0=c0|c1=c0,"
            } else {
                ""
            };
            format!("[0:a]asettb=1/{},atrim=start_pts={first}:end_pts={end},asetpts=PTS-STARTPTS,aresample=48000:async=1:first_pts=0,{pan}aformat=sample_fmts=fltp:channel_layouts=stereo,apad,atrim=end_sample={}[outa]",audio.rate,part.samples)
        } else {
            format!(
                "anullsrc=r=48000:cl=stereo,atrim=end_sample={}[outa]",
                part.samples
            )
        };
        let name = format!("{kind}-{index:05}.wav");
        command
            .args([
                "-filter_complex",
                &filter,
                "-map",
                "[outa]",
                "-c:a",
                "pcm_f32le",
                "-ar",
                "48000",
                "-ac",
                "2",
                "-rf64",
                "auto",
                "-progress",
                "pipe:1",
                "-f",
                "wav",
            ])
            .arg(&name);
        let start = progress.start
            + (index as f64 / parts.len() as f64) * (progress.end - progress.start) * 0.8;
        let end = progress.start
            + ((index + 1) as f64 / parts.len() as f64) * (progress.end - progress.start) * 0.8;
        process::encode(
            &mut command,
            cancel,
            callback,
            part.samples as f64 / 48000.0,
            "recording assembly",
            start..end,
        )?;
        items.push((name, part.first_sample, part.samples));
    }
    concat_list(directory, &format!("{kind}.txt"), &items, 48000)?;
    let mut mux = command(ffmpeg, directory);
    mux.args([
        "-f",
        "concat",
        "-safe",
        "1",
        "-protocol_whitelist",
        "file,pipe",
        "-i",
        &format!("{kind}.txt"),
        "-map",
        "0:a:0",
        "-af",
        "asetpts=N/SR/TB",
        "-c:a",
        "pcm_f32le",
        "-ar",
        "48000",
        "-ac",
        "2",
        "-rf64",
        "auto",
        "-progress",
        "pipe:1",
        "-f",
        "wav",
    ])
    .arg(format!("{kind}.wav"));
    let duration = parts
        .last()
        .map(|part| part.offset + part.source.duration)
        .unwrap();
    process::encode(
        &mut mux,
        cancel,
        callback,
        duration,
        "recording assembly",
        progress.start + (progress.end - progress.start) * 0.8..progress.end,
    )?;
    let info = probe::probe_with_cancel(ffmpeg, &directory.join(format!("{kind}.wav")), cancel)?;
    let samples = timeline::unit_at(duration, 48000);
    if !info.has_audio()
        || info
            .duration()
            .is_none_or(|duration| (duration - samples as f64 / 48000.0).abs() > 0.000_002)
    {
        return Err("Assembled audio did not preserve its sample grid.".into());
    }
    for (name, _, _) in items {
        fs::remove_file(directory.join(name)).map_err(|error| error.to_string())?;
    }
    fs::remove_file(directory.join(format!("{kind}.txt"))).map_err(|error| error.to_string())?;
    Ok(())
}

fn concat_list(
    directory: &Path,
    name: &str,
    items: &[(String, u64, u64)],
    rate: u32,
) -> Result<(), String> {
    let mut file =
        BufWriter::new(File::create(directory.join(name)).map_err(|error| error.to_string())?);
    for (path, first, count) in items {
        let start = (*first as f64 / rate as f64 * 1e6).round();
        let end = ((*first + *count) as f64 / rate as f64 * 1e6).round();
        writeln!(file, "file '{path}'\nduration {:.9}", (end - start) / 1e6)
            .map_err(|error| error.to_string())?;
    }
    file.flush().map_err(|error| error.to_string())
}

fn grid_shift(first: u64, offset: f64, rate: u32) -> f64 {
    let shift = first as f64 / rate as f64 - offset;
    if shift.abs() <= screenrecord_core::TIME_EPSILON {
        0.0
    } else {
        shift.max(0.0)
    }
}

fn publish_directory(source: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH},
        };
        let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<_> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|error| {
            format!("Cannot publish assembled recording without replacing existing media: {error}")
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (source, destination);
        Err("Recording assembly publication requires Windows.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_part_boundaries_do_not_skip_a_native_sample() {
        let offset = 0.3 + 0.6;
        let shift = grid_shift(timeline::unit_at(offset, 48000), offset, 48000);
        assert_eq!(shift, 0.0);
        assert_eq!(audio::native_sample_at(shift, 48000), 0);
        let fractional = 0.901234;
        let shift = grid_shift(timeline::unit_at(fractional, 48000), fractional, 48000);
        assert!(shift > screenrecord_core::TIME_EPSILON && shift < 1.0 / 48000.0);
    }

    #[cfg(windows)]
    #[test]
    fn publication_never_replaces_an_existing_directory() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("staged");
        let destination = root.path().join("published");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("screen.mp4"), b"new").unwrap();
        fs::write(destination.join("screen.mp4"), b"existing").unwrap();
        assert!(publish_directory(&source, &destination).is_err());
        assert_eq!(fs::read(source.join("screen.mp4")).unwrap(), b"new");
        assert_eq!(
            fs::read(destination.join("screen.mp4")).unwrap(),
            b"existing"
        );
    }

    #[test]
    fn part_paths_must_resolve_inside_the_project() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("audio.wav"), b"outside").unwrap();
        assert!(confined(
            &outside.path().join("audio.wav"),
            &root.path().canonicalize().unwrap()
        )
        .is_err());
        assert!(confined(Path::new("relative.wav"), root.path()).is_err());
    }
}
