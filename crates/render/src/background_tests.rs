use super::*;
use screenrecord_core::{
    CameraInterval, MaskKeyframe, MaskTracking, Redaction, ReframeKeyframe, VideoClip,
};

fn render(ffmpeg: &Path, project: &Project, name: &str, chunk_frames: u64) -> ExportOptions {
    let mut output = options(Path::new(&project.directory), name);
    output.fps = 30;
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    export_inner(
        project,
        &output,
        ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        chunk_frames,
    )
    .unwrap();
    output
}

fn assert_color(actual: &[u8], expected: [u8; 3], context: &str) {
    assert!(
        actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) < 12),
        "{context}: {actual:?} != {expected:?}"
    );
}

#[test]
fn encoded_rgb_colors_match_bt709_tags_on_hardware_and_software_paths() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let source = directory.path().join("rgb-bars.mkv");
    let colors = [
        ("red", [255, 0, 0]),
        ("lime", [0, 255, 0]),
        ("blue", [0, 0, 255]),
        ("cyan", [0, 255, 255]),
        ("magenta", [255, 0, 255]),
        ("yellow", [255, 255, 0]),
        ("white", [255, 255, 255]),
        ("0x808080", [128, 128, 128]),
        ("black", [0, 0, 0]),
    ];
    let mut filter = "nullsrc=s=360x180:r=30:d=2.8,format=rgba".to_owned();
    for (index, (name, _)) in colors.iter().enumerate() {
        filter.push_str(&format!(
            ",drawbox=x={}:y=0:w=40:h=180:c={name}:t=fill",
            index * 40
        ));
    }
    run(media_command(&ffmpeg)
        .args([
            "-f", "lavfi", "-i", &filter, "-c:v", "ffv1", "-pix_fmt", "gbrp",
        ])
        .arg(&source));
    project.screen_path = source.to_string_lossy().into_owned();
    project.width = 360;
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits = EditSettings::new(project.duration);
    project.edits.trim_end = 0.2;
    project.edits.camera.enabled = false;
    project.edits.show_cursor = false;
    project.edits.corner_radius = 0;
    project.edits.padding = 0;
    let mut options = options(directory.path(), "colors.mp4");
    options.fps = 30;
    let spec = process::VideoSpec {
        width: 2160,
        height: 1080,
        fps: 30,
    };
    let cancel = AtomicBool::new(false);
    let mut encoders = vec![
        process::choose_encoder(&ffmpeg, spec, &cancel).unwrap(),
        process::software_encoder(&ffmpeg, spec, &cancel).unwrap(),
    ];
    let mut amd_probe = media_command(&ffmpeg);
    amd_probe
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=2160x1080:r=30:d=0.1",
            "-frames:v",
            "3",
            "-an",
        ])
        .args(process::Encoder::Amd.args())
        .args(["-f", "null", "-"]);
    if process::capture(&mut amd_probe, Duration::from_secs(8), &cancel)
        .is_ok_and(|(success, _, _)| success)
        && !encoders.contains(&process::Encoder::Amd)
    {
        encoders.push(process::Encoder::Amd);
    }
    encoders.dedup();
    for encoder in encoders {
        let work = directory.path().join(format!("color-{encoder:?}"));
        fs::create_dir(&work).unwrap();
        let graph =
            graph::RenderGraph::prepare(&project, &options, &ffmpeg, &work, &cancel).unwrap();
        let chunks = graph.chunks(900).unwrap();
        assert_eq!(chunks.len(), 1);
        run(&mut graph
            .video_command(&project, &options, &ffmpeg, encoder, chunks[0], &cancel)
            .unwrap());
        run(&mut graph
            .mux_command(&project, &options, &ffmpeg, &chunks)
            .unwrap());
        let output = work.join("encoded.mp4");
        let info: serde_json::Value =
            serde_json::from_slice(&run(Command::new(ffprobe_path(&ffmpeg))
                .args(["-v", "error", "-show_streams", "-of", "json"])
                .arg(&output)))
            .unwrap();
        let stream = &info["streams"][0];
        for field in ["color_space", "color_transfer", "color_primaries"] {
            assert_eq!(stream[field], "bt709", "{encoder:?} {field}");
        }
        assert_eq!(stream["color_range"], "tv", "{encoder:?}");
        let decoded = video_pixels(&ffmpeg, &output.to_string_lossy());
        assert_eq!(decoded.len(), 6 * 96 * 54 * 3);
        let mut worst = 0_u8;
        for frame in decoded.as_chunks::<{ 96 * 54 * 3 }>().0 {
            for (index, (_, expected)) in colors.iter().enumerate() {
                let x = ((index as f64 + 0.5) / colors.len() as f64 * 96.0).floor() as usize;
                let actual = &frame[(27 * 96 + x) * 3..][..3];
                for (actual, expected) in actual.iter().zip(expected) {
                    worst = worst.max(actual.abs_diff(*expected));
                }
                assert!(
                    actual
                        .iter()
                        .zip(expected)
                        .all(|(a, e)| a.abs_diff(*e) <= 5),
                    "{encoder:?} bar{index}: {actual:?} != {expected:?}"
                );
            }
        }
        eprintln!("{encoder:?}: decoded9 RGB/grayscale bars in6frames, maxchannelerror{worst}, BT709 matrix/primaries/transfer and limitedrange tags verified.");
    }
}

#[test]
fn video_background_advances_its_own_crop_on_every_frame_across_cuts_and_chunks() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let screen = directory.path().join("alternating-view.mkv");
    run(media_command(&ffmpeg).args([
        "-f", "lavfi", "-i",
        "color=c=yellow:s=320x180:r=30:d=2.8,format=rgba,drawbox=x=32:y=18:w=128:h=144:c=blue:t=fill,drawbox=x=160:y=18:w=128:h=144:c=cyan:t=fill,drawbox=x=0:y=0:w=320:h=180:c=red:t=fill:enable='between(t,0.8,1.2)'",
        "-c:v", "ffv1", "-pix_fmt", "gbrp",
    ]).arg(&screen));
    project.screen_path = screen.to_string_lossy().into_owned();
    let original = fs::read(&screen).unwrap();
    project.edits.background_mode = "videoBlur".into();
    project.edits.background_blur = 64.0;
    project.edits.aspect_ratio = "9:16".into();
    project.edits.fit_mode = "fit".into();
    project.edits.padding = 240;
    project.edits.corner_radius = 40;
    project.edits.camera.enabled = false;
    project.edits.zooms.clear();
    project.edits.show_cursor = true;
    project.cursor.iter_mut().for_each(|point| {
        point.x = 0.3;
        point.y = 0.5;
    });
    project.edits.video_clips = [(0.217, 0.733), (1.217, 1.733)]
        .into_iter()
        .enumerate()
        .map(|(index, (start, end))| VideoClip {
            id: format!("cut-{index}"),
            start,
            end,
        })
        .collect();
    project.edits.reframe_keyframes = (0..31)
        .map(|index| ReframeKeyframe {
            id: format!("frame-{index}"),
            time: if index < 16 {
                0.217 + index as f64 / 30.0
            } else {
                1.217 + index as f64 / 30.0 - 0.516
            },
            x: if index % 2 == 0 { 0.3 } else { 0.7 },
            y: 0.5,
            scale: 3.0,
        })
        .collect();
    let graph_directory = directory.path().join("command-contract");
    fs::create_dir(&graph_directory).unwrap();
    let mut graph_options = options(directory.path(), "command-contract.mp4");
    graph_options.fps = 30;
    let graph = graph::RenderGraph::prepare(
        &project,
        &graph_options,
        &ffmpeg,
        &graph_directory,
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut command_bytes = 0;
    for chunk in graph.chunks(7).unwrap() {
        let _command = graph
            .video_command(
                &project,
                &graph_options,
                &ffmpeg,
                process::Encoder::SoftwareMf,
                chunk,
                &AtomicBool::new(false),
            )
            .unwrap();
        let foreground = fs::read_to_string(graph_directory.join("motion.txt")).unwrap();
        let background = fs::read_to_string(graph_directory.join("background-motion.txt")).unwrap();
        assert!(!foreground.contains("crop@background"));
        assert!(!background.contains("crop@view") && !background.contains("overlay@pointer"));
        assert!(background.starts_with("0.000000 crop@background"));
        assert!(foreground.len() + background.len() <= 256 * 1024);
        command_bytes = command_bytes.max(foreground.len() + background.len());
    }
    let started = Instant::now();
    let whole = render(&ffmpeg, &project, "background-whole.mp4", 900);
    let chunked = render(&ffmpeg, &project, "background-chunked.mp4", 7);
    let mut decoded = Vec::new();
    for output in [&whole, &chunked] {
        let pixels = video_pixels(&ffmpeg, &output.output_path);
        assert_eq!(pixels.len(), 31 * 96 * 54 * 3);
        for (index, frame) in pixels.as_chunks::<{ 96 * 54 * 3 }>().0.iter().enumerate() {
            let expected = if index % 2 == 0 {
                [0, 0, 255]
            } else {
                [0, 255, 255]
            };
            for (x, y) in [
                (0, 0),
                (95, 0),
                (0, 53),
                (95, 53),
                (48, 2),
                (48, 51),
                (2, 27),
                (93, 27),
            ] {
                assert_color(
                    &frame[(y * 96 + x) * 3..][..3],
                    expected,
                    &format!("background frame {index}, point{x},{y}"),
                );
            }
        }
        decoded.push(pixels);
    }
    let error = decoded[0]
        .iter()
        .zip(&decoded[1])
        .map(|(a, b)| a.abs_diff(*b) as f64)
        .sum::<f64>()
        / decoded[0].len() as f64;
    assert!(error < 2.0, "Background chunk parity MAE={error}");
    assert_eq!(
        audio_samples(&ffmpeg, &whole.output_path),
        audio_samples(&ffmpeg, &chunked.output_path)
    );
    assert_eq!(fs::read(screen).unwrap(), original);
    eprintln!("Background alternating source-time ROI: 31 frames, 7/900-frame chunk MAE={error:.3}, identical decoded AAC.");
    eprintln!("Both background exports and verification: {:.3}s; max combined motion command bytes in a7-frame chunk: {command_bytes}; blur scratch540x960.",started.elapsed().as_secs_f64());
}

#[test]
fn video_background_keeps_one_pixel_and_odd_crop_edges_opaque_at_blur_limits() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits = EditSettings::new(project.duration);
    project.edits.trim_end = 0.1;
    project.edits.background = "#102030".into();
    project.edits.background_mode = "videoBlur".into();
    project.edits.aspect_ratio = "1:1".into();
    project.edits.fit_mode = "fill".into();
    project.edits.padding = 240;
    project.edits.camera.enabled = false;
    project.edits.show_cursor = false;
    for (index, (width, height, x, y, crop_width, crop_height, pixels)) in [
        (10, 100, 0.9, 0.0, 0.1, 1.0, (9, 0, 1, 100)),
        (100, 10, 0.0, 0.9, 1.0, 0.1, (0, 9, 100, 1)),
        (10, 10, 0.9, 0.9, 0.1, 0.1, (9, 9, 1, 1)),
        (321, 181, 0.17, 0.12, 0.61, 0.73, (54, 21, 196, 132)),
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!("color=c=yellow:s={width}x{height}:r=30:d=2.8,format=rgba,drawbox=x={}:y={}:w={}:h={}:c=blue:t=fill",pixels.0,pixels.1,pixels.2,pixels.3);
        let path = directory.path().join(format!("edge-{index}.mkv"));
        run(media_command(&ffmpeg)
            .args([
                "-f", "lavfi", "-i", &source, "-c:v", "ffv1", "-pix_fmt", "gbrp",
            ])
            .arg(&path));
        project.screen_path = path.to_string_lossy().into_owned();
        project.width = width;
        project.height = height;
        project.edits.crop = Some(CropSettings {
            x,
            y,
            width: crop_width,
            height: crop_height,
        });
        project.edits.reframe_keyframes = vec![ReframeKeyframe {
            id: "small-view".into(),
            time: 0.0,
            x: 1.0,
            y: 1.0,
            scale: 3.0,
        }];
        for strength in [4.0, 64.0] {
            project.edits.background_blur = strength;
            let output = render(
                &ffmpeg,
                &project,
                &format!("edge-{index}-{strength}.mp4"),
                7,
            );
            let decoded = video_pixels(&ffmpeg, &output.output_path);
            assert_eq!(decoded.len(), 3 * 96 * 54 * 3);
            for pixel in decoded.as_chunks::<3>().0 {
                assert_color(
                    pixel,
                    [0, 0, 255],
                    &format!("edge fixture {index}, sigma{strength}"),
                );
            }
        }
    }
}

#[test]
fn video_background_applies_solid_masks_and_lost_tracking_before_both_crops() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let screen = directory.path().join("private-center.mkv");
    run(media_command(&ffmpeg).args(["-f","lavfi","-i","color=c=blue:s=320x180:r=30:d=2.8,format=rgba,drawbox=x=96:y=54:w=96:h=72:c=lime:t=fill","-c:v","ffv1","-pix_fmt","gbrp"]).arg(&screen));
    project.screen_path = screen.to_string_lossy().into_owned();
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits = EditSettings::new(project.duration);
    project.edits.trim_end = 0.2;
    project.edits.background_mode = "videoBlur".into();
    project.edits.aspect_ratio = "9:16".into();
    project.edits.padding = 240;
    project.edits.camera.enabled = false;
    project.edits.show_cursor = false;
    project.edits.crop = Some(CropSettings {
        x: 0.3,
        y: 0.3,
        width: 0.3,
        height: 0.4,
    });
    let key = |time, kind: &str| MaskKeyframe {
        time,
        x: 0.28,
        y: 0.28,
        width: 0.36,
        height: 0.44,
        confidence: 1.0,
        kind: kind.into(),
    };
    project.edits.redactions = vec![Redaction {
        id: "secret".into(),
        start: 0.0,
        end: 1.1,
        x: 0.28,
        y: 0.28,
        width: 0.36,
        height: 0.44,
        mode: "solid".into(),
        strength: 8.0,
        tracking: None,
    }];
    let output = render(&ffmpeg, &project, "static-mask-background.mp4", 7);
    for pixel in video_pixels(&ffmpeg, &output.output_path)
        .as_chunks::<3>()
        .0
    {
        assert_color(pixel, [0, 0, 0], "static mask background");
    }
    project.edits.trim_start = 0.017;
    project.edits.trim_end = 1.017;
    project.edits.redactions[0].mode = "blur".into();
    project.edits.redactions[0].tracking = Some(MaskTracking {
        reviewed: true,
        keyframes: vec![
            key(0.0, "manual"),
            key(0.3, "auto"),
            MaskKeyframe {
                time: 0.5,
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                confidence: 0.0,
                kind: "lost".into(),
            },
            key(0.8, "manual"),
        ],
    });
    let output = render(&ffmpeg, &project, "lost-mask-background.mp4", 7);
    let decoded = video_pixels(&ffmpeg, &output.output_path);
    assert_eq!(decoded.len(), 30 * 96 * 54 * 3);
    for (index, frame) in decoded.as_chunks::<{ 96 * 54 * 3 }>().0.iter().enumerate() {
        let time = 0.017 + index as f64 / 30.0;
        if (0.3..0.8).contains(&time) {
            for pixel in frame.as_chunks::<3>().0 {
                assert_color(pixel, [0, 0, 0], "lost tracking background");
            }
        } else {
            assert!(
                frame[1] > 120,
                "Normal tracked blur did not resume at {time}"
            );
        }
    }
}

#[test]
fn camera_only_video_background_stays_solid_and_split_keeps_its_reserved_pane() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let screen = directory.path().join("blue-screen.mp4");
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x180:r=30:d=2.8",
            "-c:v",
            "mpeg4",
            "-q:v",
            "1",
        ])
        .arg(&screen));
    project.screen_path = screen.to_string_lossy().into_owned();
    let camera = directory.path().join("red-camera.mp4");
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=160x120:r=30:d=0.6",
            "-c:v",
            "mpeg4",
            "-q:v",
            "1",
        ])
        .arg(&camera));
    project.camera_path = Some(camera.to_string_lossy().into_owned());
    project.camera_intervals = Some(vec![CameraInterval {
        start: 0.2,
        end: 0.4,
    }]);
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits = EditSettings::new(project.duration);
    project.edits.background_mode = "videoBlur".into();
    project.edits.background = "#102030".into();
    project.edits.trim_end = 0.6;
    project.edits.camera.layout = "fullscreen".into();
    project.edits.camera.enabled = true;
    project.edits.corner_radius = 0;
    for mode in ["solid", "videoBlur"] {
        project.edits.background_mode = mode.into();
        for chunk_frames in [7, 900] {
            let output = render(
                &ffmpeg,
                &project,
                &format!("camera-only-{mode}-{chunk_frames}.mp4"),
                chunk_frames,
            );
            let decoded = video_pixels(&ffmpeg, &output.output_path);
            assert_eq!(decoded.len(), 18 * 96 * 54 * 3);
            for (index, frame) in decoded.as_chunks::<{ 96 * 54 * 3 }>().0.iter().enumerate() {
                assert_color(&frame[..3], [16, 32, 48], "camera-only corner");
                assert_color(
                    &frame[(27 * 96 + 48) * 3..][..3],
                    if (6..12).contains(&index) {
                        [255, 0, 0]
                    } else {
                        [16, 32, 48]
                    },
                    &format!("camera-only {mode} chunk{chunk_frames} gap frame {index}"),
                );
            }
        }
    }
    project.edits.camera.enabled = false;
    let output = render(&ffmpeg, &project, "camera-disabled-background.mp4", 7);
    for pixel in video_pixels(&ffmpeg, &output.output_path)
        .as_chunks::<3>()
        .0
    {
        assert_color(pixel, [16, 32, 48], "disabled camera-only background");
    }
    project.edits.camera.layout = "split".into();
    project.edits.camera.enabled = true;
    project.edits.camera.width = 0.3;
    project.edits.aspect_ratio = "9:16".into();
    let geometry = math::layout(&project, "1080p").unwrap();
    let pane = geometry.camera_pane.unwrap();
    assert!(geometry.frame_y + geometry.frame_height <= pane.y);
    let output = render(&ffmpeg, &project, "split-background.mp4", 7);
    let pixels = video_pixels(&ffmpeg, &output.output_path);
    assert_eq!(pixels.len(), 18 * 96 * 54 * 3);
    for (index, frame) in pixels.as_chunks::<{ 96 * 54 * 3 }>().0.iter().enumerate() {
        assert_color(&frame[..3], [0, 0, 255], "split video background");
        assert_color(
            &frame[(18 * 96 + 48) * 3..][..3],
            [0, 0, 255],
            "split screen stayed in its pane",
        );
        assert_color(
            &frame[(45 * 96 + 48) * 3..][..3],
            if (6..12).contains(&index) {
                [255, 0, 0]
            } else {
                [0, 0, 255]
            },
            &format!("split camera gap frame{index}"),
        );
    }
}
