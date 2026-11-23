use super::*;
use screenrecord_core::{MaskKeyframe, MaskTracking, Redaction, ReframeKeyframe, VideoClip};

fn render(ffmpeg: &Path, project: &Project, name: &str, chunk_frames: u64) -> ExportOptions {
    render_profile(ffmpeg, project, name, chunk_frames, false)
}

fn render_profile(
    ffmpeg: &Path,
    project: &Project,
    name: &str,
    chunk_frames: u64,
    compact: bool,
) -> ExportOptions {
    let mut output = options(Path::new(&project.directory), name);
    output.fps = 30;
    if compact {
        output.preset = screenrecord_core::ExportPreset::Whatsapp;
        output.resolution = "auto".into();
    }
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

fn frame(ffmpeg: &Path, path: &str, time: f64) -> Vec<u8> {
    run(media_command(ffmpeg).args([
        "-ss",
        &time.to_string(),
        "-i",
        path,
        "-frames:v",
        "1",
        "-pix_fmt",
        "rgb24",
        "-f",
        "rawvideo",
        "-",
    ]))
}

fn pixel(bytes: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    bytes[((y * width + x) * 3) as usize..][..3]
        .try_into()
        .unwrap()
}

fn near(actual: [u8; 3], expected: [u8; 3]) {
    assert!(
        actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) < 12),
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn fill_exports_cover_the_screen_pane_and_fit_keeps_letterboxing() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let screen = directory.path().join("crop-sentinel.mp4");
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=yellow:s=320x180:r=30:d=2.8,drawbox=x=64:y=18:w=192:h=144:c=blue:t=fill",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
        ])
        .arg(&screen));
    project.screen_path = screen.to_string_lossy().into_owned();
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits = EditSettings::new(project.duration);
    project.edits.trim_end = 0.4;
    project.edits.background = "#102030".into();
    project.edits.corner_radius = 0;
    project.edits.show_cursor = false;
    project.edits.camera.enabled = false;
    project.edits.camera.width = 0.3;
    project.edits.crop = Some(CropSettings {
        x: 0.2,
        y: 0.1,
        width: 0.6,
        height: 0.8,
    });
    for (mode, camera_layout, aspect, expected) in [
        ("fit", "bubble", "9:16", (32, 579, 1016, 762)),
        ("fill", "bubble", "9:16", (32, 32, 1016, 1856)),
        ("fill", "split", "16:9", (32, 32, 1282, 1016)),
        ("fill", "split", "9:16", (32, 32, 1016, 1282)),
    ] {
        project.edits.fit_mode = mode.into();
        project.edits.camera.layout = camera_layout.into();
        project.edits.aspect_ratio = aspect.into();
        let geometry = math::layout(&project, "1080p").unwrap();
        assert_eq!(
            (
                geometry.frame_x,
                geometry.frame_y,
                geometry.frame_width,
                geometry.frame_height
            ),
            expected
        );
        let output = render(
            &ffmpeg,
            &project,
            &format!("{mode}-{camera_layout}-{}.mp4", aspect.replace(':', "-")),
            7,
        );
        let pixels = frame(&ffmpeg, &output.output_path, 0.2);
        assert_eq!(
            pixels.len(),
            geometry.width as usize * geometry.height as usize * 3
        );
        for (x, y) in [
            (expected.0 + 12, expected.1 + 12),
            (expected.0 + expected.2 - 12, expected.1 + expected.3 - 12),
        ] {
            near(pixel(&pixels, geometry.width, x, y), [0, 0, 255]);
        }
        near(pixel(&pixels, geometry.width, 8, 8), [16, 32, 48]);
        if mode == "fit" {
            near(pixel(&pixels, geometry.width, 540, 100), [16, 32, 48]);
        }
        if let Some(camera) = geometry.camera_pane {
            near(
                pixel(
                    &pixels,
                    geometry.width,
                    camera.x + camera.width / 2,
                    camera.y + camera.height / 2,
                ),
                [16, 32, 48],
            );
        }
        assert!(
            !pixels
                .as_chunks::<3>()
                .0
                .iter()
                .any(|p| p[0] > 160 && p[1] > 160 && p[2] < 80),
            "Pixels outside the manual crop leaked into {mode}"
        );
    }
    project.edits.fit_mode = "fill".into();
    project.edits.camera.layout = "fullscreen".into();
    let hidden = render(&ffmpeg, &project, "fill-camera-only.mp4", 7);
    let pixels = frame(&ffmpeg, &hidden.output_path, 0.2);
    for actual in pixels.as_chunks::<3>().0 {
        near(*actual, [16, 32, 48]);
    }
}

#[test]
fn fill_cut_zoom_tracking_cursor_and_audio_keep_privacy_across_chunk_boundaries() {
    privacy_export(false, false, false);
}

#[test]
fn reframe_cut_zoom_tracking_cursor_and_audio_keep_privacy_across_chunk_boundaries() {
    privacy_export(true, false, false);
}

#[test]
fn video_background_preserves_tracked_masks_crop_reframe_cut_audio_and_cursor() {
    privacy_export(true, true, false);
}

#[test]
fn whatsapp_cuts_tracked_masks_reframe_blur_cursor_and_audio_keep_privacy() {
    privacy_export(true, true, true);
}

fn privacy_export(reframe: bool, background: bool, compact: bool) {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let screen = directory.path().join("moving-private-content.mp4");
    run(media_command(&ffmpeg).args([
        "-f", "lavfi", "-i",
        "color=c=yellow:s=320x180:r=30:d=2.8,drawbox=x=32:y=18:w=256:h=144:c=blue:t=fill,drawbox=x=80:y=60:w=32:h=36:c=lime:t=fill:enable='lt(t,0.8)',drawbox=x=160:y=60:w=32:h=36:c=lime:t=fill:enable='gte(t,1.2)',drawbox=x=0:y=0:w=320:h=180:c=red:t=fill:enable='between(t,0.8,1.2)'",
        "-c:v", "mpeg4", "-q:v", "2",
    ]).arg(&screen));
    project.screen_path = screen.to_string_lossy().into_owned();
    let originals = [
        &project.screen_path,
        project.microphone_path.as_ref().unwrap(),
        project.system_audio_path.as_ref().unwrap(),
    ]
    .map(|path| (path.clone(), fs::read(path).unwrap()));
    project.edits.fit_mode = "fill".into();
    if background {
        project.edits.background_mode = "videoBlur".into();
        project.edits.background_blur = 64.0;
        project.edits.padding = 240;
    }
    project.edits.aspect_ratio = "9:16".into();
    project.edits.camera.enabled = false;
    project.edits.corner_radius = 0;
    project.edits.video_clips = [(0.217, 0.733), (1.217, 1.733)]
        .into_iter()
        .enumerate()
        .map(|(index, (start, end))| VideoClip {
            id: format!("cut-{index}"),
            start,
            end,
        })
        .collect();
    project.edits.zooms = vec![ZoomSegment {
        id: "follow".into(),
        start: 0.0,
        end: 2.0,
        scale: 1.7,
        x: 0.5,
        y: 0.5,
        follow_cursor: true,
    }];
    project.cursor = (0..240)
        .map(|n| CursorSample {
            buttons: 0,
            time: n as f64 / 120.0,
            x: 0.42 + n as f64 / 240.0 * 0.2,
            y: 0.63,
            visible: true,
            pressed: false,
        })
        .collect();
    if reframe {
        project.edits.reframe_keyframes = [(0.0, 0.42, 1.0), (2.0, 0.62, 1.3)]
            .into_iter()
            .enumerate()
            .map(|(index, (time, x, scale))| ReframeKeyframe {
                id: format!("reframe-{index}"),
                time,
                x,
                y: 0.6,
                scale,
            })
            .collect();
    }
    let key = |time, x, kind: &str| MaskKeyframe {
        time,
        x,
        y: 0.3,
        width: 0.14,
        height: 0.3,
        confidence: 1.0,
        kind: kind.into(),
    };
    project.edits.redactions = vec![Redaction {
        id: "private-moving".into(),
        start: 0.0,
        end: 2.0,
        x: 0.24,
        y: 0.3,
        width: 0.14,
        height: 0.3,
        mode: "solid".into(),
        strength: 12.0,
        tracking: Some(MaskTracking {
            reviewed: true,
            keyframes: vec![
                key(0.0, 0.24, "manual"),
                key(0.6, 0.49, "auto"),
                key(1.2, 0.49, "manual"),
            ],
        }),
    }];
    let whole = render_profile(&ffmpeg, &project, "fill-whole.mp4", 900, compact);
    let chunked = render_profile(&ffmpeg, &project, "fill-chunked.mp4", 7, compact);
    let pixels = video_pixels(&ffmpeg, &whole.output_path);
    let chunk_pixels = video_pixels(&ffmpeg, &chunked.output_path);
    assert_eq!(pixels.len(), 31 * 96 * 54 * 3);
    assert_eq!(pixels.len(), chunk_pixels.len());
    let difference = pixels
        .iter()
        .zip(&chunk_pixels)
        .map(|(a, b)| a.abs_diff(*b) as f64)
        .sum::<f64>()
        / pixels.len() as f64;
    assert!(
        difference < 2.0,
        "Chunking changed Fill composition, MAE={difference}"
    );
    for output in [&whole, &chunked] {
        let rendered = video_pixels(&ffmpeg, &output.output_path);
        for p in rendered.as_chunks::<3>().0 {
            assert!(
                !(p[1] > 130 && p[0] < 75 && p[2] < 75),
                "Tracked private content leaked: {p:?}"
            );
            assert!(
                !(p[0] > 160 && p[1] < 75 && p[2] < 75),
                "Deleted content leaked: {p:?}"
            );
            assert!(
                !(p[0] > 160 && p[1] > 160 && p[2] < 75),
                "Content outside manual crop leaked: {p:?}"
            );
        }
        if background {
            for frame in rendered.as_chunks::<{ 96 * 54 * 3 }>().0 {
                for y in 0..54 {
                    for x in [0, 1, 94, 95] {
                        let p = &frame[(y * 96 + x) * 3..][..3];
                        assert!(
                            p[0] < 12 && p[1] < 12,
                            "Background leaked a source sentinel: {p:?}"
                        );
                    }
                }
            }
        }
        let full = frame(&ffmpeg, &output.output_path, 0.3);
        assert!(
            full.as_chunks::<3>()
                .0
                .iter()
                .any(|p| p.iter().all(|v| *v > 210)),
            "Cursor disappeared from the Fill viewport"
        );
    }
    assert_eq!(
        audio_samples(&ffmpeg, &whole.output_path),
        audio_samples(&ffmpeg, &chunked.output_path)
    );
    for (path, original) in originals {
        assert_eq!(fs::read(path).unwrap(), original);
    }
}
