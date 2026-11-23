mod assembly;
mod assets;
mod audio;
mod background;
#[cfg(test)]
mod chunk_tests;
mod clicks;
mod compact;
mod ducking;
mod effects;
mod envelopes;
mod graph;
pub mod math;
mod probe;
mod process;
mod subtitles;
mod timeline;

pub use assembly::{assemble_recording, AssembledRecording, RecordingPart};
pub use math::auto_zoom;
pub use probe::{ffprobe_path, probe_media, MediaFormat, MediaInfo, MediaStream};
pub use subtitles::{subtitle_preview, subtitle_previews};

use screenrecord_core::{ExportOptions, ExportPreset, ExportProgress, Project, WHATSAPP_MAX_BYTES};
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

pub fn export(
    project: &Project,
    options: &ExportOptions,
    ffmpeg_path: &Path,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(ExportProgress) + Send + 'static,
) -> Result<(), String> {
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(progress)));
    process::emit(&callback, "running", 0.0, &options.output_path, None);
    let result = export_inner(
        project,
        options,
        ffmpeg_path,
        &cancel,
        &callback,
        graph::MAX_CHUNK_FRAMES,
    );
    match &result {
        Ok(bytes) => process::emit_with_bytes(
            &callback,
            "completed",
            1.0,
            &options.output_path,
            None,
            Some(*bytes),
        ),
        Err(error) if cancel.load(Ordering::Relaxed) => process::emit(
            &callback,
            "cancelled",
            0.0,
            &options.output_path,
            Some(error.clone()),
        ),
        Err(error) => process::emit(
            &callback,
            "failed",
            0.0,
            &options.output_path,
            Some(error.clone()),
        ),
    }
    result.map(|_| ())
}

struct ExportRun {
    chunk_frames: u64,
    max_bytes: Option<u64>,
    encoder: Option<process::Encoder>,
}

fn export_inner(
    project: &Project,
    options: &ExportOptions,
    ffmpeg: &Path,
    cancel: &Arc<AtomicBool>,
    callback: &process::Progress,
    chunk_frames: u64,
) -> Result<u64, String> {
    export_with_limits(
        project,
        options,
        ffmpeg,
        cancel,
        callback,
        ExportRun {
            chunk_frames,
            max_bytes: (options.preset == ExportPreset::Whatsapp).then_some(WHATSAPP_MAX_BYTES),
            encoder: None,
        },
    )
}

fn export_with_limits(
    project: &Project,
    options: &ExportOptions,
    ffmpeg: &Path,
    cancel: &Arc<AtomicBool>,
    callback: &process::Progress,
    run: ExportRun,
) -> Result<u64, String> {
    project.validate()?;
    screenrecord_core::validate_mask_tracking_review(&project.edits, project.duration)?;
    options.validate()?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Export cancelled.".into());
    }
    if !ffmpeg.is_file() {
        return Err("FFmpeg is missing. Repair the application installation.".into());
    }
    let destination = Path::new(&options.output_path);
    let parent = destination.parent().ok_or("Choose an output directory.")?;
    let temporary = tempfile::Builder::new()
        .prefix(".screenrecord-export-")
        .tempdir_in(parent)
        .map_err(|error| format!("Cannot write to the output folder: {error}"))?;
    let graph = graph::RenderGraph::prepare(project, options, ffmpeg, temporary.path(), cancel)?;
    let chunks = graph.chunks(run.chunk_frames)?;
    let spec = process::VideoSpec {
        width: graph.layout.width,
        height: graph.layout.height,
        fps: options.fps,
    };
    let bitrate = graph.video_bitrate();
    let encoder = match run.encoder {
        Some(encoder) => encoder,
        None => process::choose_encoder_for_bitrate(ffmpeg, spec, cancel, bitrate)?,
    };
    let compact = bitrate.is_some();
    let video_end = if compact {
        0.6
    } else if graph.has_audio() {
        0.8
    } else {
        0.95
    };
    let render_chunks = |encoder, bitrate, start: f64, end: f64| {
        for chunk in &chunks {
            if cancel.load(Ordering::Relaxed) {
                return Err(process::EncodeError::from("Export cancelled."));
            }
            let chunk_start =
                start + chunk.first_frame as f64 / graph.frame_count as f64 * (end - start);
            let chunk_end = start
                + (chunk.first_frame + chunk.frames) as f64 / graph.frame_count as f64
                    * (end - start);
            let mut command = if bitrate == graph.video_bitrate() {
                graph.video_command(project, options, ffmpeg, encoder, *chunk, cancel)?
            } else {
                graph.video_command_with_rate(
                    project,
                    options,
                    ffmpeg,
                    (encoder, bitrate),
                    *chunk,
                    cancel,
                )?
            };
            process::encode(
                &mut command,
                cancel,
                callback,
                chunk.frames as f64 / options.fps as f64,
                &options.output_path,
                chunk_start..chunk_end,
            )?;
        }
        Ok(())
    };
    let mut result = render_chunks(encoder, bitrate, 0.0, video_end);
    if result
        .as_ref()
        .is_err_and(|error| error.retry_with_software)
        && !cancel.load(Ordering::Relaxed)
        && !encoder.is_software()
    {
        let fallback_start = if compact { 0.6 } else { 0.0 };
        let fallback_end = if compact { 0.65 } else { video_end };
        process::emit(
            callback,
            "running",
            fallback_start,
            &options.output_path,
            None,
        );
        let fallback = process::software_encoder_for_bitrate(ffmpeg, spec, cancel, bitrate)?;
        result = render_chunks(fallback, bitrate, fallback_start, fallback_end);
    }
    result?;
    if graph.has_audio() {
        let mut ducker = graph.ducker();
        for chunk in graph.audio_chunks() {
            let (audio_start, audio_width) = if compact { (0.65, 0.1) } else { (0.8, 0.15) };
            let start =
                audio_start + chunk.first_sample as f64 / 48_000.0 / graph.duration * audio_width;
            let end = (audio_start
                + (chunk.first_sample + chunk.samples) as f64 / 48_000.0 / graph.duration
                    * audio_width)
                .min(audio_start + audio_width);
            let mut command = graph.audio_command(ffmpeg, chunk);
            if let Some(ducker) = &mut ducker {
                process::stream_audio(
                    &mut command,
                    cancel,
                    callback,
                    chunk.samples,
                    &options.output_path,
                    start..end,
                    |input, progress| {
                        graph.mix_audio_stream(input, chunk, ducker, cancel, progress)
                    },
                )?;
            } else {
                process::encode(
                    &mut command,
                    cancel,
                    callback,
                    chunk.samples as f64 / 48_000.0,
                    &options.output_path,
                    start..end,
                )?;
            }
        }
    }
    process::encode(
        &mut graph.mux_command(project, options, ffmpeg, &chunks)?,
        cancel,
        callback,
        graph.duration,
        &options.output_path,
        if compact { 0.75..0.8 } else { 0.95..0.995 },
    )?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Export cancelled.".into());
    }
    let encoded = temporary.path().join("encoded.mp4");
    let inspect_encoded = || -> Result<u64, String> {
        let info = probe::probe_with_cancel(ffmpeg, &encoded, cancel)?;
        validate_output(
            &info,
            &graph.layout,
            graph.frame_count,
            options.fps,
            graph.duration,
            graph.has_audio(),
        )?;
        if compact && info.video().and_then(|video| video.pix_fmt.as_deref()) != Some("yuv420p") {
            return Err("The encoder produced an incompatible WhatsApp pixel format. No output was published.".into());
        }
        fs::metadata(&encoded)
            .map(|metadata| metadata.len())
            .map_err(|error| format!("Cannot inspect the encoded file: {error}"))
    };
    let mut output_bytes = inspect_encoded()?;
    if let Some(maximum) = run.max_bytes {
        if output_bytes > maximum {
            let lower = compact::retry_bitrate(
                bitrate.ok_or(compact::SIZE_ERROR)?,
                output_bytes,
                maximum,
                graph.duration,
                graph.has_audio(),
            )?;
            if cancel.load(Ordering::Relaxed) {
                return Err("Export cancelled.".into());
            }
            let retry_encoder = match run.encoder {
                Some(encoder) => encoder,
                None => process::choose_encoder_for_bitrate(ffmpeg, spec, cancel, Some(lower))?,
            };
            let mut retry = render_chunks(retry_encoder, Some(lower), 0.8, 0.95);
            if retry.as_ref().is_err_and(|error| error.retry_with_software)
                && !cancel.load(Ordering::Relaxed)
                && !retry_encoder.is_software()
            {
                let fallback =
                    process::software_encoder_for_bitrate(ffmpeg, spec, cancel, Some(lower))?;
                retry = render_chunks(fallback, Some(lower), 0.95, 0.97);
            }
            retry?;
            process::encode(
                &mut graph.mux_command(project, options, ffmpeg, &chunks)?,
                cancel,
                callback,
                graph.duration,
                &options.output_path,
                0.97..0.995,
            )?;
            if cancel.load(Ordering::Relaxed) {
                return Err("Export cancelled.".into());
            }
            output_bytes = inspect_encoded()?;
            if output_bytes > maximum {
                return Err(compact::SIZE_ERROR.into());
            }
        }
    }
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&encoded)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("Cannot flush the finished video: {error}"))?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Export cancelled.".into());
    }
    // TempPath uses a no-replace rename on Windows, including FAT/exFAT destinations.
    tempfile::TempPath::try_from_path(encoded)
        .map_err(|error| error.to_string())?
        .persist_noclobber(destination)
        .map_err(|error| {
            format!("Cannot publish the finished video without overwriting a file: {error}")
        })?;
    Ok(output_bytes)
}

fn validate_output(
    info: &MediaInfo,
    layout: &math::Layout,
    frames: u64,
    fps: u32,
    duration: f64,
    audio: bool,
) -> Result<(), String> {
    let video = info
        .video()
        .ok_or("The encoder produced no video stream.")?;
    if video.codec_name.as_deref() != Some("h264")
        || video.width != Some(layout.width)
        || video.height != Some(layout.height)
    {
        return Err("The encoded video does not match the requested output format.".into());
    }
    let actual_frames = video
        .nb_frames
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok());
    let rate = video
        .avg_frame_rate
        .as_deref()
        .and_then(|value| value.split_once('/'))
        .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?));
    if actual_frames != Some(frames)
        || rate.is_none_or(|value| !value.is_finite() || (value - fps as f64).abs() > 0.001)
    {
        return Err("The encoded video has missing frames or an unexpected frame rate. No output was published.".into());
    }
    let mut audio_streams = info
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("audio"));
    let audio_stream = audio_streams.next();
    if audio {
        let stream = audio_stream
            .ok_or("The encoded video is missing its audio track. No output was published.")?;
        if audio_streams.next().is_some()
            || stream.codec_name.as_deref() != Some("aac")
            || stream.sample_rate.as_deref() != Some("48000")
            || stream.channels != Some(2)
        {
            return Err("The encoded audio does not match the requested stereo AAC format. No output was published.".into());
        }
        let actual_duration = stream
            .duration
            .as_deref()
            .and_then(|value| value.parse::<f64>().ok());
        let actual_start = stream
            .start_time
            .as_deref()
            .and_then(|value| value.parse::<f64>().ok());
        // AAC can end within one 1024-sample packet of the requested trim.
        let packet_tolerance = 1024.0 / 48_000.0 + 0.000001;
        if actual_start
            .is_none_or(|value| !value.is_finite() || value.abs() > 1.0 / 48_000.0 + 0.000001)
            || actual_duration.is_none_or(|value| {
                !value.is_finite() || (value - duration).abs() > packet_tolerance
            })
        {
            return Err(
                "The encoded audio is incomplete or out of sync. No output was published.".into(),
            );
        }
    } else if audio_stream.is_some() {
        return Err("The encoded video contains unexpected audio. No output was published.".into());
    }
    if info
        .duration()
        .is_none_or(|value| (value - duration).abs() > 1.0 / fps as f64 + 0.001)
    {
        return Err("The encoder produced an incomplete video. No output was published.".into());
    }
    Ok(())
}
