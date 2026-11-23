use crate::{probe, process};
use screenrecord_core::{MusicClip, Project};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
};

const RATE: f64 = 48_000.0;
pub(crate) const LIMITER: &str = "alimiter=level=0:limit=0.95:attack=5:release=50:asc=0:latency=1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Bus {
    Microphone,
    System,
    Music,
}

#[derive(Clone, Debug)]
pub(crate) struct AudioTrack {
    bus: Bus,
    path: PathBuf,
    seek: f64,
    duration: f64,
    delay: u64,
    volume: f64,
    fades: String,
    mono: bool,
    timeline_start: f64,
    music: Option<MusicClip>,
    sample_rate: u32,
    source_end: f64,
}

pub(crate) fn prepare(
    project: &Project,
    ffmpeg: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<AudioTrack>, String> {
    let mut tracks = Vec::new();
    let duration = project.edits.trim_end - project.edits.trim_start;
    for (path, volume, bus) in [
        (
            &project.microphone_path,
            project.edits.microphone_volume * project.edits.audio_processing.microphone_gain,
            Bus::Microphone,
        ),
        (
            &project.system_audio_path,
            project.edits.system_volume * project.edits.audio_processing.system_gain,
            Bus::System,
        ),
    ] {
        if let Some(path) = path {
            if volume == 0.0 {
                continue;
            }
            let info = probe::probe_with_cancel(ffmpeg, Path::new(path), cancel)?;
            let stream = info
                .streams
                .iter()
                .find(|stream| stream.codec_type.as_deref() == Some("audio"))
                .ok_or_else(|| format!("The selected recording has no audio stream: {path}"))?;
            tracks.push(AudioTrack {
                bus,
                path: path.into(),
                seek: project.edits.trim_start,
                duration,
                delay: 0,
                volume,
                fades: String::new(),
                mono: stream.channels == Some(1),
                timeline_start: project.edits.trim_start,
                music: None,
                sample_rate: native_rate(stream)?,
                source_end: project.edits.trim_end,
            });
        }
    }
    for clip in &project.edits.music {
        let start = clip.start.max(project.edits.trim_start);
        let end = (clip.start + clip.duration).min(project.edits.trim_end);
        if clip.volume == 0.0 || end <= start {
            continue;
        }
        let info = probe::probe_with_cancel(ffmpeg, Path::new(&clip.path), cancel)?;
        let stream = info
            .streams
            .iter()
            .find(|stream| stream.codec_type.as_deref() == Some("audio"))
            .ok_or_else(|| format!("Music has no audio stream: {}", clip.name))?;
        let available = stream
            .duration
            .as_deref()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .or_else(|| info.duration())
            .ok_or_else(|| format!("Cannot determine music duration: {}", clip.name))?;
        if clip.source_start + clip.duration > available + 0.05 {
            return Err(format!(
                "Music ends before its selected source trim: {}",
                clip.name
            ));
        }
        let skip = start - clip.start;
        tracks.push(AudioTrack {
            bus: Bus::Music,
            path: clip.path.clone().into(),
            seek: clip.source_start + skip,
            duration: end - start,
            delay: samples(start - project.edits.trim_start),
            volume: clip.volume,
            fades: fade_filters(clip, skip),
            mono: stream.channels == Some(1),
            timeline_start: start,
            music: Some(clip.clone()),
            sample_rate: native_rate(stream)?,
            source_end: end + (clip.source_start - clip.start),
        });
    }
    Ok(tracks)
}

fn native_rate(stream: &probe::MediaStream) -> Result<u32, String> {
    stream
        .sample_rate
        .as_deref()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|rate| *rate > 0)
        .ok_or_else(|| "Cannot read the audio source sample rate.".into())
}

pub(crate) fn native_sample_at(seconds: f64, rate: u32) -> u64 {
    let candidate = (seconds * rate as f64).floor().max(0.0) as u64;
    candidate + u64::from((candidate as f64 / rate as f64) < seconds)
}

pub(crate) fn samples(seconds: f64) -> u64 {
    (seconds * RATE).round().max(0.0) as u64
}

fn fade_filters(clip: &MusicClip, skip: f64) -> String {
    let mut filters = String::new();
    if clip.fade_in > skip {
        filters.push_str(&format!(
            ",afade=t=in:ss=0:ns={}:curve=tri:silence={:.12}",
            samples(clip.fade_in - skip).max(1),
            skip / clip.fade_in
        ));
    }
    if clip.fade_out > 0.0 {
        let fade_start = clip.duration - clip.fade_out;
        let elapsed = (skip - fade_start).max(0.0);
        filters.push_str(&format!(
            ",afade=t=out:ss={}:ns={}:curve=tri:unity={:.12}",
            samples((fade_start - skip).max(0.0)),
            samples(clip.fade_out - elapsed).max(1),
            1.0 - elapsed / clip.fade_out
        ));
    }
    filters
}

#[cfg(test)]
pub(crate) fn append_inputs(
    command: &mut Command,
    tracks: &[AudioTrack],
    duration: f64,
) -> Vec<String> {
    append_range_inputs(command, tracks, samples(duration), 1, true)
}

pub(crate) fn slice(tracks: &[AudioTrack], start: f64, end: f64) -> Vec<AudioTrack> {
    tracks
        .iter()
        .filter_map(|track| {
            let overlap_start = start.max(track.timeline_start);
            let overlap_end = end.min(track.timeline_start + track.duration);
            if overlap_end <= overlap_start {
                return None;
            }
            let mut result = track.clone();
            let source_offset = track.seek - track.timeline_start;
            result.seek = overlap_start + source_offset;
            result.source_end = overlap_end + source_offset;
            result.duration = overlap_end - overlap_start;
            result.delay = samples(overlap_start - start);
            result.timeline_start = overlap_start;
            if let Some(clip) = &track.music {
                result.fades = fade_filters(clip, result.seek - clip.source_start);
            }
            Some(result)
        })
        .collect()
}

pub(crate) fn append_range_inputs(
    command: &mut Command,
    tracks: &[AudioTrack],
    sample_count: u64,
    first_input: usize,
    limit: bool,
) -> Vec<String> {
    let mut filters = append_track_inputs(command, tracks, sample_count, first_input);
    if !tracks.is_empty() {
        let labels = (0..tracks.len())
            .map(|index| format!("[audio{index}]"))
            .collect::<String>();
        let limiter = if limit {
            format!(",{LIMITER}")
        } else {
            String::new()
        };
        filters.push(format!(
            "{labels}amix=inputs={}:duration=longest:normalize=0{limiter}[outa]",
            tracks.len()
        ));
    } else {
        filters.push(format!(
            "anullsrc=r=48000:cl=stereo,atrim=end_sample={sample_count}[outa]"
        ));
    }
    filters
}

pub(crate) fn needs_ducking(
    tracks: &[AudioTrack],
    settings: &screenrecord_core::DuckingSettings,
    intervals: &[screenrecord_core::KeptInterval],
) -> bool {
    let present = |bus| {
        tracks.iter().any(|track| {
            track.bus == bus
                && intervals.iter().any(|interval| {
                    track.timeline_start < interval.end
                        && track.timeline_start + track.duration > interval.start
                })
        })
    };
    present(Bus::Microphone)
        && ((settings.enabled && present(Bus::Music))
            || (settings.system_enabled && present(Bus::System)))
}

pub(crate) fn active_envelopes(
    tracks: &[AudioTrack],
    keys: &screenrecord_core::VolumeKeyframes,
    intervals: &[screenrecord_core::KeptInterval],
) -> [bool; 3] {
    [
        (Bus::Microphone, &keys.microphone),
        (Bus::System, &keys.system),
        (Bus::Music, &keys.music),
    ]
    .map(|(bus, keys)| {
        if keys.iter().all(|key| key.volume == 1.0) {
            return false;
        }
        tracks.iter().filter(|track| track.bus == bus).any(|track| {
            intervals.iter().any(|interval| {
                let start = track.timeline_start.max(interval.start);
                let end = (track.timeline_start + track.duration).min(interval.end);
                start < end && crate::envelopes::differs_from_unity(keys, start, end)
            })
        })
    })
}

pub(crate) fn append_bus_inputs(
    command: &mut Command,
    tracks: &[AudioTrack],
    sample_count: u64,
) -> Vec<String> {
    let mut filters = append_track_inputs(command, tracks, sample_count, 0);
    for (bus_index, bus) in [Bus::Microphone, Bus::System, Bus::Music]
        .into_iter()
        .enumerate()
    {
        let labels: Vec<_> = tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| track.bus == bus)
            .map(|(index, _)| format!("[audio{index}]"))
            .collect();
        if labels.is_empty() {
            filters.push(format!(
                "anullsrc=r=48000:cl=stereo,atrim=end_sample={sample_count},asetnsamples=n=1024:p=1[bus{bus_index}]"
            ));
        } else {
            filters.push(format!(
                "{}amix=inputs={}:duration=longest:normalize=0,asetnsamples=n=1024:p=1[bus{bus_index}]",
                labels.join(""),
                labels.len()
            ));
        }
    }
    // Explicit channel mapping keeps the three stereo buses independent of surround layouts.
    filters.push(format!("[bus0][bus1][bus2]join=inputs=3:channel_layout=5.1:map=0.0-FL|0.1-FR|1.0-FC|1.1-LFE|2.0-BL|2.1-BR,atrim=end_sample={sample_count}[outa]"));
    filters
}

fn append_track_inputs(
    command: &mut Command,
    tracks: &[AudioTrack],
    sample_count: u64,
    first_input: usize,
) -> Vec<String> {
    command.arg("-copyts");
    let mut filters = Vec::new();
    for (index, track) in tracks.iter().enumerate() {
        command
            .args([
                "-ss",
                &format!("{:.0}", track.seek.floor()),
                "-noaccurate_seek",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
            ])
            .arg(process::media_path(&track.path));
        let channels = if track.mono {
            "pan=stereo|c0=c0|c1=c0,"
        } else {
            ""
        };
        let native_start = native_sample_at(track.seek, track.sample_rate);
        let native_end = native_sample_at(track.source_end, track.sample_rate);
        filters.push(format!("[{}:a]asettb=1/{},atrim=start_pts={native_start}:end_pts={native_end},asetpts=PTS-STARTPTS,aresample=48000:async=1:first_pts=0,{channels}aformat=sample_fmts=fltp:channel_layouts=stereo,atrim=end_sample={}{},volume={:.9},adelay={}S:all=1,apad,atrim=end_sample={}[audio{index}]",
            index + first_input, track.sample_rate, samples(track.duration), track.fades, track.volume, track.delay, sample_count));
    }
    filters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sample_bounds_distinguish_exact_endpoints_from_real_fractional_samples() {
        assert_eq!(native_sample_at(1.1, 48_000), 52_800);
        let boundary = 0.1_f64;
        assert_eq!(native_sample_at(boundary, 48_000), 4800);
        assert_eq!(
            native_sample_at(f64::from_bits(boundary.to_bits() + 1), 48_000),
            4801
        );
        assert_eq!(
            native_sample_at(f64::from_bits(boundary.to_bits() - 1), 48_000),
            4800
        );
        assert_eq!(native_sample_at(0.201234, 44_100), 8875);
    }
}
