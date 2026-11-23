use super::*;
use screenrecord_core::{ExportPreset, VideoClip};

fn sample(time: f64, x: f64, y: f64, buttons: u8, pressed: bool, visible: bool) -> CursorSample {
    CursorSample {
        time,
        x,
        y,
        buttons,
        pressed,
        visible,
    }
}

fn click_project(ffmpeg: &Path, directory: &Path) -> Project {
    let mut project = fixture(ffmpeg, directory);
    let screen = directory.join("plain.mkv");
    run(media_command(ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x180:r=60:d=2.8",
            "-c:v",
            "ffv1",
        ])
        .arg(&screen));
    project.screen_path = screen.to_string_lossy().into_owned();
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits = EditSettings::new(2.8);
    project.edits.trim_end = 1.2;
    project.edits.camera.enabled = false;
    project.edits.padding = 0;
    project.edits.corner_radius = 0;
    project.edits.cursor_size = 1.0;
    project.edits.cursor_smoothing = 1.0;
    project.cursor = vec![
        sample(0.0, 0.25, 0.4, 0, false, true),
        sample(0.1, 0.25, 0.4, 1, true, true),
        sample(0.12, 0.7, 0.8, 0, false, true),
        sample(0.35, 0.55, 0.4, 2, true, true),
        sample(0.37, 0.7, 0.8, 0, false, true),
        sample(0.55, 0.8, 0.4, 0, true, true),
        sample(0.57, 0.7, 0.8, 0, false, true),
        sample(1.1, 0.7, 0.8, 0, false, true),
    ];
    project
}

fn render_clicks(
    ffmpeg: &Path,
    project: &Project,
    name: &str,
    fps: u32,
    compact: bool,
    chunk: u64,
) -> String {
    let mut output = options(Path::new(&project.directory), name);
    output.fps = fps;
    if compact {
        output.preset = ExportPreset::Whatsapp;
        output.resolution = "auto".into();
    }
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    export_inner(
        project,
        &output,
        ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        chunk,
    )
    .unwrap();
    output.output_path
}

fn frame(ffmpeg: &Path, path: &str, index: u32) -> Vec<u8> {
    run(media_command(ffmpeg).args([
        "-i",
        path,
        "-vf",
        &format!("select=eq(n\\,{index})"),
        "-frames:v",
        "1",
        "-pix_fmt",
        "rgb24",
        "-f",
        "rawvideo",
        "-",
    ]))
}

fn neighborhood(pixels: &[u8], width: u32, x: f64, y: f64, kind: usize) -> bool {
    let x = x.round() as i32;
    let y = y.round() as i32;
    (-3..=3).any(|dy| {
        (-3..=3).any(|dx| {
            let offset = (((y + dy) as usize * width as usize) + (x + dx) as usize) * 3;
            let p = &pixels[offset..offset + 3];
            match kind {
                0 => p[0] > 100 && p[0] > p[2].saturating_add(45) && p[1] > 60,
                1 => {
                    p[2] > 100
                        && p[1] > 95
                        && p[2] > p[0].saturating_add(40)
                        && p[1] > p[0].saturating_add(30)
                }
                _ => p.iter().all(|v| *v > 90) && p[0].abs_diff(p[2]) < 18,
            }
        })
    })
}

fn centered_ring(pixels: &[u8], width: u32, x: f64, y: f64, radius: f64, kind: usize) -> u8 {
    let height = pixels.len() / (width as usize * 3);
    let extent = radius * 2.5 + 10.0;
    let mut peak_channel = 0;
    let mut points = Vec::new();
    for py in (y - extent).max(0.0) as usize..((y + extent).ceil() as usize).min(height) {
        for px in (x - extent).max(0.0) as usize..((x + extent).ceil() as usize).min(width as usize)
        {
            let p = &pixels[(py * width as usize + px) * 3..][..3];
            let colored = match kind {
                0 => {
                    p[0] > 20
                        && p[0] > p[2].saturating_add(10)
                        && p[0] as f64 > p[1] as f64 * 1.1
                        && p[1] as f64 > p[2] as f64 * 1.3
                }
                1 => {
                    p[2] > 20
                        && p[1] > 20
                        && p[2] > p[0].saturating_add(8)
                        && p[1] > p[0].saturating_add(8)
                }
                _ => p.iter().all(|v| *v > 15) && p[0].abs_diff(p[2]) < 8,
            };
            if colored {
                peak_channel = peak_channel.max(p[0].max(p[1]).max(p[2]));
                points.push((px as f64 + 0.5, py as f64 + 0.5));
            }
        }
    }
    assert!(
        points.len() > 10,
        "Ring pixels missing, kind{kind}, center{x},{y}"
    );
    let center = (
        points.iter().map(|p| p.0).sum::<f64>() / points.len() as f64,
        points.iter().map(|p| p.1).sum::<f64>() / points.len() as f64,
    );
    assert!(
        (center.0 - x).abs() <= 2.0 && (center.1 - y).abs() <= 2.0,
        "Ring centroid drift: kind{kind}, expected({x},{y}), actual{center:?}, radius{radius}"
    );
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        for factor in if kind == 1 {
            vec![1.0, 0.64]
        } else {
            vec![1.0]
        } {
            let target = (x + dx * radius * factor, y + dy * radius * factor);
            assert!(
                points
                    .iter()
                    .any(|p| (p.0 - target.0).abs() <= 3.0 && (p.1 - target.1).abs() <= 3.0),
                "Missing ring quadrant {dx},{dy}, factor{factor}, center{x},{y}, radius{radius}, kind{kind}"
            );
        }
    }
    eprintln!(
        "Ring kind{kind}: expected center({x:.2},{y:.2}), measured({:.2},{:.2}), radius{radius:.2}",
        center.0, center.1
    );
    peak_channel
}

#[test]
fn click_rings_use_raw_positions_left_right_and_legacy_at_30_60_and_compact() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let project = click_project(&ffmpeg, directory.path());
    let original = fs::read(&project.screen_path).unwrap();
    for (fps, compact, width, height) in [
        (30, false, 1920, 1080),
        (60, false, 1920, 1080),
        (30, true, 1280, 720),
    ] {
        let output = render_clicks(
            &ffmpeg,
            &project,
            &format!("clicks-{fps}-{compact}.mp4"),
            fps,
            compact,
            7,
        );
        let scale = height as f64 / 1080.0;
        for (time, start, x, kind) in [
            (0.2, 0.1, 0.25, 0),
            (0.4, 0.35, 0.55, 1),
            (0.6, 0.55, 0.8, 2),
        ] {
            let pixels = frame(&ffmpeg, &output, (time * fps as f64).round() as u32);
            let radius = (12.0 + 16.0 * ((time - start) / 0.4)) * scale;
            centered_ring(
                &pixels,
                width,
                x * width as f64,
                0.4 * height as f64,
                radius,
                kind,
            );
            assert!(
                neighborhood(
                    &pixels,
                    width,
                    x * width as f64 - radius,
                    0.4 * height as f64,
                    kind
                ),
                "Missing kind{kind} ring at raw position, {fps}fps compact={compact}"
            );
            if kind == 1 {
                assert!(
                    neighborhood(
                        &pixels,
                        width,
                        x * width as f64 - radius * 0.64,
                        0.4 * height as f64,
                        kind
                    ),
                    "Right click inner ring missing"
                );
            }
            let center = ((0.4 * height as f64).round() as usize * width as usize
                + (x * width as f64).round() as usize)
                * 3;
            assert!(
                pixels[center..center + 3].iter().all(|v| *v < 15),
                "Ring was filled instead of hollow"
            );
        }
        for index in [0, fps, fps / 10 - 1] {
            let pixels = frame(&ffmpeg, &output, index);
            assert!(
                !pixels
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .any(|p| p[0] > 100 && p[0] > p[2].saturating_add(45) && p[1] > 60),
                "Left click appeared outside its lifetime"
            );
        }
        run(media_command(&ffmpeg)
            .arg("-i")
            .arg(&output)
            .args(["-f", "null", "-"]));
    }
    assert_eq!(fs::read(&project.screen_path).unwrap(), original);
}

#[test]
fn click_rings_respect_hidden_trim_cuts_disabled_and_chunk_boundaries() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = click_project(&ffmpeg, directory.path());
    project.cursor = vec![
        sample(0.0, 0.4, 0.4, 0, false, true),
        sample(0.1, 0.4, 0.4, 1, true, true),
        sample(0.3, 0.4, 0.4, 1, true, false),
        sample(0.4, 0.4, 0.4, 1, true, true),
        sample(0.5, 0.7, 0.8, 0, false, true),
        sample(0.6, 0.6, 0.4, 2, true, true),
        sample(0.8, 0.7, 0.8, 0, false, true),
    ];
    let whole = render_clicks(&ffmpeg, &project, "click-whole.mp4", 60, false, 900);
    let chunks = render_clicks(&ffmpeg, &project, "click-chunks.mp4", 60, false, 7);
    for index in [5, 6, 7, 17, 18, 24, 35, 36, 59, 60] {
        let a = frame(&ffmpeg, &whole, index);
        let b = frame(&ffmpeg, &chunks, index);
        let difference = a
            .iter()
            .zip(&b)
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / a.len() as f64;
        assert!(
            difference < 0.3,
            "Click changed at chunk boundary frame{index}: {difference}"
        );
        if [5, 18, 24, 35, 60].contains(&index) {
            assert!(!a
                .as_chunks::<3>()
                .0
                .iter()
                .any(|p| p[0] > 100 && p[0] > p[2].saturating_add(45) && p[1] > 60));
        }
        if index == 6 {
            assert!(neighborhood(&a, 1920, 0.4 * 1920.0 - 12.0, 0.4 * 1080.0, 0));
        }
        if index == 36 {
            assert!(neighborhood(&a, 1920, 0.6 * 1920.0 - 12.0, 0.4 * 1080.0, 1));
        }
    }
    project.edits.trim_start = 0.2;
    project.edits.video_clips = vec![
        VideoClip {
            id: "a".into(),
            start: 0.2,
            end: 0.55,
        },
        VideoClip {
            id: "b".into(),
            start: 0.7,
            end: 1.2,
        },
    ];
    for mode in 0..5 {
        let mut variant = project.clone();
        match mode {
            1 => variant.edits.show_clicks = false,
            2 => variant.edits.show_cursor = false,
            3 => variant.edits.camera.layout = "fullscreen".into(),
            4 => {
                variant.edits.crop = Some(CropSettings {
                    x: 0.0,
                    y: 0.0,
                    width: 0.2,
                    height: 1.0,
                })
            }
            _ => {}
        }
        let output = render_clicks(
            &ffmpeg,
            &variant,
            &format!("click-hidden-{mode}.mp4"),
            30,
            false,
            7,
        );
        let pixels = video_pixels(&ffmpeg, &output);
        assert!(
            !pixels.as_chunks::<3>().0.iter().any(|p| (p[0] > 100
                && p[0] > p[2].saturating_add(45)
                && p[1] > 60)
                || (p[2] > 100
                    && p[1] > 95
                    && p[2] > p[0].saturating_add(40)
                    && p[1] > p[0].saturating_add(30))),
            "Click escaped disabled/retained/crop gate{mode}"
        );
    }
}

#[test]
fn click_rings_stay_inside_zoomed_cropped_split_foreground_and_leave_blur_clean() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = click_project(&ffmpeg, directory.path());
    project.edits.padding = 60;
    project.edits.corner_radius = 80;
    project.edits.camera.layout = "split".into();
    project.edits.camera.width = 0.35;
    project.edits.background_mode = "videoBlur".into();
    project.edits.crop = Some(CropSettings {
        x: 0.1,
        y: 0.1,
        width: 0.8,
        height: 0.8,
    });
    project.edits.zooms = vec![ZoomSegment {
        id: "fixed".into(),
        start: 0.0,
        end: 1.2,
        scale: 2.0,
        x: 0.5,
        y: 0.5,
        follow_cursor: false,
    }];
    project.cursor = vec![
        sample(0.0, 0.5, 0.5, 0, false, true),
        sample(0.5, 0.5, 0.5, 1, true, true),
        sample(0.52, 0.95, 0.95, 0, false, true),
        sample(0.9, 0.05, 0.05, 2, true, true),
        sample(0.92, 0.95, 0.95, 0, false, true),
    ];
    let output = render_clicks(&ffmpeg, &project, "click-split.mp4", 60, false, 7);
    let layout = math::layout(&project, "1080p").unwrap();
    let pixels = frame(&ffmpeg, &output, 36);
    let x = layout.frame_x as f64 + layout.frame_width as f64 / 2.0;
    let y = layout.frame_y as f64 + layout.frame_height as f64 / 2.0;
    assert!(
        neighborhood(&pixels, layout.width, x - 16.0, y, 0),
        "Raw center must follow the zoomed screen viewport even when the arrow moved outside it"
    );
    centered_ring(&pixels, layout.width, x, y, 16.0, 0);
    let mut colored = 0;
    for (index, pixel) in pixels.as_chunks::<3>().0.iter().enumerate() {
        if pixel[0] > 100 && pixel[0] > pixel[2].saturating_add(45) && pixel[1] > 60 {
            colored += 1;
            let x = index as u32 % layout.width;
            let y = index as u32 / layout.width;
            assert!(
                x >= layout.frame_x
                    && x < layout.frame_x + layout.frame_width
                    && y >= layout.frame_y
                    && y < layout.frame_y + layout.frame_height,
                "Click tinted blur/background/camera pane"
            );
        }
    }
    assert!(colored > 30);
    let outside = frame(&ffmpeg, &output, 60);
    assert!(
        !outside
            .as_chunks::<3>()
            .0
            .iter()
            .any(|p| p[2] > 100 && p[1] > 95 && p[2] > p[0].saturating_add(40)),
        "Excluded raw click center leaked into the crop"
    );
}

fn lossless_click_frames(ffmpeg: &Path, project: &Project, directory: &Path) -> String {
    let output = directory.join("click-geometry.mkv");
    let mut options = options(directory, "click-geometry.mp4");
    options.fps = 60;
    let cancel = AtomicBool::new(false);
    let graph = graph::RenderGraph::prepare(project, &options, ffmpeg, directory, &cancel).unwrap();
    let chunks = graph.chunks(graph::MAX_CHUNK_FRAMES).unwrap();
    let [chunk] = chunks.as_slice() else {
        panic!("The synthetic click timeline must fit in one chunk");
    };
    let production = graph
        .video_command(
            project,
            &options,
            ffmpeg,
            process::Encoder::SoftwareMf,
            *chunk,
            &cancel,
        )
        .unwrap();
    // Preserve the production inputs, filters and output mapping before the H.264 encoder.
    run(Command::new(ffmpeg)
        .current_dir(directory)
        .args(
            production
                .get_args()
                .take_while(|arg| *arg != std::ffi::OsStr::new("-an")),
        )
        .args([
            "-an",
            "-frames:v",
            &chunk.frames.to_string(),
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&output));
    output.to_string_lossy().into_owned()
}

#[test]
fn click_ring_centroids_stay_fixed_during_growth_and_fade_at_size_limits() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = click_project(&ffmpeg, directory.path());
    project.edits.trim_end = 1.6;
    project.edits.cursor_smoothing = 0.0;
    project.cursor = vec![
        sample(0.0, 0.5, 0.5, 0, false, true),
        sample(0.1, 0.5, 0.5, 1, true, true),
        sample(0.12, 0.8, 0.85, 0, false, true),
        sample(0.8, 0.8, 0.85, 0, false, true),
        sample(0.9, 0.5, 0.5, 2, true, true),
        sample(0.92, 0.8, 0.85, 0, false, true),
        sample(1.5, 0.8, 0.85, 0, false, true),
    ];
    for size in [0.5, 4.0] {
        project.edits.cursor_size = size;
        let stage = tempfile::tempdir_in(directory.path()).unwrap();
        let output = lossless_click_frames(&ffmpeg, &project, stage.path());
        for (kind, start) in [(0, 0.1), (1, 0.9)] {
            let mut previous_peak = None;
            for age in [0.05, 0.15, 0.3] {
                let pixels = frame(&ffmpeg, &output, ((start + age) * 60.0_f64).round() as u32);
                let peak = centered_ring(
                    &pixels,
                    1920,
                    960.0,
                    540.0,
                    (12.0 + 16.0 * age / 0.4) * size,
                    kind,
                );
                if let Some(previous) = previous_peak {
                    assert!(peak < previous, "Ring did not fade at size {size}, age {age}, kind {kind}: {previous} -> {peak}");
                }
                previous_peak = Some(peak);
            }
        }
    }
}
