use super::{ducking_media::*, *};
use crate::ducking::Ducker;
use screenrecord_core::{VolumeKeyframe, VolumeKeyframes};

fn points(values: &[(f64, f64)]) -> Vec<VolumeKeyframe> {
    values
        .iter()
        .enumerate()
        .map(|(index, &(time, volume))| VolumeKeyframe {
            id: index.to_string(),
            time,
            volume,
        })
        .collect()
}

fn automated_fixture(ffmpeg: &Path, directory: &Path) -> Project {
    let mut project = configured_fixture(ffmpeg, directory);
    project.edits.camera.enabled = false;
    project.edits.audio_processing.ducking.enabled = false;
    project.edits.video_clips = [(0.201234, 0.901239), (1.501234, 2.201240)]
        .into_iter()
        .enumerate()
        .map(|(i, (start, end))| screenrecord_core::VideoClip {
            id: i.to_string(),
            start,
            end,
        })
        .collect();
    project.edits.audio_processing.volume_keyframes = VolumeKeyframes {
        microphone: points(&[
            (0.0, 0.0),
            (0.5, 1.0),
            (1.0, 0.0),
            (1.5, 1.0),
            (2.0, 0.0),
            (2.8, 0.8),
        ]),
        system: points(&[(0.0, 1.0), (2.8, 0.0)]),
        music: points(&[(0.0, 0.3), (0.6, 0.9), (2.8, 0.1)]),
    };
    let mut overlap = project.edits.music[0].clone();
    overlap.id = "overlap".into();
    overlap.start = 0.7;
    overlap.source_start = 0.2;
    overlap.duration = 1.8;
    overlap.volume = 0.25;
    project.edits.music.push(overlap);
    project
}

fn reference_pcm(ffmpeg: &Path, project: &Project, directory: &Path) -> Vec<f32> {
    fs::create_dir_all(directory).unwrap();
    let output = options(directory, "unused.mp4");
    let cancel = AtomicBool::new(false);
    let graph = graph::RenderGraph::prepare(project, &output, ffmpeg, directory, &cancel).unwrap();
    let keys = &project.edits.audio_processing.volume_keyframes;
    let settings = &project.edits.audio_processing.ducking;
    let mut reference = Ducker::new(
        settings.amount_db,
        settings.enabled,
        settings.system_enabled,
    );
    let mut expected = Vec::new();
    let chunks = graph.audio_chunks();
    assert_eq!(chunks.len(), 2);
    assert_eq!((chunks[0].first_sample, chunks[0].samples), (0, 33_601));
    assert_eq!(
        (chunks[1].first_sample, chunks[1].samples),
        (33_601, 33_600)
    );
    for (interval, chunk) in chunks.into_iter().enumerate() {
        let raw = floats(&run(&mut graph.audio_command(ffmpeg, chunk)));
        assert_eq!(raw.len(), chunk.samples as usize * 6);
        for values in raw.as_chunks::<6>().0 {
            let output_sample = expected.len() / 2;
            let time = if interval == 0 {
                0.201234 + output_sample as f64 / 48_000.0
            } else {
                1.501234 + output_sample as f64 / 48_000.0 - 0.700005
            };
            let mut buses = *values;
            for (bus, curve) in [&keys.microphone, &keys.system, &keys.music]
                .into_iter()
                .enumerate()
            {
                let gain = screenrecord_core::volume_at(curve, time);
                for sample in &mut buses[bus * 2..bus * 2 + 2] {
                    *sample = (f64::from(*sample) * gain) as f32;
                }
            }
            expected.extend(reference.sample(buses));
        }
    }
    expected
}

#[test]
fn volume_envelopes_follow_source_samples_before_ducking_across_fractional_cuts() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = automated_fixture(&ffmpeg, directory.path());
    let originals: Vec<_> = [
        &project.screen_path,
        project.microphone_path.as_ref().unwrap(),
        project.system_audio_path.as_ref().unwrap(),
        &project.edits.music[0].path,
    ]
    .into_iter()
    .map(|path| (path.clone(), fs::read(path).unwrap()))
    .collect();
    for ducking in [false, true] {
        project.edits.audio_processing.ducking.enabled = ducking;
        project.edits.audio_processing.ducking.system_enabled = ducking;
        let actual = process_audio(
            &ffmpeg,
            &project,
            &directory.path().join(format!("mixed-{ducking}")),
            65_536,
        );
        let expected = reference_pcm(
            &ffmpeg,
            &project,
            &directory.path().join(format!("reference-{ducking}")),
        );
        let maximum = actual
            .iter()
            .zip(&expected)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert_eq!(actual.len(), 67_201 * 2);
        assert!(
            maximum <= 1e-7,
            "source-time envelope before ducking error {maximum}"
        );
        let mut split = project.clone();
        split.edits.video_clips[0].end = 0.501234;
        split.edits.video_clips.insert(
            1,
            screenrecord_core::VideoClip {
                id: "adjacent".into(),
                start: 0.501234,
                end: 0.901239,
            },
        );
        let fragmented = process_audio(
            &ffmpeg,
            &split,
            &directory.path().join(format!("fragmented-{ducking}")),
            7,
        );
        assert_eq!(
            actual, fragmented,
            "Adjacent split/pipe boundaries must preserve every PCM sample."
        );
        eprintln!("Volume envelopes: duck={ducking},67201frames,3buses,fractional cuts,maximum reference error={maximum},7-byte read/adjacent split identical");
    }
    project.edits.audio_processing.volume_keyframes = VolumeKeyframes {
        microphone: points(&[(0.0, 0.0)]),
        system: points(&[(0.0, 0.0)]),
        music: points(&[(0.0, 0.0)]),
    };
    let muted = process_audio(
        &ffmpeg,
        &project,
        &directory.path().join("all-zero"),
        65_536,
    );
    assert_eq!(muted.len(), 67_201 * 2);
    assert!(muted.iter().all(|sample| *sample == 0.0));
    for (path, original) in originals {
        assert_eq!(fs::read(path).unwrap(), original);
    }
}

#[test]
fn volume_envelopes_bypass_unity_missing_muted_and_deleted_targets() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let base = automated_fixture(&ffmpeg, directory.path());
    let mut bypass_pcm = None;
    for variant in [
        "empty",
        "unity",
        "missing",
        "muted",
        "deleted",
        "deleted-music",
        "active",
    ] {
        let mut project = base.clone();
        let keys = &mut project.edits.audio_processing.volume_keyframes;
        *keys = VolumeKeyframes::default();
        match variant {
            "unity" => {
                keys.microphone = points(&[(0.0, 1.0), (2.8, 1.0)]);
                keys.system = keys.microphone.clone();
                keys.music = keys.microphone.clone();
            }
            "missing" => {
                keys.microphone = points(&[(0.0, 0.0)]);
                project.microphone_path = None;
            }
            "muted" => {
                keys.system = points(&[(0.0, 0.0)]);
                project.edits.system_volume = 0.0;
            }
            "deleted" => {
                keys.microphone = points(&[(0.901239, 1.0), (1.0, 0.0), (1.5, 1.0)]);
            }
            "deleted-music" => {
                keys.music = points(&[(0.0, 0.0)]);
                project.edits.music.truncate(1);
                project.edits.music[0].start = 1.0;
                project.edits.music[0].duration = 0.4;
                project.edits.music[0].fade_in = 0.0;
                project.edits.music[0].fade_out = 0.0;
            }
            "active" => {
                keys.system = points(&[(0.0, 0.0)]);
                project.microphone_path = None;
            }
            _ => {}
        }
        let target = directory.path().join(variant);
        fs::create_dir(&target).unwrap();
        let graph = graph::RenderGraph::prepare(
            &project,
            &options(&target, "unused.mp4"),
            &ffmpeg,
            &target,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(graph.ducker().is_some(), variant == "active", "{variant}");
        if variant == "empty" || variant == "unity" {
            let mut pcm = Vec::new();
            for chunk in graph.audio_chunks() {
                run(&mut graph.audio_command(&ffmpeg, chunk));
                pcm.extend(decode_audio(
                    &ffmpeg,
                    &target.join(format!("audio-{:05}.wav", chunk.index)),
                ));
            }
            if let Some(reference) = &bypass_pcm {
                assert_eq!(
                    &pcm, reference,
                    "Unity points must preserve the legacy PCM path exactly."
                );
            } else {
                bypass_pcm = Some(pcm);
            }
        }
    }
}

#[test]
fn volume_envelopes_standard_and_whatsapp_encode_verified_pcm_with_one_limiter() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = automated_fixture(&ffmpeg, directory.path());
    project.edits.microphone_volume = 2.0;
    project.edits.audio_processing.microphone_gain = 8.0;
    project.edits.system_volume = 2.0;
    project.edits.audio_processing.system_gain = 8.0;
    for clip in &mut project.edits.music {
        clip.volume = 2.0;
    }
    for ducking in [false, true] {
        project.edits.audio_processing.ducking.enabled = ducking;
        project.edits.audio_processing.ducking.system_enabled = ducking;
        let reference = process_audio(
            &ffmpeg,
            &project,
            &directory.path().join(format!("pcm-{ducking}")),
            65_536,
        );
        let expected = reference_pcm(
            &ffmpeg,
            &project,
            &directory.path().join(format!("numeric-{ducking}")),
        );
        assert!(reference
            .iter()
            .zip(&expected)
            .all(|(a, b)| (a - b).abs() <= 2e-7));
        let raw = directory.path().join(format!("reference-{ducking}.f32le"));
        fs::write(
            &raw,
            reference
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let limited = floats(&run(media_command(&ffmpeg)
            .args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
            .arg(&raw)
            .args(["-af", audio::LIMITER, "-f", "f32le", "-"])));
        let peak = limited
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max);
        assert!(peak <= 0.950001);
        if !ducking {
            assert!(reference.iter().any(|sample| sample.abs() > 0.95));
        }
        for compact in [false, true] {
            let mut output = options(
                directory.path(),
                &format!("envelopes-{ducking}-{compact}.mp4"),
            );
            output.fps = 30;
            if compact {
                output.preset = screenrecord_core::ExportPreset::Whatsapp;
                output.resolution = "auto".into();
            }
            let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
            export_inner(
                &project,
                &output,
                &ffmpeg,
                &Arc::new(AtomicBool::new(false)),
                &callback,
                7,
            )
            .unwrap();
            let actual = decode_audio(&ffmpeg, Path::new(&output.output_path));
            let aac = directory
                .path()
                .join(format!("reference-{ducking}-{compact}.m4a"));
            run(media_command(&ffmpeg)
                .args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
                .arg(&raw)
                .args([
                    "-af",
                    &format!("asetpts=N/SR/TB,{}", audio::LIMITER),
                    "-c:a",
                    "aac",
                    "-b:a",
                    if compact { "96000" } else { "192000" },
                    "-ar",
                    "48000",
                    "-t",
                    "1.400011000",
                    "-f",
                    "mp4",
                ])
                .arg(&aac));
            assert_eq!(
                actual,
                decode_audio(&ffmpeg, &aac),
                "Preset must encode verified PCM with exactly one final limiter."
            );
            run(media_command(&ffmpeg).args(["-i", &output.output_path, "-f", "null", "-"]));
            eprintln!("Volume envelopes MP4: duck={ducking},compact={compact},7-frame chunks,fractional cuts,AAC exact independent encode,limited PCM peak={peak}");
        }
    }
    let output = options(directory.path(), "cancelled-envelopes.mp4");
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let callback: process::Progress =
        Arc::new(Mutex::new(Box::new(move |event: ExportProgress| {
            if event.progress > 0.8001 {
                trigger.store(true, Ordering::Release);
            }
        })));
    let error = export_inner(&project, &output, &ffmpeg, &cancel, &callback, 900).unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(!Path::new(&output.output_path).exists());
    assert!(!fs::read_dir(directory.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".screenrecord-export-")));
}

#[test]
#[ignore = "Explicit 60-second PCM performance comparison; no physical devices."]
fn measure_a_minute_of_volume_envelopes_with_maximum_key_counts() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let keys = points(
        &(0..2048)
            .map(|i| (i as f64 * 60.0 / 2047.0, if i % 2 == 0 { 0.2 } else { 1.0 }))
            .collect::<Vec<_>>(),
    );
    let compiled = crate::envelopes::Envelopes::new(
        &VolumeKeyframes {
            microphone: keys.clone(),
            system: keys.clone(),
            music: keys,
        },
        [true; 3],
    )
    .unwrap();
    let mut cursor = compiled.at(0.0);
    let mut command = media_command(&ffmpeg);
    command.args([
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.1|-0.1|0.03|-0.02|0.2|-0.15:sample_rate=48000:duration=60",
        "-c:a",
        "pcm_f32le",
        "-f",
        "f32le",
        "pipe:1",
    ]);
    let cancel = AtomicBool::new(false);
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    let mut duck = Ducker::new(12.0, true, false);
    let started = Instant::now();
    process::stream_audio(
        &mut command,
        &cancel,
        &callback,
        2_880_000,
        "test-audio",
        0.0..1.0,
        |reader, progress| {
            duck.mix_stream_with(
                reader,
                directory.path(),
                0,
                2_880_000,
                &cancel,
                progress,
                |sample, buses| cursor.apply(sample, buses),
            )
        },
    )
    .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(
        fs::metadata(directory.path().join(crate::ducking::MIX_FILE))
            .unwrap()
            .len(),
        23_040_000
    );
    eprintln!("60s 6-bus decode+3 envelopes(2048keys each)+duck+stereo write: {:.4}s,2880000frames,23040000bytes,128KiB PCM arrays+144KiB numeric curves+bounded IO", elapsed.as_secs_f64());
}
