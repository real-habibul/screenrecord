use crate::{cleanup, process};
use serde::Serialize;
use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

mod cache;
pub use cache::{analyze_cached, AnalysisCachePolicy};

#[derive(Clone)]
pub struct AudioAnalysisInput {
    pub audio_path: PathBuf,
    pub ffmpeg_path: PathBuf,
    pub source_duration: f64,
    pub retained_ranges: Vec<(f64, f64)>,
    pub bins: usize,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaveformBin {
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioAnalysis {
    pub source_duration: f64,
    pub bins: Vec<WaveformBin>,
    pub peak: f64,
    pub rms: f64,
    pub analyzed_seconds: f64,
    pub recommended_gain: f64,
}

/// Bins cover the complete file timeline; statistics use only retained source intervals.
pub fn analyze(
    input: AudioAnalysisInput,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(f64) + Send + 'static,
) -> Result<AudioAnalysis, String> {
    validate(&input)?;
    if cancel.load(Ordering::Acquire) {
        return Err("Audio analysis cancelled.".into());
    }
    let mut command = process::command(&input.ffmpeg_path);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-xerror",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(&input.audio_path)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-t"])
        .arg(format!("{:.9}", input.source_duration))
        .args(["-c:a", "pcm_f32le", "-f", "wav", "pipe:1"]);
    run(command, input, cancel, progress, Duration::from_secs(600))
}

fn validate(input: &AudioAnalysisInput) -> Result<(), String> {
    if !input.source_duration.is_finite()
        || !(0.0..=86_400.0).contains(&input.source_duration)
        || input.source_duration == 0.0
    {
        return Err("Audio analysis supports a source duration up to 24 hours.".into());
    }
    if !(1..=4096).contains(&input.bins) || input.retained_ranges.len() > 512 {
        return Err("Audio analysis supports up to 4096 bins and 512 retained intervals.".into());
    }
    let mut previous = 0.0;
    for &(start, end) in &input.retained_ranges {
        if !start.is_finite()
            || !end.is_finite()
            || start + screenrecord_core::TIME_EPSILON < previous
            || end <= start
            || end > input.source_duration
        {
            return Err("Audio analysis requires ordered nonoverlapping source intervals.".into());
        }
        previous = end;
    }
    if !input.audio_path.is_absolute()
        || !input.audio_path.is_file()
        || !input.ffmpeg_path.is_absolute()
        || !input.ffmpeg_path.is_file()
    {
        return Err("The local audio file or media processor is unavailable.".into());
    }
    Ok(())
}

fn run(
    mut command: Command,
    input: AudioAnalysisInput,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(f64),
    deadline: Duration,
) -> Result<AudioAnalysis, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Cannot start local audio analysis.".to_owned())?;
    let job = match process::ChildJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = cleanup::run(
                Path::new("audio-analysis"),
                Duration::from_secs(2),
                move || child.wait().map(|_| ()).map_err(|e| e.to_string()),
            );
            return Err(error);
        }
    };
    let stdout = child
        .stdout
        .take()
        .ok_or("Audio analysis pipe is unavailable.")?;
    let (done, result) = mpsc::sync_channel(1);
    let (updates, positions) = mpsc::sync_channel(1);
    let reader_cancel = cancel.clone();
    let reader = match thread::Builder::new()
        .name("audio-analysis-reader".into())
        .spawn(move || {
            let output = scan(
                BufReader::with_capacity(65_536, stdout),
                &input,
                &reader_cancel,
                |position| {
                    let _ = updates.try_send(position);
                },
            );
            let _ = done.send(output);
        }) {
        Ok(reader) => reader,
        Err(_) => {
            let _ = job.terminate();
            let _ = cleanup::run(
                Path::new("audio-analysis"),
                Duration::from_secs(2),
                move || child.wait().map(|_| ()).map_err(|e| e.to_string()),
            );
            return Err("Cannot start the audio analysis reader.".into());
        }
    };
    progress(0.0);
    let started = Instant::now();
    let mut last_progress = Instant::now();
    let mut value = None;
    let outcome = loop {
        if cancel.load(Ordering::Acquire) {
            break Err("Audio analysis cancelled.".into());
        }
        if started.elapsed() > deadline {
            break Err("Audio analysis exceeded its 10-minute deadline.".into());
        }
        if last_progress.elapsed() > Duration::from_secs(30) {
            break Err("Audio analysis stopped making progress.".into());
        }
        for position in positions.try_iter() {
            last_progress = Instant::now();
            progress(position.min(0.99));
        }
        if value.is_none() {
            match result.recv_timeout(Duration::from_millis(25)) {
                Ok(Ok(output)) => value = Some(output),
                Ok(Err(error)) => break Err(error),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err("Audio analysis reader stopped unexpectedly.".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        } else {
            thread::sleep(Duration::from_millis(5));
        }
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                break Err("The audio source could not be decoded completely.".into())
            }
            Ok(Some(_)) if value.is_some() => break Ok(value.take().unwrap()),
            Err(_) => break Err("Cannot inspect the audio analysis process.".into()),
            _ => {}
        }
    };
    let _ = job.terminate();
    let _ = child.kill();
    let released = cleanup::run(
        Path::new("audio-analysis"),
        Duration::from_secs(2),
        move || {
            child
                .wait()
                .map_err(|_| "Cannot release the audio analysis process.".to_owned())?;
            reader
                .join()
                .map_err(|_| "Audio analysis reader stopped unexpectedly.".to_owned())
        },
    );
    released?;
    let output = outcome?;
    progress(1.0);
    Ok(output)
}

pub(crate) fn sample_at(seconds: f64, rate: u32) -> u64 {
    let rate = f64::from(rate);
    let sample = (seconds * rate).floor() as u64;
    sample + u64::from(sample as f64 / rate < seconds)
}

pub(crate) fn recommended_gain(peak: f64, rms: f64) -> f64 {
    if rms <= 1e-12 || peak <= 1e-12 {
        return 1.0;
    }
    (10f64.powf(-18.0 / 20.0) / rms)
        .min(10f64.powf(-1.0 / 20.0) / peak)
        .clamp(0.01, screenrecord_core::MAX_RECORDED_GAIN)
}

fn scan(
    mut reader: impl Read,
    input: &AudioAnalysisInput,
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<AudioAnalysis, String> {
    let (rate, channels) = header(&mut reader)?;
    let limit = sample_at(input.source_duration, rate);
    let ranges: Vec<_> = if input.retained_ranges.is_empty() {
        vec![(0, limit)]
    } else {
        input
            .retained_ranges
            .iter()
            .map(|&(a, b)| (sample_at(a, rate), sample_at(b, rate)))
            .collect()
    };
    let mut bins = vec![WaveformBin::default(); input.bins];
    let mut seen = vec![false; input.bins];
    let mut frame = 0u64;
    let mut retained = 0u64;
    let mut range = 0usize;
    let mut peak = 0f64;
    let mut squares = 0f64;
    let mut correction = 0f64;
    let frame_bytes = channels * 4;
    let mut buffer = [0u8; 65_536];
    let mut pending = 0;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Audio analysis cancelled.".into());
        }
        let read = reader
            .read(&mut buffer[pending..])
            .map_err(|_| "Audio analysis stream failed.".to_owned())?;
        let bytes = pending + read;
        let whole = bytes / frame_bytes * frame_bytes;
        for data in buffer[..whole].chunks_exact(frame_bytes) {
            if frame >= limit {
                break;
            }
            while range < ranges.len() && frame >= ranges[range].1 {
                range += 1;
            }
            let selected = range < ranges.len() && frame >= ranges[range].0;
            let bin = ((frame as f64 / f64::from(rate) / input.source_duration * input.bins as f64)
                .floor() as usize)
                .min(input.bins - 1);
            for sample in data.as_chunks::<4>().0 {
                let sample = f64::from(f32::from_le_bytes(*sample));
                if !sample.is_finite() {
                    return Err("Audio contains nonfinite samples.".into());
                }
                if !seen[bin] {
                    bins[bin] = WaveformBin {
                        min: sample,
                        max: sample,
                    };
                    seen[bin] = true;
                } else {
                    bins[bin].min = bins[bin].min.min(sample);
                    bins[bin].max = bins[bin].max.max(sample);
                }
                if selected {
                    peak = peak.max(sample.abs());
                    let value = sample * sample - correction;
                    let total = squares + value;
                    correction = (total - squares) - value;
                    squares = total;
                }
            }
            if selected {
                retained += 1;
            }
            frame += 1;
        }
        progress(frame as f64 / limit.max(1) as f64);
        if read == 0 {
            if bytes != whole {
                return Err("Audio analysis ended with an incomplete sample.".into());
            }
            break;
        }
        // Consume the producer through EOF even after the requested duration, keeping its pipe bounded.
        pending = bytes - whole;
        buffer.copy_within(whole..bytes, 0);
    }
    if frame == 0 {
        return Err("The selected file contains no complete audio samples.".into());
    }
    let rms = if retained == 0 {
        0.0
    } else {
        (squares / (retained as f64 * channels as f64)).sqrt()
    };
    Ok(AudioAnalysis {
        source_duration: input.source_duration,
        bins,
        peak,
        rms,
        analyzed_seconds: retained as f64 / f64::from(rate),
        recommended_gain: recommended_gain(peak, rms),
    })
}

fn header(reader: &mut impl Read) -> Result<(u32, usize), String> {
    let invalid = || "The audio processor returned an unsupported PCM stream.".to_owned();
    let mut riff = [0; 12];
    reader.read_exact(&mut riff).map_err(|_| invalid())?;
    if &riff[..4] != b"RIFF" || &riff[8..] != b"WAVE" {
        return Err(invalid());
    }
    let mut format = None;
    let mut consumed = 12usize;
    loop {
        let mut chunk = [0; 8];
        reader.read_exact(&mut chunk).map_err(|_| invalid())?;
        let length = u32::from_le_bytes(chunk[4..].try_into().unwrap()) as usize;
        consumed = consumed.saturating_add(8);
        if &chunk[..4] == b"data" {
            return format.ok_or_else(invalid);
        }
        if length > 65_536 || consumed.saturating_add(length) > 65_536 {
            return Err(invalid());
        }
        let mut data = vec![0; length + length % 2];
        reader.read_exact(&mut data).map_err(|_| invalid())?;
        consumed += data.len();
        if &chunk[..4] == b"fmt " {
            if data.len() < 16 {
                return Err(invalid());
            }
            let tag = u16::from_le_bytes(data[..2].try_into().unwrap());
            let channels = u16::from_le_bytes(data[2..4].try_into().unwrap()) as usize;
            let rate = u32::from_le_bytes(data[4..8].try_into().unwrap());
            let bits = u16::from_le_bytes(data[14..16].try_into().unwrap());
            let float = tag == 3
                || (tag == 0xfffe && data.len() >= 40 && data[24..26] == 3u16.to_le_bytes());
            if !float
                || bits != 32
                || !(1..=8).contains(&channels)
                || !(1..=768_000).contains(&rate)
                || u16::from_le_bytes(data[12..14].try_into().unwrap()) as usize != channels * 4
            {
                return Err(invalid());
            }
            format = Some((rate, channels));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn input(duration: f64, ranges: Vec<(f64, f64)>, bins: usize) -> AudioAnalysisInput {
        AudioAnalysisInput {
            audio_path: std::env::current_exe().unwrap(),
            ffmpeg_path: std::env::current_exe().unwrap(),
            source_duration: duration,
            retained_ranges: ranges,
            bins,
        }
    }

    fn pcm(rate: u32, channels: u16, samples: &[f32]) -> Vec<u8> {
        let length = (samples.len() * 4) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + length).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&3u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * u32::from(channels) * 4).to_le_bytes());
        bytes.extend_from_slice(&(channels * 4).to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&length.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn removed_loud_samples_affect_waveform_but_not_normalization() {
        for rate in [44_100, 48_000] {
            let ranges = vec![(0.201234, 0.452345), (1.201234, 1.452345)];
            let samples: Vec<_> = (0..rate * 2)
                .map(|frame| {
                    let time = f64::from(frame) / f64::from(rate);
                    if ranges.iter().any(|&(a, b)| time >= a && time < b) {
                        0.1
                    } else {
                        0.9
                    }
                })
                .collect();
            let data = pcm(rate, 1, &samples);
            let output = scan(
                Cursor::new(data),
                &input(2.0, ranges, 4),
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap();
            assert!((output.peak - 0.1).abs() < 1e-7);
            assert!((output.rms - 0.1).abs() < 1e-7);
            assert!((output.recommended_gain - 10f64.powf(2.0 / 20.0)).abs() < 1e-6);
            assert!(output.bins.iter().all(|bin| bin.max > 0.89));
            assert!(output.bins[0].min < 0.11 && output.bins[2].min < 0.11);
            assert!(output.bins[1].min > 0.89 && output.bins[3].min > 0.89);
            let expected =
                samples.iter().filter(|value| **value < 0.2).count() as f64 / f64::from(rate);
            assert_eq!(output.analyzed_seconds, expected);
        }
    }

    #[test]
    fn normalization_handles_silence_peak_cap_and_bounded_boost() {
        assert_eq!(recommended_gain(0.0, 0.0), 1.0);
        assert_eq!(recommended_gain(0.00001, 0.00001), 2.0);
        assert!((recommended_gain(1.0, 0.01) - 10f64.powf(-1.0 / 20.0)).abs() < 1e-12);
        let output = scan(
            Cursor::new(pcm(8_000, 2, &[0.5, -0.5, 0.0, 0.0])),
            &input(0.00025, vec![], 2),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(output.peak, 0.5);
        assert!((output.rms - 0.125f64.sqrt()).abs() < 1e-12);
        assert_eq!(output.bins[0].min, -0.5);
        assert_eq!(output.bins[0].max, 0.5);
    }

    #[test]
    fn normalization_cap_preserves_targets_at_rms_and_peak_boundaries() {
        let target_rms = 10f64.powf(-18.0 / 20.0);
        let target_peak = 10f64.powf(-1.0 / 20.0);
        let rms_boundary = target_rms / 2.0;
        let peak_boundary = target_peak / 2.0;
        for factor in [0.999_999, 1.0, 1.000_001] {
            let gain = recommended_gain(0.2, rms_boundary * factor);
            assert!((gain - (2.0 / factor).min(2.0)).abs() < 1e-12);
            let gain = recommended_gain(peak_boundary * factor, 0.001);
            assert!((gain - (2.0 / factor).min(2.0)).abs() < 1e-12);
        }
        assert!((recommended_gain(0.5, target_rms) - 1.0).abs() < 1e-12);
        assert_eq!(recommended_gain(100.0, 100.0), 0.01);
        for amplitude in [0.000_001, 0.000_01, 0.001, 0.01] {
            assert_eq!(recommended_gain(amplitude, amplitude), 2.0);
        }
        for amplitude in [0.0, 1e-14, 1e-12] {
            assert_eq!(recommended_gain(amplitude, amplitude), 1.0);
        }
    }

    #[test]
    fn near_full_scale_peaks_prevent_boost_even_when_average_is_quiet() {
        let mut samples = vec![0.001; 48_000 * 2];
        samples[48_000] = 0.99;
        samples[48_001] = -0.99;
        let output = scan(
            Cursor::new(pcm(48_000, 2, &samples)),
            &input(1.0, vec![], 16),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let expected_peak = f64::from(0.99f32);
        let expected_rms = ((2.0 * expected_peak.powi(2) + 95_998.0 * f64::from(0.001f32).powi(2))
            / 96_000.0)
            .sqrt();
        assert_eq!(output.peak, expected_peak);
        assert!((output.rms - expected_rms).abs() < 1e-12);
        assert!(output.recommended_gain < 1.0);
        assert!((output.peak * output.recommended_gain - 10f64.powf(-1.0 / 20.0)).abs() < 1e-12);
    }

    #[test]
    fn quiet_retained_ranges_cap_at_two_while_removed_peaks_remain_in_waveform() {
        for rate in [44_100, 48_000] {
            let ranges = vec![(0.201234, 0.452345), (1.201234, 1.452345)];
            let mut selected = 0u64;
            let samples: Vec<f32> = (0..rate * 2)
                .flat_map(|frame| {
                    let time = f64::from(frame) / f64::from(rate);
                    if ranges
                        .iter()
                        .any(|&(start, end)| time >= start && time < end)
                    {
                        selected += 1;
                        [0.01, -0.02]
                    } else {
                        [0.99, -0.99]
                    }
                })
                .collect();
            let output = scan(
                Cursor::new(pcm(rate, 2, &samples)),
                &input(2.0, ranges, 4),
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap();
            let expected_rms =
                ((f64::from(0.01f32).powi(2) + f64::from(0.02f32).powi(2)) / 2.0).sqrt();
            assert_eq!(output.peak, f64::from(0.02f32));
            assert!((output.rms - expected_rms).abs() < 1e-12);
            assert_eq!(output.recommended_gain, 2.0);
            assert_eq!(output.analyzed_seconds, selected as f64 / f64::from(rate));
            assert!(output
                .bins
                .iter()
                .all(|bin| bin.max > 0.98 && bin.min < -0.98));
        }
    }

    #[test]
    fn analysis_rejects_invalid_ranges_nonfinite_pcm_and_partial_frames() {
        for ranges in [
            vec![(0.5, 0.4)],
            vec![(0.0, 0.8), (0.7, 1.0)],
            vec![(0.0, f64::NAN)],
            vec![(0.0, 2.0)],
        ] {
            assert!(validate(&input(1.0, ranges, 1)).is_err());
        }
        assert!(validate(&input(1.0, vec![], 4097)).is_err());
        assert!(scan(
            Cursor::new(pcm(8_000, 1, &[f32::NAN])),
            &input(1.0, vec![], 1),
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err());
        let mut truncated = pcm(8_000, 1, &[0.5]);
        truncated.pop();
        assert!(scan(
            Cursor::new(truncated),
            &input(1.0, vec![], 1),
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err());
        assert_eq!(sample_at(1.1, 48_000), 52_800);
        assert_eq!(
            sample_at(f64::from_bits(1.1f64.to_bits() + 1), 48_000),
            52_801
        );
    }

    #[test]
    fn long_stream_has_fixed_bin_count_and_cancels_without_full_buffering() {
        let header = pcm(48_000, 2, &[]);
        let data = Cursor::new(header).chain(std::io::repeat(0).take(60 * 48_000 * 8));
        let output = scan(
            data,
            &input(60.0, vec![], 4096),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(output.bins.len(), 4096);
        assert_eq!(output.analyzed_seconds, 60.0);
        assert_eq!(output.recommended_gain, 1.0);
        let cancel = AtomicBool::new(false);
        let data = Cursor::new(pcm(48_000, 2, &[])).chain(std::io::repeat(0));
        assert!(scan(data, &input(3600.0, vec![], 4096), &cancel, |_| cancel
            .store(true, Ordering::Release))
        .unwrap_err()
        .contains("cancelled"));
    }

    #[test]
    #[ignore = "Synthetic FFmpeg audio analysis; set SCREENRECORD_FFMPEG. No devices."]
    fn media_contract_audio_analysis_streams_native_pcm_and_cancels_owned_child() {
        let ffmpeg: PathBuf = std::env::var_os("SCREENRECORD_FFMPEG")
            .expect("Set SCREENRECORD_FFMPEG")
            .into();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audio.wav");
        let rate = 44_100;
        let samples: Vec<_> = (0..rate * 2)
            .map(|frame| if frame < rate { 0.1 } else { 0.9 })
            .collect();
        std::fs::write(&path, pcm(rate, 1, &samples)).unwrap();
        let make_input = || AudioAnalysisInput {
            audio_path: path.clone(),
            ffmpeg_path: ffmpeg.clone(),
            source_duration: 2.0,
            retained_ranges: vec![(0.2, 0.8)],
            bins: 32,
        };
        let output = analyze(make_input(), Arc::new(AtomicBool::new(false)), |_| {}).unwrap();
        assert!((output.rms - 0.1).abs() < 1e-7);
        assert!(output.bins[31].max > 0.89);
        let mut command = process::command(&ffmpeg);
        command.args([
            "-v",
            "error",
            "-nostdin",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-t",
            "30",
            "-c:a",
            "pcm_f32le",
            "-f",
            "wav",
            "pipe:1",
        ]);
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            signal.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert!(run(
            command,
            make_input(),
            cancel,
            |_| {},
            Duration::from_secs(5)
        )
        .unwrap_err()
        .contains("cancelled"));
        canceller.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
