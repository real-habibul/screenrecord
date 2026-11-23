use super::*;

fn float_samples(bytes: &[u8]) -> Vec<f32> {
    assert!(bytes.len().is_multiple_of(8));
    let samples: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|sample| f32::from_le_bytes(*sample))
        .collect();
    assert!(samples.iter().all(|sample| sample.is_finite()));
    samples
}

fn pcm_mix(ffmpeg: &Path, project: &Project) -> Vec<f32> {
    let tracks = audio::prepare(project, ffmpeg, &AtomicBool::new(false)).unwrap();
    let mut command = media_command(ffmpeg);
    command.args(["-f", "lavfi", "-i", "color=s=16x16:r=1:d=1"]);
    let filters = audio::append_inputs(&mut command, &tracks, 1.0);
    float_samples(&run(command.args([
        "-filter_complex",
        &filters.join(";"),
        "-map",
        "[outa]",
        "-c:a",
        "pcm_f32le",
        "-f",
        "f32le",
        "-",
    ])))
}

fn metrics(samples: &[f32], channel: usize) -> (f64, f64) {
    assert!(samples.len() >= 48_000 * 2);
    let window = &samples[12_000 * 2..36_000 * 2];
    let power = window
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| f64::from(frame[channel]).powi(2))
        .sum::<f64>()
        / 24_000.0;
    let peak = samples
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| f64::from(frame[channel]).abs())
        .fold(0.0, f64::max);
    (power.sqrt(), peak)
}

fn amplitude(samples: &[f32], channel: usize, frequency: f64) -> f64 {
    let (mut real, mut imaginary) = (0.0, 0.0);
    for (index, frame) in samples[12_000 * 2..36_000 * 2]
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
    {
        let phase = std::f64::consts::TAU * frequency * index as f64 / 48_000.0;
        real += f64::from(frame[channel]) * phase.cos();
        imaginary += f64::from(frame[channel]) * phase.sin();
    }
    2.0 * real.hypot(imaginary) / 24_000.0
}

fn normal_export(ffmpeg: &Path, project: &Project, name: &str) -> (ExportOptions, Vec<f32>) {
    let mut output = options(Path::new(&project.directory), name);
    output.fps = 30;
    crate::export(
        project,
        &output,
        ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let info = probe_media(ffmpeg, Path::new(&output.output_path)).unwrap();
    assert_eq!(info.video().unwrap().codec_name.as_deref(), Some("h264"));
    let audio = info
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    let samples = if let Some(audio) = audio {
        assert_eq!(audio.codec_name.as_deref(), Some("aac"));
        assert_eq!(audio.channels, Some(2));
        float_samples(&audio_samples(ffmpeg, &output.output_path))
    } else {
        Vec::new()
    };
    (output, samples)
}

#[test]
fn manual_source_gain_preserves_volume_mute_legacy_and_limiter_in_real_mp4() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for recorded-source gain media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.edits = EditSettings::new(project.duration);
    project.edits.trim_end = 1.0;
    project.edits.show_cursor = false;
    project.camera_path = None;
    project.cursor.clear();
    for (path, expression) in [
        (
            project.microphone_path.as_ref().unwrap(),
            "0.1*sin(2*PI*1000*t)|0.05*sin(2*PI*1000*t)",
        ),
        (
            project.system_audio_path.as_ref().unwrap(),
            "0.04*sin(2*PI*400*t)|0.08*sin(2*PI*400*t)",
        ),
    ] {
        run(media_command(&ffmpeg)
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("aevalsrc='{expression}':sample_rate=48000:duration=2.8"),
                "-c:a",
                "pcm_f32le",
            ])
            .arg(path));
    }
    let originals: Vec<_> = [
        &project.screen_path,
        project.microphone_path.as_ref().unwrap(),
        project.system_audio_path.as_ref().unwrap(),
    ]
    .into_iter()
    .map(|path| (path.clone(), fs::read(path).unwrap()))
    .collect();
    let mut baseline = Vec::new();
    let mut baseline_rms = [0.0; 2];
    for (name, mic_volume, system_volume, mic_gain, system_gain) in [
        ("gain-baseline", 0.5, 0.25, 1.0, 1.0),
        ("gain-double", 0.5, 0.25, 2.0, 2.0),
        ("gain-mic-muted", 0.0, 0.25, 2.0, 2.0),
        ("gain-system-muted", 0.5, 0.0, 2.0, 2.0),
        ("gain-legacy", 0.125, 0.03125, 4.0, 8.0),
        ("gain-minimum", 0.5, 0.25, 0.01, 0.01),
    ] {
        project.edits.microphone_volume = mic_volume;
        project.edits.system_volume = system_volume;
        project.edits.audio_processing.microphone_gain = mic_gain;
        project.edits.audio_processing.system_gain = system_gain;
        project.validate().unwrap();
        let pcm = pcm_mix(&ffmpeg, &project);
        assert_eq!(pcm.len(), 48_000 * 2);
        let (_, aac) = normal_export(&ffmpeg, &project, &format!("{name}.mp4"));
        for (channel, (mic_source, system_source)) in
            [(0.1_f64, 0.04_f64), (0.05, 0.08)].into_iter().enumerate()
        {
            let expected_mic = mic_source * mic_volume * mic_gain;
            let expected_system = system_source * system_volume * system_gain;
            let expected_rms = ((expected_mic.powi(2) + expected_system.powi(2)) / 2.0).sqrt();
            for (frequency, expected) in [(1000.0, expected_mic), (400.0, expected_system)] {
                let actual_pcm = amplitude(&pcm, channel, frequency);
                let actual_aac = amplitude(&aac, channel, frequency);
                assert!(
                    (actual_pcm - expected).abs() < 0.000001,
                    "{name} channel{channel} {frequency}Hz PCM {actual_pcm} != {expected}"
                );
                let tolerance = if expected == 0.0 {
                    0.00002
                } else {
                    expected * 0.03 + 0.0000002
                };
                assert!(
                    (actual_aac - expected).abs() < tolerance,
                    "{name} channel{channel} {frequency}Hz AAC {actual_aac} != {expected}"
                );
            }
            let (pcm_rms, pcm_peak) = metrics(&pcm, channel);
            let (aac_rms, aac_peak) = metrics(&aac, channel);
            assert!((pcm_rms - expected_rms).abs() < 0.000001);
            assert!((aac_rms - expected_rms).abs() < expected_rms * 0.03 + 0.0000002);
            eprintln!("{name} channel{channel}: PCM RMS={pcm_rms:.8} peak={pcm_peak:.8}; AAC RMS={aac_rms:.8} peak={aac_peak:.8}");
            if name == "gain-baseline" {
                baseline_rms[channel] = aac_rms;
            }
            if name == "gain-double" {
                let ratio = aac_rms / baseline_rms[channel];
                assert!(
                    (ratio - 2.0).abs() < 0.02,
                    "channel{channel} doubled gain ratio={ratio}"
                );
                eprintln!("Doubled gain channel{channel}: AAC RMS ratio={ratio:.6}");
            }
        }
        if name == "gain-baseline" {
            baseline = aac;
        } else if name == "gain-legacy" {
            assert_eq!(
                baseline, aac,
                "Legacy gain must not be clamped during export."
            );
        }
    }
    project.edits.microphone_volume = 0.0;
    project.edits.system_volume = 0.0;
    project.edits.audio_processing.microphone_gain = 2.0;
    project.edits.audio_processing.system_gain = 2.0;
    assert!(normal_export(&ffmpeg, &project, "gain-both-muted.mp4")
        .1
        .is_empty());

    for (path, name) in [
        (&mut project.microphone_path, "loud-mic.wav"),
        (&mut project.system_audio_path, "loud-system.wav"),
    ] {
        let source = directory.path().join(name);
        run(media_command(&ffmpeg).args(["-f", "lavfi", "-i",
            "aevalsrc='0.4*sin(2*PI*1000*t)|0.4*sin(2*PI*1000*t)':sample_rate=48000:duration=2.8", "-c:a", "pcm_f32le"]).arg(&source));
        *path = Some(source.to_string_lossy().into_owned());
    }
    project.edits.microphone_volume = 1.0;
    project.edits.system_volume = 1.0;
    let pcm = pcm_mix(&ffmpeg, &project);
    let (_, aac) = normal_export(&ffmpeg, &project, "gain-summed-limiter.mp4");
    let limited_pcm = directory.path().join("independently-limited.f32le");
    fs::write(
        &limited_pcm,
        pcm.iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let reference = directory.path().join("independently-encoded.m4a");
    run(media_command(&ffmpeg)
        .args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
        .arg(&limited_pcm)
        .args([
            "-c:a", "aac", "-b:a", "192000", "-ar", "48000", "-t", "1", "-f", "mp4",
        ])
        .arg(&reference));
    let independent_aac = float_samples(&audio_samples(&ffmpeg, &reference.to_string_lossy()));
    assert_eq!(
        aac, independent_aac,
        "AAC must encode the verified limited PCM without bypassing or applying a second limiter."
    );
    for channel in 0..2 {
        let (pcm_rms, pcm_peak) = metrics(&pcm, channel);
        let (aac_rms, aac_peak) = metrics(&aac, channel);
        assert!(
            (pcm_peak - 0.95).abs() < 0.000001,
            "PCM sample limiter peak={pcm_peak}"
        );
        assert!((aac_rms - pcm_rms).abs() < pcm_rms * 0.01);
        assert!((aac_rms - metrics(&aac, 1 - channel).0).abs() < 0.00001);
        eprintln!("gain-summed-limiter channel{channel}: unprocessed peak=1.60000000; PCM RMS={pcm_rms:.8} peak={pcm_peak:.8}; AAC RMS={aac_rms:.8} peak={aac_peak:.8}");
    }
    for (path, bytes) in originals {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}
