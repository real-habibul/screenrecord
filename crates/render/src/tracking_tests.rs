use super::*;
use screenrecord_core::{MaskKeyframe, MaskTracking, Redaction, VideoClip};

fn key(time: f64, x: f64, width: f64, kind: &str) -> MaskKeyframe {
    MaskKeyframe {
        time,
        x,
        y: 0.125,
        width,
        height: 0.375,
        confidence: 0.95,
        kind: kind.into(),
    }
}

fn tracked(mode: &str) -> Redaction {
    let mut lost = key(0.7, 0.0, 1.0, "lost");
    lost.y = 0.0;
    lost.height = 1.0;
    Redaction {
        id: "moving-mask".into(),
        start: 0.0,
        end: 1.2,
        x: 0.125,
        y: 0.125,
        width: 0.25,
        height: 0.375,
        mode: mode.into(),
        strength: 8.0,
        tracking: Some(MaskTracking {
            reviewed: true,
            keyframes: vec![
                key(0.0, 0.125, 0.25, "manual"),
                key(0.217, 0.45, 0.2, "auto"),
                key(0.433, 0.7, 0.2, "manual"),
                lost,
                key(0.9, 0.15, 0.125, "manual"),
            ],
        }),
    }
}

fn filtered(
    ffmpeg: &Path,
    directory: &Path,
    project: &Project,
    start: f64,
    frames: u64,
    source: &str,
) -> Vec<u8> {
    let filter = format!(
        "[0:v]format=rgba[effectsource];{}format=rgb24[out]",
        effects::filters(
            project,
            start,
            frames,
            30,
            directory,
            &AtomicBool::new(false),
            &effects::patch_sizes(project)
        )
        .unwrap()
    );
    run(media_command(ffmpeg).current_dir(directory).args([
        "-filter_complex_threads",
        "2",
        "-f",
        "lavfi",
        "-i",
        source,
        "-filter_complex",
        &filter,
        "-map",
        "[out]",
        "-frames:v",
        &frames.to_string(),
        "-pix_fmt",
        "rgb24",
        "-f",
        "rawvideo",
        "-",
    ]))
}

#[test]
fn tracked_solid_matches_every_causal_source_frame_and_chunk_restart() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.width = 64;
    project.height = 48;
    project.edits.redactions = vec![tracked("solid")];
    let source = "color=c=white:s=64x48:r=30:d=2";
    let whole = filtered(&ffmpeg, directory.path(), &project, 0.017, 36, source);
    assert_eq!(whole.len(), 36 * 64 * 48 * 3);
    for (n, frame) in whole.as_chunks::<{ 64 * 48 * 3 }>().0.iter().enumerate() {
        let time = 0.017 + n as f64 / 30.0;
        let region = &project.edits.redactions[0];
        let (x, y, w, h) = effects::rectangle(
            screenrecord_core::redaction_rectangle_at(region, time),
            64,
            48,
        );
        for py in 0..48 {
            for px in 0..64 {
                let covered = time < region.end && px >= x && px < x + w && py >= y && py < y + h;
                let pixel = &frame[((py * 64 + px) * 3) as usize..][..3];
                assert_eq!(
                    pixel,
                    if covered {
                        &[0, 0, 0]
                    } else {
                        &[255, 255, 255]
                    },
                    "frame {n}, pixel {px},{py}"
                );
            }
        }
    }
    let mut chunked = filtered(&ffmpeg, directory.path(), &project, 0.017, 11, source);
    chunked.extend(filtered(
        &ffmpeg,
        directory.path(),
        &project,
        0.017 + 11.0 / 30.0,
        25,
        source,
    ));
    assert_eq!(whole, chunked);
    assert!(effects::filters(
        &project,
        0.0,
        901,
        30,
        directory.path(),
        &AtomicBool::new(false),
        &effects::patch_sizes(&project)
    )
    .is_err());
    assert!(effects::filters(
        &project,
        0.0,
        900,
        30,
        directory.path(),
        &AtomicBool::new(true),
        &effects::patch_sizes(&project)
    )
    .is_err());
}

#[test]
fn tracked_blur_patch_alpha_and_loss_match_independent_static_reference() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.width = 64;
    project.height = 48;
    project.edits.redactions = vec![tracked("blur")];
    let source = "nullsrc=s=64x48:r=30:d=2,geq=lum='mod(X+Y,2)*255':cb=128:cr=128";
    let whole = filtered(&ffmpeg, directory.path(), &project, 0.017, 36, source);
    let (pw, ph) = effects::patch_size(&project.edits.redactions[0], 64, 48);
    assert_eq!((pw, ph), (34, 18));
    for n in [0usize, 7, 13, 21, 27, 35] {
        let time = 0.017 + n as f64 / 30.0;
        let rect = screenrecord_core::redaction_rectangle_at(&project.edits.redactions[0], time);
        let (x, y, w, h) = effects::rectangle(rect, 64, 48);
        let px = x.min(64 - pw);
        let py = y.min(48 - ph);
        let reference = if rect.lost {
            "[0:v]format=rgba,drawbox=x=0:y=0:w=iw:h=ih:c=black:t=fill,format=rgb24[out]".into()
        } else {
            format!("[0:v]format=rgba,split=2[keep][patch];[patch]crop={pw}:{ph}:{px}:{py}:exact=1,gblur=sigma=8:steps=3,crop={w}:{h}:{}:{}:exact=1[blur];[keep][blur]overlay={x}:{y}:format=auto:shortest=1,format=rgb24[out]",x-px,y-py)
        };
        let expected = run(media_command(&ffmpeg).args([
            "-filter_complex_threads",
            "1",
            "-f",
            "lavfi",
            "-i",
            source,
            "-filter_complex",
            &reference,
            "-map",
            "[out]",
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "-",
        ]));
        let actual = &whole[n * 64 * 48 * 3..(n + 1) * 64 * 48 * 3];
        assert_eq!(actual, expected, "tracked blur differs at frame {n}");
    }
    let mut chunked = filtered(&ffmpeg, directory.path(), &project, 0.017, 11, source);
    chunked.extend(filtered(
        &ffmpeg,
        directory.path(),
        &project,
        0.017 + 11.0 / 30.0,
        25,
        source,
    ));
    assert_eq!(whole, chunked);
    let mut all_lost = project.clone();
    let lost = all_lost.edits.redactions[0]
        .tracking
        .as_ref()
        .unwrap()
        .keyframes[3]
        .clone();
    all_lost.edits.redactions[0]
        .tracking
        .as_mut()
        .unwrap()
        .keyframes = vec![MaskKeyframe { time: 0.0, ..lost }];
    assert_eq!(
        effects::patch_size(&all_lost.edits.redactions[0], 64, 48),
        (1, 1)
    );
    assert!(
        filtered(&ffmpeg, directory.path(), &all_lost, 0.0, 3, source)
            .iter()
            .all(|value| *value == 0)
    );
    for (width, height, pixel_width, pixel_height) in [
        (0.001, 0.375, 1, 18),
        (0.25, 0.001, 16, 1),
        (0.001, 0.001, 1, 1),
    ] {
        let mut tiny = project.clone();
        tiny.edits.redactions[0].width = width;
        tiny.edits.redactions[0].height = height;
        let mut point = key(0.0, 0.125, width, "manual");
        point.height = height;
        tiny.edits.redactions[0]
            .tracking
            .as_mut()
            .unwrap()
            .keyframes = vec![point];
        let tracked_pixels = filtered(&ffmpeg, directory.path(), &tiny, 0.0, 3, source);
        assert_eq!(tracked_pixels.len(), 3 * 64 * 48 * 3);
        tiny.edits.redactions[0].tracking = None;
        let static_pixels = filtered(&ffmpeg, directory.path(), &tiny, 0.0, 3, source);
        assert!(
            tracked_pixels == static_pixels,
            "tiny {pixel_width}x{pixel_height}: first difference {:?}, tracked {:?}, static {:?}",
            tracked_pixels
                .iter()
                .zip(&static_pixels)
                .enumerate()
                .find(|(_, (a, b))| a != b),
            tracked_pixels[(6 * 64 + 8) * 3..(6 * 64 + 8 + pixel_width) * 3].to_vec(),
            static_pixels[(6 * 64 + 8) * 3..(6 * 64 + 8 + pixel_width) * 3].to_vec()
        );
        tiny.edits.redactions.clear();
        let original = filtered(&ffmpeg, directory.path(), &tiny, 0.0, 3, source);
        if pixel_width > 1 || pixel_height > 1 {
            assert_ne!(tracked_pixels, original);
        }
        for (n, (actual, expected)) in tracked_pixels
            .as_chunks::<3>()
            .0
            .iter()
            .zip(original.as_chunks::<3>().0.iter())
            .enumerate()
        {
            let x = n % 64;
            let y = n / 64 % 48;
            if !(8..8 + pixel_width).contains(&x) || !(6..6 + pixel_height).contains(&y) {
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn tracked_export_requires_review_preserves_sources_and_obeys_edl_loss_and_cancel() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.microphone_path = None;
    project.system_audio_path = None;
    project.camera_path = None;
    project.edits.camera.enabled = false;
    project.edits.show_cursor = false;
    project.edits.padding = 0;
    project.edits.corner_radius = 0;
    project.edits.trim_start = 0.017;
    project.edits.trim_end = 1.217;
    project.edits.video_clips = vec![
        VideoClip {
            id: "one".into(),
            start: 0.017,
            end: 0.317,
        },
        VideoClip {
            id: "two".into(),
            start: 0.517,
            end: 1.217,
        },
    ];
    project.edits.redactions = vec![tracked("blur")];
    let original = fs::read(&project.screen_path).unwrap();
    let mut output = options(directory.path(), "tracked-export.mp4");
    output.fps = 30;
    project.edits.redactions[0]
        .tracking
        .as_mut()
        .unwrap()
        .reviewed = false;
    assert!(export(
        &project,
        &output,
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {}
    )
    .unwrap_err()
    .contains("Review"));
    assert!(!Path::new(&output.output_path).exists());
    project.edits.redactions[0]
        .tracking
        .as_mut()
        .unwrap()
        .reviewed = true;
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
    let pixels = video_pixels(&ffmpeg, &output.output_path);
    assert_eq!(pixels.len(), 30 * 96 * 54 * 3);
    for frame in 9..21 {
        assert!(
            pixels[frame * 96 * 54 * 3..(frame + 1) * 96 * 54 * 3]
                .iter()
                .all(|v| *v < 3),
            "private screen visible in lost frame {frame}"
        );
    }
    assert!(pixels[24 * 96 * 54 * 3..25 * 96 * 54 * 3]
        .iter()
        .any(|v| *v > 40));
    let cancelled = options(directory.path(), "cancelled-tracking.mp4");
    let flag = Arc::new(AtomicBool::new(false));
    let set = flag.clone();
    let error = export(&project, &cancelled, &ffmpeg, flag, move |progress| {
        if progress.progress > 0.0 {
            set.store(true, Ordering::Relaxed);
        }
    })
    .unwrap_err();
    assert!(error.to_lowercase().contains("cancel"), "{error}");
    assert!(!Path::new(&cancelled.output_path).exists());
    assert_eq!(fs::read(&project.screen_path).unwrap(), original);
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".screenrecord-export-")));
}

#[test]
#[ignore = "bounded 1080p tracking benchmark; run explicitly with the bundled runtime"]
fn measure_tracked_blur_export_and_command_bounds() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let screen = directory.path().join("tracking-1080.mp4");
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1920x1080:r=30:d=10",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
        ])
        .arg(&screen));
    project.screen_path = screen.to_string_lossy().into_owned();
    project.width = 1920;
    project.height = 1080;
    project.duration = 10.0;
    project.cursor.clear();
    project.camera_path = None;
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits = EditSettings::new(10.0);
    project.edits.camera.enabled = false;
    project.edits.show_cursor = false;
    let mut region = tracked("blur");
    region.end = 10.0;
    region.tracking.as_mut().unwrap().keyframes = (0..100)
        .filter(|n| !(61..70).contains(n))
        .map(|n| {
            let mut point = key(
                n as f64 / 10.0,
                0.1 + n as f64 * 0.006,
                0.15,
                if n == 0 || n == 70 { "manual" } else { "auto" },
            );
            point.y = 0.3;
            point.height = 0.2;
            if n == 60 {
                point.kind = "lost".into();
                point.x = 0.0;
                point.y = 0.0;
                point.width = 1.0;
                point.height = 1.0;
            }
            point
        })
        .collect();
    project.edits.redactions = vec![region];
    let mut output = options(directory.path(), "tracking-benchmark.mp4");
    output.fps = 30;
    let graphdir = tempfile::tempdir().unwrap();
    let graph = graph::RenderGraph::prepare(
        &project,
        &output,
        &ffmpeg,
        graphdir.path(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        graph.chunks(900).unwrap().len(),
        1,
        "keyframes must not create video processes"
    );
    let filter = effects::filters(
        &project,
        0.0,
        300,
        30,
        graphdir.path(),
        &AtomicBool::new(false),
        &effects::patch_sizes(&project),
    )
    .unwrap();
    assert!(filter.contains("trackcrop"));
    let command_bytes: u64 = fs::read_dir(graphdir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("mask-"))
        .map(|entry| entry.metadata().unwrap().len())
        .sum();
    let encoder = process::choose_encoder(
        &ffmpeg,
        process::VideoSpec {
            width: 1920,
            height: 1080,
            fps: 30,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let started = Instant::now();
    export(
        &project,
        &output,
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let seconds = started.elapsed().as_secs_f64();
    run(media_command(&ffmpeg).args(["-i", &output.output_path, "-f", "null", "-"]));
    eprintln!("TRACKING_BENCH seconds={seconds:.4} duration=10 resolution=1920x1080 fps=30 encoder={encoder:?} keyframes={} chunks=1 patch={:?} commands_bytes={command_bytes}",project.edits.redactions[0].tracking.as_ref().unwrap().keyframes.len(),effects::patch_sizes(&project));

    let mut stress = project.clone();
    stress.edits.redactions = (0..16)
        .map(|i| {
            let mut mask = project.edits.redactions[0].clone();
            mask.id = format!("mask-{i}");
            mask
        })
        .collect();
    let started = Instant::now();
    let filters = effects::filters(
        &stress,
        0.0,
        60,
        30,
        graphdir.path(),
        &AtomicBool::new(false),
        &effects::patch_sizes(&stress),
    )
    .unwrap();
    let filter = format!("[0:v]format=rgba[effectsource];{filters}null[out]");
    run(media_command(&ffmpeg)
        .current_dir(graphdir.path())
        .args(["-filter_complex_threads", "1", "-i"])
        .arg(&screen)
        .args([
            "-filter_complex",
            &filter,
            "-map",
            "[out]",
            "-frames:v",
            "60",
            "-f",
            "null",
            "-",
        ]));
    let stress_seconds = started.elapsed().as_secs_f64();
    eprintln!("TRACKING_16_MASK_BENCH seconds={stress_seconds:.4} frames=60 duration=2 source=1920x1080 fps=30 masks=16 filter_threads=1 output=null");
}
