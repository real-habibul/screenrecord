use super::*;
use screenrecord_core::{CameraInterval, MusicClip, Redaction, SubtitleCue, VideoClip};

fn clips(ranges: &[(f64, f64)]) -> Vec<VideoClip> {
    ranges
        .iter()
        .enumerate()
        .map(|(index, (start, end))| VideoClip {
            id: format!("clip-{index}"),
            start: *start,
            end: *end,
        })
        .collect()
}

fn hash_file(ffmpeg: &Path, path: &Path) -> String {
    String::from_utf8(run(media_command(ffmpeg)
        .args(["-f", "data", "-i"])
        .arg(path)
        .args([
            "-map", "0", "-c", "copy", "-f", "hash", "-hash", "SHA256", "-",
        ])))
    .unwrap()
}

#[test]
fn irregular_camera_timestamps_keep_offset_cut_gaps_and_audio_without_stretching() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for cut media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let camera = directory.path().join("irregular-camera.mp4");
    let timestamps = [0, 73, 191, 410, 733, 997, 1051, 1373, 1777, 1820];
    let select = timestamps
        .iter()
        .map(|time| format!("eq(n,{time})"))
        .collect::<Vec<_>>()
        .join("+");
    run(media_command(&ffmpeg)
        .args([
            "-f", "lavfi", "-i",
            &format!("color=c=blue:s=160x120:r=1000:d=1.821,drawbox=color=green:t=fill:enable='gte(n,733)',drawbox=color=white:t=fill:enable='gte(n,1373)',drawbox=color=yellow:t=fill:enable='gte(n,1777)',select='{select}'"),
            "-fps_mode", "vfr", "-enc_time_base", "1:1000", "-c:v", "mpeg4", "-q:v", "2",
        ])
        .arg(&camera));
    let packet_bytes = run(Command::new(ffmpeg.with_file_name("ffprobe.exe"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_packets",
            "-show_entries",
            "packet=pts_time,duration_time",
            "-of",
            "json",
        ])
        .arg(&camera));
    let packet_info: serde_json::Value = serde_json::from_slice(&packet_bytes).unwrap();
    let packets = packet_info["packets"].as_array().unwrap();
    assert_eq!(packets.len(), timestamps.len());
    for (packet, timestamp) in packets.iter().zip(timestamps) {
        let actual = packet["pts_time"].as_str().unwrap().parse::<f64>().unwrap();
        assert!((actual - timestamp as f64 / 1000.0).abs() < 0.000001);
    }
    let camera_info = probe_media(&ffmpeg, &camera).unwrap();
    let camera_end = camera_info
        .video()
        .unwrap()
        .duration
        .as_ref()
        .unwrap()
        .parse::<f64>()
        .unwrap();
    assert!((camera_end - 1.821).abs() < 0.000001);
    project.camera_path = Some(camera.to_string_lossy().into_owned());
    project.camera_offset_seconds = 0.137;
    project.camera_intervals = Some(vec![
        CameraInterval {
            start: 0.0,
            end: 0.31,
        },
        CameraInterval {
            start: 0.44,
            end: project.duration,
        },
    ]);
    project.edits = EditSettings::new(project.duration);
    project.edits.background = "#102030".into();
    project.edits.padding = 0;
    project.edits.corner_radius = 0;
    project.edits.show_cursor = false;
    project.edits.camera.layout = "fullscreen".into();
    project.edits.video_clips = clips(&[(0.111, 0.707), (1.011, 2.321)]);
    let originals: Vec<_> = [
        project.screen_path.as_str(),
        project.camera_path.as_ref().unwrap(),
        project.microphone_path.as_ref().unwrap(),
        project.system_audio_path.as_ref().unwrap(),
    ]
    .into_iter()
    .map(|path| (path.to_owned(), fs::read(path).unwrap()))
    .collect();
    let mut output = options(directory.path(), "irregular-camera-cut.mp4");
    output.fps = 30;
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    let cancel = Arc::new(AtomicBool::new(false));
    export_inner(&project, &output, &ffmpeg, &cancel, &callback, 7).unwrap();
    let pixels = video_pixels(&ffmpeg, &output.output_path);
    let frame_bytes = 96 * 54 * 3;
    assert_eq!(pixels.len(), 58 * frame_bytes);
    for (index, frame) in pixels.chunks_exact(frame_bytes).enumerate() {
        let output_time = index as f64 / 30.0;
        let source_time = if output_time < 0.596 {
            0.111 + output_time
        } else {
            1.011 + output_time - 0.596
        };
        let camera_time = source_time + 0.137;
        let expected = if (0.31..0.44).contains(&source_time) || camera_time >= camera_end {
            [16u8, 32, 48]
        } else if camera_time < 0.733 {
            [0, 0, 255]
        } else if camera_time < 1.373 {
            [0, 128, 0]
        } else if camera_time < 1.777 {
            [255, 255, 255]
        } else {
            [255, 255, 0]
        };
        let actual = &frame[(27 * 96 + 48) * 3..][..3];
        assert!(actual.iter().zip(expected).all(|(value, expected)| value.abs_diff(expected) < 12),
            "Camera clock shifted at frame {index}, source={source_time:.9}, camera={camera_time:.9}: {actual:?} expected {expected:?}");
    }
    project.edits.camera.enabled = false;
    let mut without_camera = options(directory.path(), "same-audio-without-camera.mp4");
    without_camera.fps = 30;
    export_inner(&project, &without_camera, &ffmpeg, &cancel, &callback, 7).unwrap();
    assert_eq!(
        audio_samples(&ffmpeg, &output.output_path),
        audio_samples(&ffmpeg, &without_camera.output_path)
    );
    for (path, original) in originals {
        assert_eq!(fs::read(&path).unwrap(), original, "Source changed: {path}");
    }
    eprintln!("Irregular camera clock: 10 exact source PTS, 137ms offset, 2 fractional kept ranges, 58 output frames, declared gap and early EOF hidden, decoded AAC identical with/without camera, source bytes unchanged.");
}

#[test]
fn cut_media_preserves_source_effects_camera_music_and_omits_removed_caption_text() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for cut media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.camera_offset_seconds = 0.1;
    let music = directory.path().join("cut music.wav");
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.12:sample_rate=48000:duration=3",
            "-c:a",
            "pcm_f32le",
        ])
        .arg(&music));
    project.edits.music.push(MusicClip {
        id: "cut-music".into(),
        name: "Cut music".into(),
        path: music.to_string_lossy().into_owned(),
        source_duration: 3.0,
        start: 0.4,
        source_start: 0.15,
        duration: 2.0,
        volume: 0.5,
        fade_in: 0.5,
        fade_out: 0.5,
    });
    project.edits.redactions.push(Redaction {
        tracking: None,
        id: "source-mask".into(),
        start: 1.3,
        end: 2.1,
        x: 0.2,
        y: 0.3,
        width: 0.15,
        height: 0.12,
        mode: "solid".into(),
        strength: 12.0,
    });
    project.edits.subtitles = vec![
        SubtitleCue {
            words: vec![],
            source_ranges: vec![],
            id: "first".into(),
            start: 0.3,
            end: 0.7,
            text: "Kept first".into(),
        },
        SubtitleCue {
            words: vec![],
            source_ranges: vec![],
            id: "last".into(),
            start: 1.95,
            end: 2.4,
            text: "Kept last".into(),
        },
    ];
    let originals: Vec<_> = [
        project.screen_path.as_str(),
        project.camera_path.as_ref().unwrap(),
        project.microphone_path.as_ref().unwrap(),
        project.system_audio_path.as_ref().unwrap(),
        project.edits.music[0].path.as_str(),
    ]
    .into_iter()
    .map(|path| (PathBuf::from(path), hash_file(&ffmpeg, Path::new(path))))
    .collect();
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    let cancel = Arc::new(AtomicBool::new(false));
    let baseline = options(directory.path(), "baseline.mp4");
    export_inner(&project, &baseline, &ffmpeg, &cancel, &callback, 900).unwrap();
    let mut split = project.clone();
    split.edits.video_clips = clips(&[(0.21, 1.411), (1.411, 2.58)]);
    assert_eq!(timeline::merged(&split).len(), 1);
    let split_output = options(directory.path(), "split.mp4");
    export_inner(&split, &split_output, &ffmpeg, &cancel, &callback, 900).unwrap();
    assert_eq!(timeline::merged(&project), timeline::merged(&split));
    let baseline_pixels = video_pixels(&ffmpeg, &baseline.output_path);
    let split_pixels = video_pixels(&ffmpeg, &split_output.output_path);
    assert_eq!(baseline_pixels.len(), split_pixels.len());
    let split_error = baseline_pixels
        .iter()
        .zip(&split_pixels)
        .map(|(a, b)| (*a as f64 - *b as f64).abs())
        .sum::<f64>()
        / baseline_pixels.len() as f64;
    assert!(split_error<0.5,"Adjacent split changed decoded frames beyond hardware encoding variation: MAE={split_error}");
    assert!(
        audio_samples(&ffmpeg, &baseline.output_path)
            == audio_samples(&ffmpeg, &split_output.output_path),
        "Adjacent Split reset audio processing."
    );
    let mut cut = project.clone();
    cut.edits.video_clips = clips(&[(0.21, 0.811), (1.311, 1.414), (1.914, 2.58)]);
    cut.edits.subtitles.push(SubtitleCue {
        words: vec![],
        source_ranges: vec![],
        id: "removed-words".into(),
        start: 0.75,
        end: 1.4,
        text: "REMOVED PRIVATE WORDS".into(),
    });
    cut.edits
        .subtitles
        .sort_by(|a, b| a.start.total_cmp(&b.start));
    cut.validate().unwrap();
    let cut_output = options(directory.path(), "cut.mp4");
    export_inner(&cut, &cut_output, &ffmpeg, &cancel, &callback, 12).unwrap();
    let full = video_pixels(&ffmpeg, &baseline.output_path);
    let edited = video_pixels(&ffmpeg, &cut_output.output_path);
    let frame_bytes = 96 * 54 * 3;
    assert_eq!(edited.len() / frame_bytes, 83);
    let mut maximum_error = 0.0_f64;
    for (frame, output) in edited.chunks_exact(frame_bytes).enumerate() {
        let reference_index = frame
            + if frame < 37 {
                0
            } else if frame < 43 {
                30
            } else {
                60
            };
        let reference = &full[reference_index * frame_bytes..(reference_index + 1) * frame_bytes];
        let error = output
            .iter()
            .zip(reference)
            .map(|(a, b)| (*a as f64 - *b as f64).abs())
            .sum::<f64>()
            / frame_bytes as f64;
        maximum_error = maximum_error.max(error);
        assert!(
            error < 3.0,
            "Output frame {frame} must match original frame {reference_index}, RGB MAE={error}"
        );
        let caption_error = output[36 * 96 * 3..]
            .iter()
            .zip(&reference[36 * 96 * 3..])
            .map(|(a, b)| (*a as f64 - *b as f64).abs())
            .sum::<f64>()
            / (18 * 96 * 3) as f64;
        assert!(
            caption_error < 3.0,
            "Caption changed or leaked at output frame {frame}: {caption_error}"
        );
    }
    let layout = math::layout(&cut, "1080p").unwrap();
    assert!(subtitles::write_chunk(&cut, &layout, directory.path(), 0.21, 37, 60).unwrap());
    let captions = fs::read_to_string(directory.path().join("subtitles.ass")).unwrap();
    assert!(
        captions.contains("Kept first") && !captions.contains("REMOVED PRIVATE WORDS"),
        "A caption crossing removed time leaked into export captions."
    );
    let glyph = subtitles::subtitle_preview(&cut, "removed-words", None, &ffmpeg, &cancel).unwrap();
    let mut unsplit_caption = cut.clone();
    unsplit_caption.edits.video_clips.clear();
    assert!(
        glyph
            == subtitles::subtitle_preview(
                &unsplit_caption,
                "removed-words",
                None,
                &ffmpeg,
                &cancel
            )
            .unwrap(),
        "Glyph cache raster must not depend on saved EDL or cue visibility."
    );
    let graph =
        graph::RenderGraph::prepare(&cut, &cut_output, &ffmpeg, directory.path(), &cancel).unwrap();
    assert_eq!(graph.frame_count, 83);
    assert_eq!(
        graph
            .audio_chunks()
            .iter()
            .map(|chunk| chunk.samples)
            .sum::<u64>(),
        65_760
    );
    let tracks = audio::prepare(&project, &ffmpeg, &cancel).unwrap();
    let mut command = media_command(&ffmpeg);
    command.args(["-f", "lavfi", "-i", "color=s=16x16:r=1:d=3"]);
    let filters = audio::append_inputs(&mut command, &tracks, 2.37);
    let pcm = run(command.args([
        "-filter_complex",
        &filters.join(";"),
        "-map",
        "[outa]",
        "-c:a",
        "pcm_f32le",
        "-f",
        "f32le",
        "-",
    ]));
    let samples = pcm.as_chunks::<8>().0;
    let expected: Vec<_> = [(0, 28_848), (52_848, 57_792), (81_792, 113_760)]
        .into_iter()
        .flat_map(|(start, end)| samples[start..end].iter().copied())
        .collect();
    let encoded = audio_samples(&ffmpeg, &cut_output.output_path);
    let actual = encoded.as_chunks::<8>().0;
    assert!(
        actual.len() >= expected.len() && actual.len() - expected.len() < 1024,
        "Unexpected AAC padding or repeated delay at cuts."
    );
    let rmse = (expected
        .iter()
        .zip(actual)
        .map(|(a, b)| {
            let x = f32::from_le_bytes(a[..4].try_into().unwrap()) as f64;
            let y = f32::from_le_bytes(b[..4].try_into().unwrap()) as f64;
            (x - y) * (x - y)
        })
        .sum::<f64>()
        / expected.len() as f64)
        .sqrt();
    assert!(
        rmse < 0.005,
        "Audio source timing/gain changed across cuts, RMSE={rmse}"
    );
    let info = probe_media(&ffmpeg, Path::new(&cut_output.output_path)).unwrap();
    validate_output(&info, &layout, 83, 60, 1.37, true).unwrap();
    run(media_command(&ffmpeg).args(["-i", &cut_output.output_path, "-f", "null", "-"]));
    for (path, hash) in originals {
        assert_eq!(
            hash_file(&ffmpeg, &path),
            hash,
            "Original changed: {}",
            path.display()
        );
    }
    eprintln!("EDL: adjacent split same source mapping, pixel MAE={split_error:.4}, AAC identical; 3 kept spans,83 frames,65760 audio samples; max RGB MAE={maximum_error:.4},audio RMSE={rmse:.6};all5 original SHA256 hashes unchanged.");
}

#[test]
fn cancellation_after_cut_audio_preparation_preserves_existing_output_and_removes_temporary_media()
{
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.edits.video_clips = clips(&[(0.21, 0.6), (1.4, 1.7)]);
    let existing = directory.path().join("existing.mp4");
    fs::write(&existing, b"existing export").unwrap();
    let options = options(directory.path(), "cancelled.mp4");
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let callback: process::Progress =
        Arc::new(Mutex::new(Box::new(move |event: ExportProgress| {
            if event.progress > 0.8001 {
                trigger.store(true, Ordering::Release);
            }
        })));
    let error = export_inner(&project, &options, &ffmpeg, &cancel, &callback, 900).unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(!Path::new(&options.output_path).exists());
    assert_eq!(fs::read(&existing).unwrap(), b"existing export");
    assert!(!fs::read_dir(directory.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".screenrecord-export-")));
}

#[test]
fn fractional_cut_never_samples_future_deleted_screen_or_camera_frames() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let screen = directory.path().join("sentinel-screen.mp4");
    let camera = directory.path().join("sentinel-camera.mp4");
    for (path, red_frame) in [(&screen, 64), (&camera, 67)] {
        run(media_command(&ffmpeg).args(["-f","lavfi","-i",&format!("color=c=blue:s=320x180:r=30:d=2.8,drawbox=color=red:t=fill:enable='gte(n,{red_frame})'"),"-c:v","mpeg4","-q:v","2"]).arg(path));
    }
    project.screen_path = screen.to_string_lossy().into_owned();
    project.camera_path = Some(camera.to_string_lossy().into_owned());
    project.camera_offset_seconds = 0.1;
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits.trim_start = 0.0;
    project.edits.trim_end = 2.8;
    project.edits.crop = None;
    project.edits.zooms.clear();
    project.edits.show_cursor = false;
    project.edits.padding = 0;
    project.edits.corner_radius = 0;
    project.edits.camera = CameraSettings {
        layout: "bubble".into(),
        enabled: true,
        x: 0.7,
        y: 0.7,
        width: 0.2,
        shape: "rectangle".into(),
        mirror: false,
    };
    project.edits.video_clips = clips(&[(2.017, 2.120)]);
    let mut output = options(directory.path(), "sentinel-kept.mp4");
    output.fps = 30;
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    export_inner(
        &project,
        &output,
        &ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        900,
    )
    .unwrap();
    let pixels = video_pixels(&ffmpeg, &output.output_path);
    let frame_bytes = 96 * 54 * 3;
    assert_eq!(pixels.len() / frame_bytes, 4);
    for (frame, bytes) in pixels.chunks_exact(frame_bytes).enumerate() {
        for (x, y) in [(20, 20), (80, 44)] {
            let pixel = &bytes[(y * 96 + x) * 3..(y * 96 + x) * 3 + 3];
            assert!(
                pixel[2] > 180 && pixel[0] < 50,
                "Deleted sentinel leaked at frame {frame},position({x},{y}):{pixel:?}"
            );
        }
    }
    for (path, time) in [(&screen, "2.2"), (&camera, "2.3")] {
        let red = run(media_command(&ffmpeg)
            .args(["-ss", time, "-i"])
            .arg(path)
            .args([
                "-vf",
                "format=rgb24,crop=1:1:100:100:exact=1",
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-",
            ]));
        assert!(
            red[0] > 180 && red[2] < 50,
            "The independent deleted-frame sentinel was not red."
        );
    }
}

#[test]
fn fractional_audio_cuts_exclude_deleted_native_samples_before_resampling() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.system_audio_path = None;
    project.camera_path = None;
    project.edits.microphone_volume = 1.0;
    project.edits.trim_start = 0.0;
    project.edits.trim_end = 2.8;
    let ranges = [(0.201234, 0.452345), (1.201234, 1.452345)];
    project.edits.video_clips = clips(&ranges);
    let options = options(directory.path(), "unused-audio-test.mp4");
    let cancel = Arc::new(AtomicBool::new(false));
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    for rate in [44_100, 48_000] {
        let path = directory.path().join(format!("native-{rate}.wav"));
        let native: Vec<_> = ranges
            .iter()
            .map(|(start, end)| {
                (
                    timeline::unit_at(*start, rate),
                    timeline::unit_at(*end, rate),
                )
            })
            .collect();
        let source=format!("aevalsrc=if(between(n,{},{} )+between(n,{},{}),500/32768,30000/32768):sample_rate={rate}:duration=2.8",native[0].0,native[0].1-1,native[1].0,native[1].1-1).replace(',',"\\,");
        run(media_command(&ffmpeg)
            .args(["-f", "lavfi", "-i", &source, "-c:a", "pcm_s16le"])
            .arg(&path));
        project.microphone_path = Some(path.to_string_lossy().into_owned());
        let graph =
            graph::RenderGraph::prepare(&project, &options, &ffmpeg, directory.path(), &cancel)
                .unwrap();
        let chunks = graph.audio_chunks();
        assert_eq!(
            chunks.iter().map(|chunk| chunk.samples).sum::<u64>(),
            24_107
        );
        for chunk in chunks {
            process::encode(
                &mut graph.audio_command(&ffmpeg, chunk),
                &cancel,
                &callback,
                chunk.samples as f64 / 48_000.0,
                &options.output_path,
                0.0..1.0,
            )
            .unwrap();
            let bytes = audio_samples(
                &ffmpeg,
                &directory
                    .path()
                    .join(format!("audio-{:05}.wav", chunk.index))
                    .to_string_lossy(),
            );
            let samples: Vec<_> = bytes
                .as_chunks::<8>()
                .0
                .iter()
                .map(|sample| f32::from_le_bytes(sample[..4].try_into().unwrap()))
                .collect();
            assert_eq!(samples.len() as u64, chunk.samples);
            assert!(
                (samples[0] - 500.0 / 32768.0).abs() < 0.0001,
                "Native {rate} clip{} first sample leaked: {}",
                chunk.index,
                samples[0]
            );
            assert!(
                samples
                    .iter()
                    .all(|value| value.is_finite() && *value < 0.016 && *value >= -0.00001),
                "Deleted native {rate} sentinel contaminated a retained interval or its resampler."
            );
        }
    }
}
