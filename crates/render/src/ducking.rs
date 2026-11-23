use std::{
    fs::OpenOptions,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

pub(crate) const MIX_FILE: &str = "audio-ducked.f32";

pub(crate) struct Ducker {
    envelope: f64,
    gain: f64,
    minimum_gain: f64,
    attack: f64,
    envelope_release: f64,
    gain_release: f64,
    music_enabled: bool,
    system_enabled: bool,
}

impl Ducker {
    pub(crate) fn new(amount_db: f64, music_enabled: bool, system_enabled: bool) -> Self {
        Self {
            envelope: 0.0,
            gain: 1.0,
            minimum_gain: 10.0_f64.powf(-amount_db / 20.0),
            attack: (-1.0_f64 / (48_000.0 * 0.010)).exp(),
            envelope_release: (-1.0_f64 / (48_000.0 * 0.150)).exp(),
            gain_release: (-1.0_f64 / (48_000.0 * 0.250)).exp(),
            music_enabled,
            system_enabled,
        }
    }

    pub(crate) fn sample(&mut self, buses: [f32; 6]) -> [f32; 2] {
        let [left, right] = [buses[0] as f64, buses[1] as f64];
        if !self.music_enabled && !self.system_enabled {
            return [
                (left + buses[2] as f64 + buses[4] as f64) as f32,
                (right + buses[3] as f64 + buses[5] as f64) as f32,
            ];
        }
        let power = (left * left + right * right) * 0.5;
        let coefficient = if power > self.envelope {
            self.attack
        } else {
            self.envelope_release
        };
        self.envelope = power + (self.envelope - power) * coefficient;
        let activity =
            ((self.envelope.sqrt() - 0.01) / (10.0_f64.powf(-30.0 / 20.0) - 0.01)).clamp(0.0, 1.0);
        let target = 1.0 - activity * (1.0 - self.minimum_gain);
        let coefficient = if target < self.gain {
            self.attack
        } else {
            self.gain_release
        };
        self.gain = target + (self.gain - target) * coefficient;
        let system_gain = if self.system_enabled { self.gain } else { 1.0 };
        let music_gain = if self.music_enabled { self.gain } else { 1.0 };
        [
            (left + buses[2] as f64 * system_gain + buses[4] as f64 * music_gain) as f32,
            (right + buses[3] as f64 * system_gain + buses[5] as f64 * music_gain) as f32,
        ]
    }

    pub(crate) fn mix_stream(
        &mut self,
        input: impl Read,
        directory: &Path,
        first_sample: u64,
        samples: u64,
        cancel: &AtomicBool,
        progress: &AtomicU64,
    ) -> Result<(), String> {
        self.mix_stream_with(
            input,
            directory,
            first_sample,
            samples,
            cancel,
            progress,
            |_, _| {},
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mix_stream_with(
        &mut self,
        input: impl Read,
        directory: &Path,
        first_sample: u64,
        samples: u64,
        cancel: &AtomicBool,
        progress: &AtomicU64,
        mut envelope: impl FnMut(u64, &mut [f32; 6]),
    ) -> Result<(), String> {
        let output = OpenOptions::new()
            .write(true)
            .create_new(first_sample == 0)
            .append(first_sample != 0)
            .open(directory.join(MIX_FILE))
            .map_err(|error| format!("Cannot write the processed audio: {error}"))?;
        if output.metadata().map_err(|error| error.to_string())?.len() != first_sample * 8 {
            return Err("Processed audio chunks are not contiguous.".into());
        }
        let mut input = BufReader::new(input);
        let mut output = BufWriter::new(output);
        let mut bytes = [0_u8; 4096 * 24];
        let mut mixed = [0_u8; 4096 * 8];
        let mut remaining = samples;
        while remaining > 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err("Export cancelled.".into());
            }
            let frames = remaining.min(4096) as usize;
            input
                .read_exact(&mut bytes[..frames * 24])
                .map_err(|error| error.to_string())?;
            for (index, frame) in bytes[..frames * 24].as_chunks::<24>().0.iter().enumerate() {
                let mut buses = [0.0_f32; 6];
                for (value, bytes) in buses.iter_mut().zip(frame.as_chunks::<4>().0) {
                    *value = f32::from_le_bytes(*bytes);
                    if !value.is_finite() {
                        return Err("An audio source contains invalid sample values.".into());
                    }
                }
                envelope(samples - remaining + index as u64, &mut buses);
                for (channel, sample) in self.sample(buses).iter().enumerate() {
                    if !sample.is_finite() {
                        return Err("The mixed audio exceeds the supported sample range.".into());
                    }
                    mixed[index * 8 + channel * 4..index * 8 + channel * 4 + 4]
                        .copy_from_slice(&sample.to_le_bytes());
                }
            }
            output
                .write_all(&mixed[..frames * 8])
                .map_err(|error| error.to_string())?;
            remaining -= frames as u64;
            progress.store(samples - remaining, Ordering::Relaxed);
        }
        let mut extra = [0_u8; 1];
        if input.read(&mut extra).map_err(|error| error.to_string())? != 0 {
            return Err("The decoded audio buses contain extra samples.".into());
        }
        output.flush().map_err(|error| error.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_ducks_music_only_then_releases_without_changing_recorded_buses() {
        let mut duck = Ducker::new(12.0, true, false);
        let quiet = duck.sample([0.0, 0.0, 0.03, -0.02, 0.2, -0.1]);
        assert!((quiet[0] - 0.23).abs() < 1e-7 && (quiet[1] + 0.12).abs() < 1e-7);
        for _ in 0..48_000 {
            duck.sample([0.1, -0.1, 0.03, -0.02, 0.2, -0.1]);
        }
        let actual = duck.sample([0.1, -0.1, 0.03, -0.02, 0.2, -0.1]);
        let expected = 10.0_f64.powf(-12.0 / 20.0);
        assert!((duck.gain - expected).abs() < 1e-12);
        assert!((actual[0] as f64 - (0.13 + 0.2 * expected)).abs() < 1e-7);
        assert!((actual[1] as f64 - (-0.12 - 0.1 * expected)).abs() < 1e-7);
        let before = duck.gain;
        duck.sample([0.0; 6]);
        assert!((duck.gain - before).abs() < 1e-12);
        for _ in 0..144_000 {
            duck.sample([0.0; 6]);
        }
        assert!(duck.gain > 0.999);
    }

    #[test]
    fn silence_and_zero_reduction_preserve_unity() {
        let mut silence = Ducker::new(24.0, true, false);
        let mut zero = Ducker::new(0.0, true, false);
        for _ in 0..48_000 {
            let quiet = silence.sample([0.0, 0.0, 0.01, -0.01, 0.2, -0.2]);
            assert!((quiet[0] - 0.21).abs() < 1e-7 && (quiet[1] + 0.21).abs() < 1e-7);
            assert_eq!(zero.sample([0.1, -0.1, 0.0, 0.0, 0.2, -0.2]), [0.3, -0.3]);
        }
        assert_eq!(silence.gain, 1.0);
        assert_eq!(zero.gain, 1.0);
    }

    #[test]
    fn shared_preview_reference_matches_float32_output_and_float64_state() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../testdata/ducking-reference.json")).unwrap();
        let mut duck = Ducker::new(fixture["amountDb"].as_f64().unwrap(), true, false);
        let mut checkpoint = fixture["checkpoints"].as_array().unwrap().iter().peekable();
        for frame in 0..fixture["frames"].as_u64().unwrap() {
            let mic = if (4800..24000).contains(&frame) || (28800..52800).contains(&frame) {
                ((frame % 97) as f64 - 48.0) / 240.0
            } else {
                0.0
            } as f32;
            let sample = duck.sample([mic, -mic, 0.015, -0.025, 0.2, -0.15]);
            if checkpoint
                .peek()
                .is_some_and(|point| point["frame"].as_u64() == Some(frame))
            {
                let point = checkpoint.next().unwrap();
                for (channel, name) in ["left", "right"].iter().enumerate() {
                    assert!(
                        (sample[channel] as f64 - point[name].as_f64().unwrap()).abs() < 1e-7,
                        "frame {frame} {name}"
                    );
                }
                assert!(
                    (duck.envelope - point["envelope"].as_f64().unwrap()).abs() < 1e-12,
                    "envelope frame {frame}"
                );
                assert!(
                    (duck.gain - point["gain"].as_f64().unwrap()).abs() < 1e-12,
                    "gain frame {frame}"
                );
            }
        }
        assert!(checkpoint.next().is_none());
    }

    #[test]
    fn shared_detector_routes_system_music_or_both_without_changing_microphone() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../testdata/ducking-reference.json")).unwrap();
        for (music_enabled, system_enabled) in [(false, true), (true, true), (false, false)] {
            let mut duck = Ducker::new(
                fixture["amountDb"].as_f64().unwrap(),
                music_enabled,
                system_enabled,
            );
            let mut checkpoints = fixture["checkpoints"].as_array().unwrap().iter().peekable();
            for frame in 0..fixture["frames"].as_u64().unwrap() {
                let mic = if (4800..24000).contains(&frame) || (28800..52800).contains(&frame) {
                    ((frame % 97) as f64 - 48.0) / 240.0
                } else {
                    0.0
                } as f32;
                let actual = duck.sample([mic, -mic, 0.015, -0.025, 0.2, -0.15]);
                if checkpoints
                    .peek()
                    .is_some_and(|point| point["frame"].as_u64() == Some(frame))
                {
                    let point = checkpoints.next().unwrap();
                    let gain = point["gain"].as_f64().unwrap();
                    let system = if system_enabled { gain } else { 1.0 };
                    let music = if music_enabled { gain } else { 1.0 };
                    let expected = [
                        mic as f64 + 0.015 * system + 0.2 * music,
                        -mic as f64 - 0.025 * system - 0.15 * music,
                    ];
                    for (actual, expected) in actual.into_iter().zip(expected) {
                        assert!(
                            (actual as f64 - expected).abs() < 1e-7,
                            "frame{frame}, music={music_enabled}, system={system_enabled}"
                        );
                    }
                    if music_enabled || system_enabled {
                        assert!((duck.gain - gain).abs() < 1e-12);
                        assert!(
                            (duck.envelope - point["envelope"].as_f64().unwrap()).abs() < 1e-12
                        );
                    } else {
                        assert_eq!((duck.gain, duck.envelope), (1.0, 0.0));
                    }
                }
            }
            assert!(checkpoints.next().is_none());
        }
    }
}
