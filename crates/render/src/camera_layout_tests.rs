use super::*;
use screenrecord_core::{CameraInterval, Redaction, SubtitleCue};

fn layout_project(ffmpeg: &Path, directory: &Path) -> Project {
    let mut project = fixture(ffmpeg, directory);
    let screen = directory.join("private-screen.mp4");
    let camera = directory.join("colored-camera.mp4");
    run(media_command(ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x180:r=30:d=2.8,drawbox=x=0:y=0:w=20:h=180:c=white:t=fill",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
        ])
        .arg(&screen));
    run(media_command(ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=green:s=160x120:r=30:d=1.2,drawbox=x=80:y=0:w=80:h=120:c=blue:t=fill",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
        ])
        .arg(&camera));
    project.screen_path = screen.to_string_lossy().into_owned();
    project.camera_path = Some(camera.to_string_lossy().into_owned());
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits = EditSettings::new(project.duration);
    project.edits.trim_end = 1.8;
    project.edits.background = "#102030".into();
    project.edits.corner_radius = 60;
    project.edits.camera.width = 0.3;
    project.edits.camera.mirror = true;
    project.camera_intervals = Some(vec![
        CameraInterval {
            start: 0.0,
            end: 0.3,
        },
        CameraInterval {
            start: 0.6,
            end: 1.8,
        },
    ]);
    project
}

fn render(ffmpeg: &Path, project: &Project, name: &str) -> ExportOptions {
    let mut output = options(Path::new(&project.directory), name);
    output.fps = 30;
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    export_inner(
        project,
        &output,
        ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        12,
    )
    .unwrap();
    output
}

fn frame(ffmpeg: &Path, output: &str, time: f64) -> Vec<u8> {
    run(media_command(ffmpeg).args([
        "-ss",
        &time.to_string(),
        "-i",
        output,
        "-frames:v",
        "1",
        "-pix_fmt",
        "rgb24",
        "-f",
        "rawvideo",
        "-",
    ]))
}

fn pixel(frame: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    frame[((y * width + x) * 3) as usize..][..3]
        .try_into()
        .unwrap()
}

fn background(pixel: [u8; 3]) {
    for (actual, expected) in pixel.into_iter().zip([16u8, 32, 48]) {
        assert!(
            actual.abs_diff(expected) < 9,
            "Expected background, got {pixel:?}"
        );
    }
}

#[test]
fn fullscreen_camera_never_reveals_screen_at_gaps_eof_disabled_or_missing_media() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = layout_project(&ffmpeg, directory.path());
    let originals = [
        project.screen_path.clone(),
        project.camera_path.clone().unwrap(),
    ]
    .map(|path| (path.clone(), std::fs::read(path).unwrap()));
    project.edits.camera.layout = "fullscreen".into();
    project.edits.subtitles = vec![SubtitleCue {
        words: vec![],
        source_ranges: vec![],
        id: "caption".into(),
        start: 1.2,
        end: 1.6,
        text: "Camera only".into(),
    }];
    project.edits.subtitle_style.enabled = true;
    for aspect in ["16:9", "9:16"] {
        project.edits.aspect_ratio = aspect.into();
        let geometry = math::layout(&project, "1080p").unwrap();
        let output = render(
            &ffmpeg,
            &project,
            &format!("full-{}.mp4", aspect.replace(':', "-")),
        );
        let pixels = video_pixels(&ffmpeg, &output.output_path);
        assert!(
            !pixels
                .as_chunks::<3>()
                .0
                .iter()
                .any(|p| p[0] > 150 && p[1] < 70 && p[2] < 70),
            "Private red screen leaked into fullscreen camera export"
        );
        let pane = geometry.camera_pane.unwrap();
        let visible = frame(&ffmpeg, &output.output_path, 0.1);
        let blue = pixel(
            &visible,
            geometry.width,
            pane.x + pane.width / 4,
            pane.y + pane.height / 2,
        );
        let green = pixel(
            &visible,
            geometry.width,
            pane.x + pane.width * 3 / 4,
            pane.y + pane.height / 2,
        );
        assert!(
            blue[2] > 180 && blue[0] < 50,
            "Camera mirror/cover failed: {blue:?}"
        );
        assert!(
            green[1] > 85 && green[2] < 50,
            "Camera mirror/cover failed: {green:?}"
        );
        background(pixel(&visible, geometry.width, pane.x + 2, pane.y + 2));
        for time in [0.4, 1.3] {
            background(pixel(
                &frame(&ffmpeg, &output.output_path, time),
                geometry.width,
                geometry.width / 2,
                geometry.height / 2,
            ));
        }
        let caption = frame(&ffmpeg, &output.output_path, 1.3);
        assert!(
            caption
                .as_chunks::<3>()
                .0
                .iter()
                .any(|pixel| pixel.iter().all(|value| *value > 220)),
            "Final-canvas caption was lost when the camera ended"
        );
    }
    project.edits.trim_end = 0.4;
    project.edits.subtitles.clear();
    project.edits.camera.enabled = false;
    let disabled = render(&ffmpeg, &project, "full-disabled.mp4");
    let geometry = math::layout(&project, "1080p").unwrap();
    background(pixel(
        &frame(&ffmpeg, &disabled.output_path, 0.1),
        geometry.width,
        geometry.width / 2,
        geometry.height / 2,
    ));
    assert!(frame(&ffmpeg, &disabled.output_path, 0.1)
        .as_chunks::<3>()
        .0
        .iter()
        .all(|pixel| pixel
            .iter()
            .zip([16u8, 32, 48])
            .all(|(actual, expected)| actual.abs_diff(expected) < 9)));
    project.edits.camera.enabled = true;
    project.camera_path = None;
    project.camera_intervals = None;
    let missing = render(&ffmpeg, &project, "full-missing.mp4");
    background(pixel(
        &frame(&ffmpeg, &missing.output_path, 0.1),
        geometry.width,
        geometry.width / 2,
        geometry.height / 2,
    ));
    assert!(frame(&ffmpeg, &missing.output_path, 0.1)
        .as_chunks::<3>()
        .0
        .iter()
        .all(|pixel| pixel
            .iter()
            .zip([16u8, 32, 48])
            .all(|(actual, expected)| actual.abs_diff(expected) < 9)));
    for (path, original) in originals {
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}

#[test]
fn split_camera_preserves_source_masks_cursor_zoom_and_reserved_panes_in_both_orientations() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = layout_project(&ffmpeg, directory.path());
    project.edits.camera.layout = "split".into();
    project.edits.trim_end = 1.0;
    project.edits.crop = Some(CropSettings {
        x: 0.1,
        y: 0.1,
        width: 0.8,
        height: 0.8,
    });
    project.edits.zooms = vec![ZoomSegment {
        id: "zoom".into(),
        start: 0.0,
        end: 1.8,
        scale: 2.0,
        x: 0.5,
        y: 0.5,
        follow_cursor: false,
    }];
    project.edits.redactions = vec![Redaction {
        tracking: None,
        id: "mask".into(),
        start: 0.0,
        end: 1.0,
        x: 0.46,
        y: 0.46,
        width: 0.08,
        height: 0.08,
        mode: "solid".into(),
        strength: 12.0,
    }];
    project.cursor = vec![CursorSample {
        buttons: 0,
        time: 0.0,
        x: 0.4,
        y: 0.4,
        pressed: false,
        visible: true,
    }];
    for aspect in ["16:9", "9:16"] {
        project.edits.aspect_ratio = aspect.into();
        let geometry = math::layout(&project, "1080p").unwrap();
        let camera = geometry.camera_pane.unwrap();
        let output = render(
            &ffmpeg,
            &project,
            &format!("split-{}.mp4", aspect.replace(':', "-")),
        );
        let pixels = frame(&ffmpeg, &output.output_path, 0.8);
        let center = pixel(
            &pixels,
            geometry.width,
            geometry.frame_x + geometry.frame_width / 2,
            geometry.frame_y + geometry.frame_height / 2,
        );
        assert!(
            center.iter().all(|v| *v < 12),
            "Source mask did not follow crop/zoom into the screen pane: {center:?}"
        );
        let red = pixel(
            &pixels,
            geometry.width,
            geometry.frame_x + geometry.frame_width * 3 / 4,
            geometry.frame_y + geometry.frame_height / 2,
        );
        assert!(red[0] > 180 && red[1] < 40 && red[2] < 40);
        let view = math::viewport(&project, &geometry, &project.cursor, 0.8);
        let (x, y) = (
            geometry.frame_x + view.cursor_x.round() as u32,
            geometry.frame_y + view.cursor_y.round() as u32,
        );
        assert!(
            (0..25).any(
                |dy| (0..20).any(|dx| pixel(&pixels, geometry.width, x + dx, y + dy)
                    .iter()
                    .all(|v| *v > 180))
            ),
            "Cursor was not composited inside the transformed screen pane"
        );
        let blue = pixel(
            &pixels,
            geometry.width,
            camera.x + camera.width / 4,
            camera.y + camera.height / 2,
        );
        assert!(blue[2] > 180 && blue[0] < 50);
        background(pixel(&pixels, geometry.width, camera.x + 2, camera.y + 2));
        let gap = frame(&ffmpeg, &output.output_path, 0.4);
        background(pixel(
            &gap,
            geometry.width,
            camera.x + camera.width / 2,
            camera.y + camera.height / 2,
        ));
        let still_screen = pixel(
            &gap,
            geometry.width,
            geometry.frame_x + geometry.frame_width * 3 / 4,
            geometry.frame_y + geometry.frame_height / 2,
        );
        assert!(still_screen[0] > 180 && still_screen[1] < 40);
    }
    project.edits.trim_end = 0.4;
    project.edits.camera.enabled = false;
    let output = render(&ffmpeg, &project, "split-disabled.mp4");
    let geometry = math::layout(&project, "1080p").unwrap();
    let camera = geometry.camera_pane.unwrap();
    let pixels = frame(&ffmpeg, &output.output_path, 0.2);
    background(pixel(
        &pixels,
        geometry.width,
        camera.x + camera.width / 2,
        camera.y + camera.height / 2,
    ));
    let red = pixel(
        &pixels,
        geometry.width,
        geometry.frame_x + geometry.frame_width * 3 / 4,
        geometry.frame_y + geometry.frame_height / 2,
    );
    assert!(red[0] > 180 && red[1] < 40);
}

#[test]
fn rounded_camera_hides_before_first_pts_and_after_eof_without_interval_metadata() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = layout_project(&ffmpeg, directory.path());
    let camera = directory.path().join("delayed-camera.mp4");
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=160x120:r=30:d=0.6",
            "-copyts",
            "-vf",
            "setpts=PTS+0.7/TB",
            "-fps_mode",
            "passthrough",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
        ])
        .arg(&camera));
    let camera_info = probe_media(&ffmpeg, &camera).unwrap();
    assert!(
        (camera_info
            .video()
            .unwrap()
            .start_time
            .as_ref()
            .unwrap()
            .parse::<f64>()
            .unwrap()
            - 0.7)
            .abs()
            < 0.00001
    );
    project.camera_path = Some(camera.to_string_lossy().into_owned());
    project.camera_offset_seconds = 0.1;
    project.camera_intervals = None;
    project.edits.camera.layout = "fullscreen".into();
    project.edits.aspect_ratio = "16:9".into();
    let output = render(&ffmpeg, &project, "delayed-full.mp4");
    for time in [0.1, 0.5, 1.3] {
        background(pixel(
            &frame(&ffmpeg, &output.output_path, time),
            1920,
            960,
            540,
        ));
    }
    let blue = pixel(&frame(&ffmpeg, &output.output_path, 0.8), 1920, 960, 540);
    assert!(blue[2] > 180 && blue[0] < 50);
}
