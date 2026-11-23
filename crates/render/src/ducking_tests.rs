use super::*;
use crate::ducking::{Ducker, MIX_FILE};
use screenrecord_core::{MusicClip, VideoClip};

pub(super) fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|sample| f32::from_le_bytes(*sample))
        .collect()
}

pub(super) fn decode_audio(ffmpeg: &Path, path: &Path) -> Vec<f32> {
    floats(&run(media_command(ffmpeg).arg("-i").arg(path).args([
        "-map", "0:a:0", "-f", "f32le", "-ar", "48000", "-ac", "2", "-",
    ])))
}

pub(super) fn configured_fixture(ffmpeg: &Path, directory: &Path) -> Project {
    let mut project = fixture(ffmpeg, directory);
    for (path, expression) in [
        (project.microphone_path.as_ref().unwrap(), "if(between(n,24000,47999)+between(n,72000,119999),0.02,0)|if(between(n,24000,47999)+between(n,72000,119999),-0.02,0)"),
        (project.system_audio_path.as_ref().unwrap(), "0.03|-0.01"),
    ] {
        run(media_command(ffmpeg).args(["-f", "lavfi", "-i", &format!("aevalsrc='{expression}':sample_rate=48000:duration=2.8"), "-c:a", "pcm_f32le"]).arg(path));
    }
    let music = directory.join("duck music.wav");
    run(media_command(ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.2|-0.15:sample_rate=48000:duration=2.8",
            "-c:a",
            "pcm_f32le",
        ])
        .arg(&music));
    project.edits.trim_start = 0.0;
    project.edits.trim_end = 2.8;
    project.edits.microphone_volume = 0.5;
    project.edits.system_volume = 0.25;
    project.edits.audio_processing.microphone_gain = 4.0;
    project.edits.audio_processing.system_gain = 2.0;
    project.edits.audio_processing.ducking.enabled = true;
    project.edits.music = vec![MusicClip {
        id: "music-duck".into(),
        name: "Music duck".into(),
        path: music.to_string_lossy().into_owned(),
        source_duration: 2.8,
        start: 0.0,
        source_start: 0.0,
        duration: 2.8,
        volume: 0.5,
        fade_in: 0.4,
        fade_out: 0.4,
    }];
    project.edits.video_clips = [(0.1, 0.9), (1.5, 2.4)]
        .into_iter()
        .enumerate()
        .map(|(i, (start, end))| VideoClip {
            id: format!("keep-{i}"),
            start,
            end,
        })
        .collect();
    project
}

struct ShortReads<T> {
    reader: T,
    maximum: usize,
}
impl<T: std::io::Read> std::io::Read for ShortReads<T> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let count = bytes.len().min(self.maximum);
        self.reader.read(&mut bytes[..count])
    }
}

pub(super) fn process_audio(
    ffmpeg: &Path,
    project: &Project,
    directory: &Path,
    maximum: usize,
) -> Vec<f32> {
    std::fs::create_dir_all(directory).unwrap();
    let options = ExportOptions {
        preset: screenrecord_core::ExportPreset::Standard,
        output_path: directory.join("result.mp4").to_string_lossy().into_owned(),
        resolution: "1080p".into(),
        fps: 30,
    };
    let cancel = AtomicBool::new(false);
    let graph = graph::RenderGraph::prepare(project, &options, ffmpeg, directory, &cancel).unwrap();
    let mut duck = graph.ducker();
    assert!(duck.is_some());
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    for chunk in graph.audio_chunks() {
        process::stream_audio(
            &mut graph.audio_command(ffmpeg, chunk),
            &cancel,
            &callback,
            chunk.samples,
            "test-audio",
            0.0..1.0,
            |reader, progress| {
                graph.mix_audio_stream(
                    ShortReads { reader, maximum },
                    chunk,
                    duck.as_mut().unwrap(),
                    &cancel,
                    progress,
                )
            },
        )
        .unwrap();
    }
    floats(&std::fs::read(directory.join(MIX_FILE)).unwrap())
}

#[test]
fn system_ducking_routes_targets_and_keeps_detector_state_across_retained_cuts() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let base = configured_fixture(&ffmpeg, directory.path());
    let mic = decode_audio(&ffmpeg, Path::new(base.microphone_path.as_ref().unwrap()));
    let system = decode_audio(&ffmpeg, Path::new(base.system_audio_path.as_ref().unwrap()));
    let music = decode_audio(&ffmpeg, Path::new(&base.edits.music[0].path));
    let originals: Vec<_> = [
        base.microphone_path.as_ref().unwrap(),
        base.system_audio_path.as_ref().unwrap(),
        &base.edits.music[0].path,
    ]
    .into_iter()
    .map(|path| (path.clone(), fs::read(path).unwrap()))
    .collect();
    for (name, music_present, music_enabled) in [
        ("system-only", false, false),
        ("music-unaffected", true, false),
        ("both", true, true),
    ] {
        let mut project = base.clone();
        if !music_present {
            project.edits.music.clear();
        }
        project.edits.audio_processing.ducking.enabled = music_enabled;
        project.edits.audio_processing.ducking.system_enabled = true;
        let actual = process_audio(&ffmpeg, &project, &directory.path().join(name), 65_536);
        let fragmented = process_audio(
            &ffmpeg,
            &project,
            &directory.path().join(format!("{name}-fragmented")),
            7,
        );
        assert_eq!(actual.len(), 81_600 * 2);
        assert_eq!(actual, fragmented);
        let mut legacy_reference = Ducker::new(12.0, true, false);
        let mut expected = Vec::new();
        for frame in (4_800..43_200).chain(72_000..115_200) {
            let fade = (frame as f64 / 19_200.0).min(1.0);
            let recorded = [system[frame * 2] * 0.5, system[frame * 2 + 1] * 0.5];
            let music = if music_present {
                [
                    (music[frame * 2] as f64 * 0.5 * fade) as f32,
                    (music[frame * 2 + 1] as f64 * 0.5 * fade) as f32,
                ]
            } else {
                [0.0; 2]
            };
            // Move selected target buses into the already verified legacy music input.
            let unprocessed = if music_enabled { [0.0; 2] } else { music };
            let target = if music_enabled {
                [recorded[0] + music[0], recorded[1] + music[1]]
            } else {
                recorded
            };
            expected.extend_from_slice(&legacy_reference.sample([
                mic[frame * 2] * 2.0,
                mic[frame * 2 + 1] * 2.0,
                unprocessed[0],
                unprocessed[1],
                target[0],
                target[1],
            ]));
        }
        let error = actual
            .iter()
            .zip(&expected)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(error < 2e-7, "{name} routing mismatch={error}");
        eprintln!("{name}:81600 outputframes, retained cuts+7-byte reads preserve state, legacy-routed PCM maxerror={error}");
    }
    for (path, bytes) in originals {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn system_ducking_ignores_deleted_mic_and_bypasses_absent_muted_or_deleted_targets() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = configured_fixture(&ffmpeg, directory.path());
    run(media_command(&ffmpeg).args(["-f", "lavfi", "-i", "aevalsrc='if(gte(n,43200)*lt(n,72000),0.8,0)|if(gte(n,43200)*lt(n,72000),-0.8,0)':sample_rate=48000:duration=2.8", "-c:a", "pcm_f32le"]).arg(project.microphone_path.as_ref().unwrap()));
    project.edits.audio_processing.ducking.enabled = false;
    project.edits.audio_processing.ducking.system_enabled = true;
    project.edits.music.clear();
    let actual = process_audio(
        &ffmpeg,
        &project,
        &directory.path().join("no-deleted-speech"),
        7,
    );
    for sample in actual.as_chunks::<2>().0 {
        assert!(
            (sample[0] - 0.015).abs() < 1e-7 && (sample[1] + 0.005).abs() < 1e-7,
            "Deleted microphone influenced ducking: {sample:?}"
        );
    }
    let base = project.clone();
    for scenario in [
        "no-mic",
        "muted-mic",
        "no-system",
        "muted-system",
        "disabled",
        "deleted-music",
    ] {
        let mut project = base.clone();
        match scenario {
            "no-mic" => project.microphone_path = None,
            "muted-mic" => project.edits.microphone_volume = 0.0,
            "no-system" => project.system_audio_path = None,
            "muted-system" => project.edits.system_volume = 0.0,
            "disabled" => project.edits.audio_processing.ducking.system_enabled = false,
            "deleted-music" => {
                project.edits.audio_processing.ducking.system_enabled = false;
                project.edits.audio_processing.ducking.enabled = true;
                project.edits.music.push(MusicClip {
                    id: "deleted-target".into(),
                    name: "Deleted target".into(),
                    path: base.system_audio_path.clone().unwrap(),
                    source_duration: 2.8,
                    start: 1.0,
                    source_start: 0.0,
                    duration: 0.2,
                    volume: 1.0,
                    fade_in: 0.0,
                    fade_out: 0.0,
                });
            }
            _ => unreachable!(),
        }
        let target = directory.path().join(scenario);
        fs::create_dir(&target).unwrap();
        let output = options(&target, "unused.mp4");
        let graph = graph::RenderGraph::prepare(
            &project,
            &output,
            &ffmpeg,
            &target,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            graph.ducker().is_none(),
            "{scenario} must bypass unused DSP"
        );
    }
}

#[test]
fn system_ducking_standard_and_compact_mp4_match_pcm_with_and_without_music() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let base = configured_fixture(&ffmpeg, directory.path());
    for music_enabled in [false, true] {
        let mut project = base.clone();
        if !music_enabled {
            project.edits.music.clear();
        }
        project.edits.audio_processing.ducking.enabled = music_enabled;
        project.edits.audio_processing.ducking.system_enabled = true;
        let reference = process_audio(
            &ffmpeg,
            &project,
            &directory.path().join(format!("reference-{music_enabled}")),
            65_536,
        );
        for compact in [false, true] {
            let mut output = options(
                directory.path(),
                &format!("system-duck-{music_enabled}-{compact}.mp4"),
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
            assert!(actual.len() >= reference.len());
            let rmse = (actual
                .iter()
                .zip(&reference)
                .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                .sum::<f64>()
                / reference.len() as f64)
                .sqrt();
            let reference_pcm = directory
                .path()
                .join(format!("reference-{music_enabled}.f32le"));
            fs::write(
                &reference_pcm,
                reference
                    .iter()
                    .flat_map(|sample| sample.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let reference_aac = directory
                .path()
                .join(format!("reference-{music_enabled}-{compact}.m4a"));
            run(media_command(&ffmpeg)
                .args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
                .arg(&reference_pcm)
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
                    "1.7",
                    "-f",
                    "mp4",
                ])
                .arg(&reference_aac));
            assert_eq!(actual, decode_audio(&ffmpeg, &reference_aac),
                "Standard/compact AAC must exactly encode the verified post-duck PCM with one limiter.");
            let info = probe_media(&ffmpeg, Path::new(&output.output_path)).unwrap();
            assert_eq!(info.video().unwrap().codec_name.as_deref(), Some("h264"));
            assert_eq!(
                info.streams
                    .iter()
                    .find(|stream| stream.codec_type.as_deref() == Some("audio"))
                    .unwrap()
                    .codec_name
                    .as_deref(),
                Some("aac")
            );
            eprintln!("System ducking MP4: music={music_enabled}, compact={compact},7-frame chunks+retained cuts, AAC RMSE={rmse:.8}");
        }
    }
}

#[test]
fn normalization_and_music_ducking_keep_bus_identity_and_output_state_across_cuts_and_chunks() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for duck media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let project = configured_fixture(&ffmpeg, directory.path());
    let originals: Vec<_> = [
        project.microphone_path.as_ref().unwrap(),
        project.system_audio_path.as_ref().unwrap(),
        &project.edits.music[0].path,
    ]
    .iter()
    .map(|path| std::fs::read(path).unwrap())
    .collect();
    let actual = process_audio(
        &ffmpeg,
        &project,
        &directory.path().join("whole"),
        60 * 48_000,
    );
    let chunked = process_audio(&ffmpeg, &project, &directory.path().join("small"), 7);
    assert_eq!(actual.len(), 81_600 * 2);
    assert_eq!(chunked.len(), actual.len());
    let source = decode_audio(
        &ffmpeg,
        Path::new(project.microphone_path.as_ref().unwrap()),
    );
    let music = decode_audio(&ffmpeg, Path::new(&project.edits.music[0].path));
    let system = decode_audio(
        &ffmpeg,
        Path::new(project.system_audio_path.as_ref().unwrap()),
    );
    let mut duck = Ducker::new(12.0, true, false);
    let mut expected = Vec::new();
    for frame in (4_800..43_200).chain(72_000..115_200) {
        let fade = (frame as f64 / 19_200.0).min(1.0);
        expected.extend_from_slice(&duck.sample([
            source[frame * 2] * 2.0,
            source[frame * 2 + 1] * 2.0,
            system[frame * 2] * 0.5,
            system[frame * 2 + 1] * 0.5,
            (music[frame * 2] as f64 * 0.5 * fade) as f32,
            (music[frame * 2 + 1] as f64 * 0.5 * fade) as f32,
        ]));
    }
    let max = actual
        .iter()
        .zip(&expected)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    let chunk_max = actual
        .iter()
        .zip(&chunked)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    eprintln!("Ducking: {} output frames, independent reference maxerror {max}, forced 7-byte pipe fragment maxerror {chunk_max}", actual.len()/2);
    assert!(max < 2e-6);
    assert!(chunk_max < 2e-6);
    for (path, before) in [
        project.microphone_path.as_ref().unwrap(),
        project.system_audio_path.as_ref().unwrap(),
        &project.edits.music[0].path,
    ]
    .iter()
    .zip(originals)
    {
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}

#[test]
fn ducking_export_encodes_once_and_cancellation_removes_all_owned_audio_files() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for duck export tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let project = configured_fixture(&ffmpeg, directory.path());
    let output = directory.path().join("ducked.mp4");
    let options = ExportOptions {
        preset: screenrecord_core::ExportPreset::Standard,
        output_path: output.to_string_lossy().into_owned(),
        resolution: "1080p".into(),
        fps: 30,
    };
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    export_inner(
        &project,
        &options,
        &ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        12,
    )
    .unwrap();
    let decoded = decode_audio(&ffmpeg, &output);
    let reference = process_audio(
        &ffmpeg,
        &project,
        &directory.path().join("reference"),
        60 * 48_000,
    );
    assert!(decoded.len() >= reference.len());
    let rmse = (decoded
        .iter()
        .zip(&reference)
        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
        .sum::<f64>()
        / reference.len() as f64)
        .sqrt();
    eprintln!("Ducking AAC full-decode RMSE {rmse}");
    assert!(rmse < 0.01);
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(move |progress| {
        if progress.progress > 0.80 {
            signal.store(true, Ordering::Relaxed);
        }
    })));
    let options = ExportOptions {
        preset: screenrecord_core::ExportPreset::Standard,
        output_path: directory
            .path()
            .join("cancel.mp4")
            .to_string_lossy()
            .into_owned(),
        ..options
    };
    assert!(export_inner(&project, &options, &ffmpeg, &cancel, &callback, 12).is_err());
    assert!(!Path::new(&options.output_path).exists());
    assert!(output.is_file());
    assert!(!std::fs::read_dir(directory.path())
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".screenrecord-export-")));
}

#[test]
fn raw_duck_mixer_rejects_invalid_samples_and_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let mut frame = vec![0_u8; 24];
    frame[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(Ducker::new(12.0, true, false)
        .mix_stream(
            std::io::Cursor::new(frame),
            directory.path(),
            0,
            1,
            &AtomicBool::new(false),
            &std::sync::atomic::AtomicU64::new(0)
        )
        .unwrap_err()
        .contains("invalid sample"));
    let other = tempfile::tempdir().unwrap();
    assert!(Ducker::new(12.0, true, false)
        .mix_stream(
            std::io::Cursor::new([0_u8; 24]),
            other.path(),
            0,
            1,
            &AtomicBool::new(true),
            &std::sync::atomic::AtomicU64::new(0)
        )
        .unwrap_err()
        .contains("cancelled"));
    for bytes in [vec![0_u8; 23], vec![0_u8; 25]] {
        let target = tempfile::tempdir().unwrap();
        assert!(Ducker::new(12.0, true, false)
            .mix_stream(
                std::io::Cursor::new(bytes),
                target.path(),
                0,
                1,
                &AtomicBool::new(false),
                &std::sync::atomic::AtomicU64::new(0)
            )
            .is_err());
    }
}

#[test]
fn removed_microphone_audio_cannot_duck_kept_music_and_missing_microphone_bypasses_ducking() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for cut ducking tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = configured_fixture(&ffmpeg, directory.path());
    run(media_command(&ffmpeg).args(["-f", "lavfi", "-i", "aevalsrc='if(gte(n,43200)*lt(n,72000),0.8,0)|if(gte(n,43200)*lt(n,72000),-0.8,0)':sample_rate=48000:duration=2.8", "-c:a", "pcm_f32le"]).arg(project.microphone_path.as_ref().unwrap()));
    let actual = process_audio(&ffmpeg, &project, &directory.path().join("kept"), 65_536);
    let mut expected = Vec::new();
    for frame in (4_800..43_200).chain(72_000..115_200) {
        let fade = (frame as f32 / 19_200.0).min(1.0);
        expected.extend_from_slice(&[0.015 + 0.1 * fade, -0.005 - 0.075 * fade]);
    }
    let error = actual
        .iter()
        .zip(&expected)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(error < 1e-7, "removed speech changed kept music: {error}");
    project.microphone_path = None;
    let target = directory.path().join("no-microphone");
    std::fs::create_dir(&target).unwrap();
    let options = ExportOptions {
        preset: screenrecord_core::ExportPreset::Standard,
        output_path: target.join("unused.mp4").to_string_lossy().into_owned(),
        resolution: "1080p".into(),
        fps: 30,
    };
    let graph = graph::RenderGraph::prepare(
        &project,
        &options,
        &ffmpeg,
        &target,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(graph.ducker().is_none());
    let mut decoded = Vec::new();
    for chunk in graph.audio_chunks() {
        run(&mut graph.audio_command(&ffmpeg, chunk));
        decoded.extend(decode_audio(
            &ffmpeg,
            &target.join(format!("audio-{:05}.wav", chunk.index)),
        ));
    }
    assert_eq!(decoded.len(), actual.len());
    assert!(decoded
        .iter()
        .zip(&actual)
        .all(|(a, b)| (a - b).abs() < 1e-7));
}

#[test]
fn a_minute_of_ducking_streams_with_exact_sample_count_and_bounded_buffers() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for audio stream timing.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
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
            duck.mix_stream(reader, directory.path(), 0, 2_880_000, &cancel, progress)
        },
    )
    .unwrap();
    let elapsed = started.elapsed();
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(directory.path().join(MIX_FILE)).unwrap();
    assert_eq!(file.metadata().unwrap().len(), 23_040_000);
    file.seek(SeekFrom::End(-8)).unwrap();
    let mut bytes = [0_u8; 8];
    file.read_exact(&mut bytes).unwrap();
    let actual = floats(&bytes);
    let gain = 10.0_f64.powf(-12.0 / 20.0);
    assert!((actual[0] as f64 - (0.13 + 0.2 * gain)).abs() < 1e-7);
    assert!((actual[1] as f64 - (-0.12 - 0.15 * gain)).abs() < 1e-7);
    eprintln!("60s 6-bus decode+duck+stereo write: {:.4}s,2880000frames,23040000bytes,128KiB PCM arrays plus bounded IO buffers",elapsed.as_secs_f64());
}
