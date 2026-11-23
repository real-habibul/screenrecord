use super::*;
use crate::assembly::{assemble_recording, RecordingPart};

fn parts_fixture(ffmpeg: &Path, directory: &Path) -> Vec<RecordingPart> {
    let encoder = process::software_encoder(
        ffmpeg,
        process::VideoSpec {
            width: 320,
            height: 180,
            fps: 30,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    [(1.017, "red"), (0.611, "green"), (0.413, "blue")]
        .into_iter()
        .enumerate()
        .map(|(index, (duration, color))| {
            let folder = directory.join(format!("part-{index}"));
            std::fs::create_dir(&folder).unwrap();
            let screen = folder.join("screen.mp4");
            run(media_command(ffmpeg)
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("color=c={color}:s=320x180:r=30:d={}", duration + 0.1),
                ])
                .args(encoder.args())
                .arg(&screen));
            let microphone = if index != 1 {
                Some(folder.join("microphone.wav"))
            } else {
                None
            };
            let system = if index == 1 {
                Some(folder.join("system.wav"))
            } else {
                None
            };
            for path in [microphone.as_ref(), system.as_ref()].into_iter().flatten() {
                run(media_command(ffmpeg)
                    .args([
                        "-f",
                        "lavfi",
                        "-i",
                        &format!(
                            "aevalsrc='if(lt(t,{duration}),{},0.8)':sample_rate=48000:duration={}",
                            (index + 1) as f64 * 0.1,
                            duration + 0.1
                        ),
                        "-c:a",
                        "pcm_f32le",
                    ])
                    .arg(path));
            }
            let camera = if index != 1 {
                let path = folder.join("camera.mp4");
                let color = if index == 0 { "blue" } else { "white" };
                let duration = if index == 0 { 0.7 } else { 0.6 };
                let mut command = media_command(ffmpeg);
                command.args([
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("color=c={color}:s=160x120:r=30:d={duration}"),
                ]);
                if index == 2 {
                    command.args([
                        "-copyts",
                        "-vf",
                        "setpts=PTS+0.3/TB",
                        "-fps_mode",
                        "passthrough",
                    ]);
                }
                run(command.args(encoder.args()).arg(&path));
                Some(path)
            } else {
                None
            };
            RecordingPart {
                screen_path: screen,
                microphone_path: microphone,
                system_audio_path: system,
                camera_path: camera,
                camera_offset_seconds: if index == 0 { 0.2 } else { 0.1 },
                width: 320,
                height: 180,
                fps: 30,
                duration,
            }
        })
        .collect()
}

fn pcm(ffmpeg: &Path, path: &Path) -> Vec<f32> {
    run(media_command(ffmpeg).arg("-i").arg(path).args([
        "-map", "0:a:0", "-f", "f32le", "-ar", "48000", "-ac", "2", "-",
    ]))
    .as_chunks::<4>()
    .0
    .iter()
    .map(|sample| f32::from_le_bytes(*sample))
    .collect()
}

fn colors(ffmpeg: &Path, path: &Path) -> Vec<u8> {
    run(media_command(ffmpeg).arg("-i").arg(path).args([
        "-map",
        "0:v:0",
        "-vf",
        "scale=1:1",
        "-pix_fmt",
        "rgb24",
        "-f",
        "rawvideo",
        "-",
    ]))
}

fn packet_hashes(ffmpeg: &Path, path: &Path) -> Vec<String> {
    let bytes = run(media_command(ffmpeg).arg("-i").arg(path).args([
        "-map",
        "0:v:0",
        "-c:v",
        "copy",
        "-bsf:v",
        "filter_units=remove_types=6|7|8|9",
        "-f",
        "framehash",
        "-hash",
        "sha256",
        "-",
    ]));
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            line.rsplit_once(',')
                .map(|(_, hash)| hash.trim().to_string())
        })
        .collect()
}

#[test]
fn compatible_screen_parts_are_copied_without_reencoding_and_retry_publishes_separately() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for lossless assembly test.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("screen.mp4");
    let encoder = process::software_encoder(
        &ffmpeg,
        process::VideoSpec {
            width: 320,
            height: 180,
            fps: 30,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    run(media_command(&ffmpeg)
        .args(["-f", "lavfi", "-i", "testsrc2=s=320x180:r=30:d=1"])
        .args(encoder.args())
        .args(["-bf", "0"])
        .arg(&source));
    let part = RecordingPart {
        screen_path: source.clone(),
        microphone_path: None,
        system_audio_path: None,
        camera_path: None,
        camera_offset_seconds: 0.0,
        width: 320,
        height: 180,
        fps: 30,
        duration: 1.0,
    };
    let expected = packet_hashes(&ffmpeg, &source);
    assert_eq!(expected.len(), 30);
    let mut hashes = Vec::new();
    for _ in 0..2 {
        hashes.extend(expected.clone());
    }
    let first = assemble_recording(
        &[part.clone(), part.clone()],
        directory.path(),
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    assert_eq!(packet_hashes(&ffmpeg, &first.screen_path), hashes);
    let original = std::fs::read(&first.screen_path).unwrap();
    let retry = assemble_recording(
        &[part.clone(), part],
        directory.path(),
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    assert_ne!(retry.screen_path, first.screen_path);
    assert_eq!(std::fs::read(&first.screen_path).unwrap(), original);
    assert!(
        first.camera_path.is_none()
            && first.microphone_path.is_none()
            && first.system_audio_path.is_none()
    );
}

#[test]
#[ignore = "Measures 60-second recording finalization; run explicitly with SCREENRECORD_FFMPEG."]
fn measure_sixty_second_pause_assembly() {
    measure_assembly(true);
}

#[test]
#[ignore = "Measures fractional-part recording finalization; run explicitly with SCREENRECORD_FFMPEG."]
fn measure_sixty_second_fractional_pause_assembly() {
    measure_assembly(false);
}

fn measure_assembly(copy: bool) {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for assembly timing.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let encoder = process::choose_encoder(
        &ffmpeg,
        process::VideoSpec {
            width: 1920,
            height: 1080,
            fps: 60,
        },
        &cancel,
    )
    .unwrap();
    let screen = directory.path().join("screen.mp4");
    let camera = directory.path().join("camera.mp4");
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            if copy {
                "testsrc2=s=1920x1080:r=60:d=30"
            } else {
                "testsrc2=s=1920x1080:r=60:d=30.1"
            },
        ])
        .args(encoder.args())
        .args(["-bf", "0"])
        .arg(&screen));
    run(media_command(&ffmpeg)
        .args(["-f", "lavfi", "-i", "testsrc2=s=1280x720:r=30:d=30.5"])
        .args(encoder.args())
        .arg(&camera));
    let microphone = directory.path().join("microphone.wav");
    let system = directory.path().join("system.wav");
    for (path, frequency) in [(&microphone, 997), (&system, 431)] {
        run(media_command(&ffmpeg)
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency={frequency}:sample_rate=48000:duration=30.1"),
                "-c:a",
                "pcm_f32le",
            ])
            .arg(path));
    }
    let first = RecordingPart {
        screen_path: screen.clone(),
        microphone_path: Some(microphone),
        system_audio_path: Some(system),
        camera_path: Some(camera),
        camera_offset_seconds: 0.15,
        width: 1920,
        height: 1080,
        fps: 60,
        duration: if copy { 30.0 } else { 29.991 },
    };
    let mut second = first.clone();
    second.camera_offset_seconds = 0.25;
    second.duration = if copy { 30.0 } else { 30.009 };
    let started = Instant::now();
    let result =
        assemble_recording(&[first, second], directory.path(), &ffmpeg, cancel, |_| {}).unwrap();
    let elapsed = started.elapsed();
    let actual = packet_hashes(&ffmpeg, &result.screen_path);
    assert_eq!(actual.len(), 3600);
    if copy {
        let hashes = packet_hashes(&ffmpeg, &screen);
        let mut expected = hashes.clone();
        expected.extend(hashes);
        assert!(
            actual == expected,
            "CopiedVCL packet payload mismatch: counts {} vs {}, firstdifferent {:?}",
            actual.len(),
            expected.len(),
            actual.iter().zip(&expected).position(|(a, b)| a != b)
        );
    }
    for path in [Some(&result.screen_path), result.camera_path.as_ref()]
        .into_iter()
        .flatten()
    {
        run(media_command(&ffmpeg)
            .arg("-i")
            .arg(path)
            .args(["-f", "null", "-"]));
    }
    assert_eq!(
        pcm(&ffmpeg, result.microphone_path.as_ref().unwrap()).len(),
        2_880_000 * 2
    );
    assert_eq!(
        pcm(&ffmpeg, result.system_audio_path.as_ref().unwrap()).len(),
        2_880_000 * 2
    );
    eprintln!("60s pause assembly:1920x1080@60 screen {}3600frames,1280x720 camera aligned/reencoded,2stereoPCMtracks;encoder{encoder:?};wall{:.4}s;full decode passed",if copy {"copied"}else{"normalized"},elapsed.as_secs_f64());
}

#[test]
fn pause_parts_assemble_on_cumulative_grids_preserve_originals_and_hide_camera_gaps() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for pause assembly media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let parts = parts_fixture(&ffmpeg, directory.path());
    let original_paths: Vec<_> = parts
        .iter()
        .flat_map(|part| {
            [
                Some(&part.screen_path),
                part.microphone_path.as_ref(),
                part.system_audio_path.as_ref(),
                part.camera_path.as_ref(),
            ]
            .into_iter()
            .flatten()
        })
        .collect();
    let originals: Vec<_> = original_paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    let result = assemble_recording(
        &parts,
        directory.path(),
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    assert!((result.duration - 2.041).abs() < 1e-12);
    assert_eq!(result.part_offsets, vec![0.0, 1.017, 1.628]);
    assert_eq!(result.camera_offset_seconds, 0.0);
    let screen = colors(&ffmpeg, &result.screen_path);
    assert_eq!(screen.len(), 62 * 3);
    assert!(screen[0] > 240 && screen[1] < 10);
    assert!(screen[31 * 3 + 1] > 100 && screen[31 * 3] < 10);
    assert!(screen[49 * 3 + 2] > 240 && screen[49 * 3] < 10);
    let camera = colors(&ffmpeg, result.camera_path.as_ref().unwrap());
    assert_eq!(camera.len(), 62 * 3);
    assert!(camera[2 * 3 + 2] > 240);
    assert!(camera[20 * 3..20 * 3 + 3].iter().all(|value| *value < 10));
    assert!(camera[38 * 3..38 * 3 + 3].iter().all(|value| *value < 10));
    assert!(camera[55 * 3..55 * 3 + 3].iter().all(|value| *value > 240));
    let intervals = result.camera_intervals.as_ref().unwrap();
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals[0].start, 0.0);
    assert!((intervals[0].end - 0.5).abs() < 1e-6);
    assert!((intervals[1].start - 1.828).abs() < 1e-12);
    assert!((intervals[1].end - 2.041).abs() < 1e-12);
    for (system, path) in [
        (false, result.microphone_path.as_ref().unwrap()),
        (true, result.system_audio_path.as_ref().unwrap()),
    ] {
        let values = pcm(&ffmpeg, path);
        assert_eq!(values.len(), 97_968 * 2);
        for (index, part) in parts.iter().enumerate() {
            let first = timeline::unit_at(result.part_offsets[index], 48_000) as usize;
            let last =
                timeline::unit_at(result.part_offsets[index] + part.duration, 48_000) as usize;
            let present = if system {
                part.system_audio_path.is_some()
            } else {
                part.microphone_path.is_some()
            };
            let shift = first as f64 / 48_000.0 - result.part_offsets[index];
            let available = audio::native_sample_at(part.duration, 48_000)
                - audio::native_sample_at(shift, 48_000);
            for frame in first..last {
                let expected = if present && ((frame - first) as u64) < available {
                    (index + 1) as f32 * 0.1
                } else {
                    0.0
                };
                for actual in &values[frame * 2..frame * 2 + 2] {
                    assert!(
                        (actual - expected).abs() < 1e-7,
                        "part {index} frame {frame}: {actual} vs {expected}"
                    );
                }
            }
        }
    }
    let mut project = Project {
        version: screenrecord_core::PROJECT_VERSION,
        id: "26bf72de-b0ae-4a96-999a-2dacaa468daf".into(),
        name: "Paused media".into(),
        created_at: "2026-11-11T00:00:00Z".into(),
        directory: directory.path().to_string_lossy().into_owned(),
        screen_path: result.screen_path.to_string_lossy().into_owned(),
        microphone_path: None,
        system_audio_path: None,
        camera_path: None,
        camera_offset_seconds: 0.0,
        camera_intervals: None,
        width: 320,
        height: 180,
        fps: 30,
        duration: result.duration,
        cursor: Vec::new(),
        edits: EditSettings::new(result.duration),
        status: "ready".into(),
    };
    project.directory = directory.path().to_string_lossy().into_owned();
    project.screen_path = result.screen_path.to_string_lossy().into_owned();
    project.microphone_path = None;
    project.system_audio_path = None;
    project.camera_path = result
        .camera_path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    project.camera_offset_seconds = 0.0;
    project.camera_intervals = result.camera_intervals.clone();
    project.duration = result.duration;
    project.cursor.clear();
    project.edits = EditSettings::new(result.duration);
    project.edits.padding = 0;
    project.edits.corner_radius = 0;
    project.edits.show_cursor = false;
    project.edits.camera = CameraSettings {
        layout: "bubble".into(),
        enabled: true,
        x: 0.6,
        y: 0.6,
        width: 0.25,
        shape: "rectangle".into(),
        mirror: false,
    };
    let output = directory.path().join("gap-export.mp4");
    let options = ExportOptions {
        preset: screenrecord_core::ExportPreset::Standard,
        output_path: output.to_string_lossy().into_owned(),
        resolution: "1080p".into(),
        fps: 30,
    };
    export(
        &project,
        &options,
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let pixel = |time: &str| {
        run(media_command(&ffmpeg)
            .args(["-ss", time, "-i"])
            .arg(&output)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "crop=2:2:1400:750,scale=1:1",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "-",
            ]))
    };
    let present = pixel("0.2");
    let hidden = pixel("0.7");
    let resumed = pixel("1.85");
    let startup_gap = pixel("1.7");
    assert!(present[2] > 220);
    assert!(hidden[0] > 220 && hidden[2] < 20);
    assert!(resumed.iter().all(|value| *value > 220));
    assert!(startup_gap[2] > 220 && startup_gap[0] < 20);
    for (path, before) in original_paths.iter().zip(originals) {
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
    eprintln!("Pause assembly:3parts,62frames,97968stereoPCMframes,offsetcamera+gap hide verified through finalMP4;all originalbytes unchanged.");
}

#[test]
fn assembly_cancellation_mismatch_and_existing_publication_are_safe() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for assembly failure tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut parts = parts_fixture(&ffmpeg, directory.path());
    let sentinel = directory.path().join("assembled-existing");
    std::fs::create_dir(&sentinel).unwrap();
    std::fs::write(sentinel.join("screen.mp4"), b"existing user media").unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    assert!(
        assemble_recording(&parts, directory.path(), &ffmpeg, cancel, move |value| {
            if value > 0.01 {
                signal.store(true, Ordering::Relaxed)
            }
        })
        .is_err()
    );
    assert!(!std::fs::read_dir(directory.path())
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".screenrecord-assembly-")));
    assert_eq!(
        std::fs::read(sentinel.join("screen.mp4")).unwrap(),
        b"existing user media"
    );
    parts[1].width = 640;
    assert!(assemble_recording(
        &parts,
        directory.path(),
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {}
    )
    .unwrap_err()
    .contains("changed while paused"));
}
