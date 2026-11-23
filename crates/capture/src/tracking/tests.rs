use super::*;
use std::{io::Write, sync::atomic::AtomicU32};

const WIDTH: usize = 160;
const HEIGHT: usize = 96;
const PATCH_W: usize = 24;
const PATCH_H: usize = 20;

fn rectangle(x: usize, y: usize) -> TrackingRect {
    TrackingRect {
        x: x as f64 / WIDTH as f64,
        y: y as f64 / HEIGHT as f64,
        width: PATCH_W as f64 / WIDTH as f64,
        height: PATCH_H as f64 / HEIGHT as f64,
    }
}

fn paint(frame: &mut [u8], x: usize, y: usize) {
    for py in 0..PATCH_H {
        for px in 0..PATCH_W {
            let n = (px as u32).wrapping_mul(73_856_093) ^ (py as u32).wrapping_mul(19_349_663);
            frame[(y + py) * WIDTH + x + px] = (40 + n % 180) as u8;
        }
    }
}

fn frame(x: usize, y: usize) -> Vec<u8> {
    let mut frame = vec![16; WIDTH * HEIGHT];
    paint(&mut frame, x, y);
    frame
}

fn input(path: &Path, ffmpeg: &Path, start: f64, end: f64, x: usize, y: usize) -> TrackingInput {
    TrackingInput {
        source_path: path.to_owned(),
        ffmpeg_path: ffmpeg.to_owned(),
        source_width: WIDTH as u32,
        source_height: HEIGHT as u32,
        start,
        end,
        rect: rectangle(x, y),
    }
}

#[test]
fn translates_in_both_directions_and_reaches_all_source_edges() {
    let cancel = AtomicBool::new(false);
    for (x, y, dx, dy) in [
        (0, 0, 7, 9),
        (136, 76, -9, -7),
        (30, 40, -5, 8),
        (45, 25, 11, -13),
    ] {
        let mut tracker =
            matcher::Tracker::new(&frame(x, y), WIDTH, HEIGHT, rectangle(x, y)).unwrap();
        let first = tracker.locate(&frame(x, y), &cancel).unwrap();
        assert!((first.0 - x as f64 / WIDTH as f64).abs() < 1e-12);
        let next_x = (x as i32 + dx) as usize;
        let next_y = (y as i32 + dy) as usize;
        let found = tracker.locate(&frame(next_x, next_y), &cancel).unwrap();
        assert!((found.0 - next_x as f64 / WIDTH as f64).abs() < 1e-12);
        assert!((found.1 - next_y as f64 / HEIGHT as f64).abs() < 1e-12);
        assert!(found.2 > 0.999);
    }
}

#[test]
fn rejects_flat_small_occluded_and_ambiguous_targets() {
    let cancel = AtomicBool::new(false);
    assert!(
        matcher::Tracker::new(&vec![100; WIDTH * HEIGHT], WIDTH, HEIGHT, rectangle(20, 20))
            .is_none()
    );
    let mut small = rectangle(20, 20);
    small.width = 0.01;
    assert!(matcher::Tracker::new(&frame(20, 20), WIDTH, HEIGHT, small).is_none());
    let mut tracker =
        matcher::Tracker::new(&frame(20, 20), WIDTH, HEIGHT, rectangle(20, 20)).unwrap();
    assert!(tracker.locate(&vec![16; WIDTH * HEIGHT], &cancel).is_none());
    let mut repeated = frame(20, 20);
    paint(&mut repeated, 20, 44);
    assert!(tracker.locate(&repeated, &cancel).is_none());
}

#[test]
fn illumination_change_preserves_translation_but_inverted_texture_is_lost() {
    let cancel = AtomicBool::new(false);
    let mut tracker =
        matcher::Tracker::new(&frame(25, 20), WIDTH, HEIGHT, rectangle(25, 20)).unwrap();
    let shifted: Vec<_> = frame(29, 26).into_iter().map(|v| v + 20).collect();
    assert!(tracker.locate(&shifted, &cancel).is_some());
    let inverted: Vec<_> = frame(29, 26).into_iter().map(|v| 255 - v).collect();
    assert!(tracker.locate(&inverted, &cancel).is_none());
}

#[test]
fn lost_stops_scan_without_resuming_after_object_returns() {
    let frames = [
        frame(20, 20),
        frame(24, 21),
        vec![16; WIDTH * HEIGHT],
        frame(28, 22),
    ]
    .concat();
    let (result, early) = scan(
        frames.as_slice(),
        input(Path::new("unused"), Path::new("unused"), 5.0, 5.4, 20, 20),
        WIDTH,
        HEIGHT,
        4,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicU64::new(0)),
    )
    .unwrap();
    assert!(early);
    assert_eq!(result.keyframes.len(), 3);
    assert_eq!(result.lost_at, Some(5.2));
    let last = result.keyframes.last().unwrap();
    assert_eq!(
        (last.x, last.y, last.width, last.height, last.confidence),
        (0.0, 0.0, 1.0, 1.0, 0.0)
    );
    assert_eq!(last.kind, "lost");
}

#[test]
fn corrupt_or_truncated_decode_never_returns_partial_automatic_keys() {
    let mut bytes = frame(20, 20);
    bytes.extend_from_slice(&[1, 2, 3]);
    assert!(scan(
        bytes.as_slice(),
        input(Path::new("unused"), Path::new("unused"), 0.0, 0.2, 20, 20),
        WIDTH,
        HEIGHT,
        2,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicU64::new(0))
    )
    .is_err());
}

#[test]
fn resource_limits_and_subframe_ranges_are_bounded() {
    assert_eq!(dimensions(7680, 4320), (640, 360));
    assert_eq!(dimensions(320, 240), (320, 240));
    assert_eq!(frame_count(300.0), 3000);
    assert_eq!(frame_count(0.01), 1);
    assert_eq!(frame_count(1.1), 11);
    let mut data = input(Path::new("unused"), Path::new("unused"), 0.0, 301.0, 20, 20);
    assert!(validate(&data).unwrap_err().contains("300 seconds"));
    data.end = f64::NAN;
    assert!(validate(&data).is_err());
    let file = tempfile::NamedTempFile::new().unwrap();
    data = input(file.path(), file.path(), 10.1, 310.1 + 5e-10, 20, 20);
    assert!(validate(&data).is_ok());
    assert_eq!(
        frame_count((data.end - data.start).min(MAX_MASK_TRACKING_SECONDS)),
        3000
    );
    data.end = 310.1 + 2e-9;
    assert!(validate(&data).is_err());
}

fn quiet_child() -> Command {
    let mut cmd = process::command(&std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "tracking::tests::blocked_decoder_fixture",
        "--nocapture",
    ]);
    cmd.env("SCREENRECORD_TRACKING_BLOCKED_FIXTURE", "1");
    cmd
}

#[test]
fn blocked_decoder_fixture() {
    if std::env::var("SCREENRECORD_TRACKING_BLOCKED_FIXTURE").as_deref() != Ok("1") {
        return;
    }
    println!("TRACKING_FIXTURE_PID:{}", std::process::id());
    std::io::stdout().flush().unwrap();
    thread::sleep(Duration::from_secs(30));
}

fn process_has_exited(pid: u32) -> bool {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return true;
        };
        let mut code = 259;
        let queried = GetExitCodeProcess(handle, &mut code).is_ok();
        let _ = CloseHandle(handle);
        queried && code != 259
    }
}

#[test]
fn cancellation_and_watchdog_reap_a_blocked_decoder() {
    for watchdog in [false, true] {
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let pid = Arc::new(AtomicU32::new(0));
        let reader_pid = pid.clone();
        let cancelling = thread::spawn(move || {
            thread::sleep(Duration::from_millis(700));
            if !watchdog {
                signal.store(true, Ordering::Release);
            }
        });
        let start = Instant::now();
        let error = run(
            &mut quiet_child(),
            cancel,
            Duration::from_secs(5),
            Duration::from_millis(900),
            |_| {},
            move |pipe, _| {
                use std::io::BufRead;
                let mut pipe = BufReader::new(pipe);
                let mut line = String::new();
                loop {
                    line.clear();
                    if pipe
                        .read_line(&mut line)
                        .map_err(|error| error.to_string())?
                        == 0
                    {
                        return Err(
                            "The owned decoder fixture exited before its PID handshake.".into()
                        );
                    }
                    if let Some(pid) = line.trim().strip_prefix("TRACKING_FIXTURE_PID:") {
                        reader_pid.store(
                            pid.parse()
                                .map_err(|error| format!("Invalid fixture PID: {error}"))?,
                            Ordering::Release,
                        );
                        break;
                    }
                }
                let mut rest = Vec::new();
                pipe.read_to_end(&mut rest).unwrap();
                Ok(((), false))
            },
        )
        .unwrap_err();
        cancelling.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(
            error.contains(if watchdog { "progress" } else { "cancelled" }),
            "{error}"
        );
        assert_ne!(pid.load(Ordering::Acquire), 0);
        assert!(process_has_exited(pid.load(Ordering::Acquire)));
    }
}

#[test]
fn reader_failure_terminates_the_owned_child_promptly() {
    let start = Instant::now();
    let error = run::<()>(
        &mut quiet_child(),
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(5),
        Duration::from_secs(5),
        |_| {},
        |_pipe, _| Err("injected reader failure".into()),
    )
    .unwrap_err();
    assert_eq!(error, "injected reader failure");
    assert!(start.elapsed() < Duration::from_secs(2));
}

fn ffmpeg() -> PathBuf {
    std::env::var_os("SCREENRECORD_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/ffmpeg.exe")
        })
}

fn video(path: &Path, frames: &[Vec<u8>], rate: u32) {
    let mut command = process::command(&ffmpeg());
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "rawvideo",
        "-pixel_format",
        "gray",
        "-video_size",
        "160x96",
        "-framerate",
        &rate.to_string(),
        "-i",
        "pipe:0",
        "-an",
        "-c:v",
        "png",
        "-pix_fmt",
        "gray",
        "-threads",
        "1",
    ]);
    let mut child = command
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let job = process::ChildJob::attach(&child).unwrap();
    let mut pipe = child.stdin.take().unwrap();
    for frame in frames {
        pipe.write_all(frame).unwrap();
    }
    drop(pipe);
    let status = process::wait_bounded(&mut child, Duration::from_secs(15)).unwrap();
    let mut diagnostic = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut diagnostic)
        .unwrap();
    assert!(status.success(), "{diagnostic}");
    drop(job);
}

#[test]
#[ignore = "requires bundled FFmpeg; synthetic video only"]
fn media_contract_tracking_translation_causal_start_and_source_immutability() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("translation.mov");
    let frames: Vec<_> = (0..30).map(|i| frame(20 + i * 2, 20)).collect();
    video(&path, &frames, 10);
    let original = std::fs::read(&path).unwrap();
    for (start, end, first) in [(0.0, 2.9, 0), (1.2345, 2.7345, 12), (0.1999, 0.2101, 1)] {
        let result = track(
            input(&path, &ffmpeg(), start, end, 20 + first * 2, 20),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        assert_eq!(result.lost_at, None, "{result:?}");
        assert_eq!(result.keyframes.len(), frame_count(end - start));
        for key in result.keyframes {
            let expected = 20 + ((key.time * 10.0).floor() as usize) * 2;
            assert!(
                (key.x * WIDTH as f64 - expected as f64).abs() < 0.001,
                "{}: {} != {expected}",
                key.time,
                key.x * WIDTH as f64
            );
            assert!(key.time >= start && key.time < end);
        }
    }
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
#[ignore = "requires bundled FFmpeg; synthetic video only"]
fn media_contract_tracking_occlusion_latches_and_geometry_mismatch_rejects() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("occlusion.mov");
    video(
        &path,
        &[
            frame(20, 20),
            frame(23, 20),
            vec![16; WIDTH * HEIGHT],
            frame(26, 20),
        ],
        10,
    );
    let result = track(
        input(&path, &ffmpeg(), 0.0, 0.4, 20, 20),
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.lost_at, Some(0.2));
    assert_eq!(result.keyframes.len(), 3);
    let mut invalid = input(&path, &ffmpeg(), 0.0, 0.4, 20, 20);
    invalid.source_width += 1;
    assert!(track(invalid, Arc::new(AtomicBool::new(false)), |_| {})
        .unwrap_err()
        .contains("dimensions changed"));
}

#[test]
#[ignore = "requires bundled FFmpeg; synthetic video only"]
fn media_contract_tracking_rgb_sentinel_never_samples_future_frame() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("color-boundary.mov");
    let mut command = process::command(&ffmpeg());
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "rawvideo",
        "-pixel_format",
        "rgb24",
        "-video_size",
        "160x96",
        "-framerate",
        "10",
        "-i",
        "pipe:0",
        "-an",
        "-c:v",
        "png",
        "-threads",
        "1",
    ]);
    let mut child = command
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let job = process::ChildJob::attach(&child).unwrap();
    let mut pipe = child.stdin.take().unwrap();
    for index in 0..15 {
        let color = if index < 12 { [255, 0, 0] } else { [0, 0, 255] };
        pipe.write_all(&color.repeat(WIDTH * HEIGHT)).unwrap();
    }
    drop(pipe);
    assert!(process::wait_bounded(&mut child, Duration::from_secs(15))
        .unwrap()
        .success());
    drop(job);
    let sparse = directory.path().join("color-sparse.mov");
    let mut sparse_command = process::command(&ffmpeg());
    sparse_command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
        .arg(&path)
        .args([
            "-vf",
            "select='eq(n,0)+eq(n,12)+eq(n,14)'",
            "-fps_mode",
            "vfr",
            "-c:v",
            "png",
            "-threads",
            "1",
        ])
        .arg(&sparse);
    let mut sparse_child = sparse_command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let sparse_job = process::ChildJob::attach(&sparse_child).unwrap();
    assert!(
        process::wait_bounded(&mut sparse_child, Duration::from_secs(15))
            .unwrap()
            .success()
    );
    drop(sparse_job);
    for path in [&path, &sparse] {
        for (start, end, red) in [
            (1.1999, 1.2101, true),
            (1.1, 1.2, true),
            (1.2001, 1.2201, false),
        ] {
            let mut command = decode_command(
                &input(path, &ffmpeg(), start, end, 20, 20),
                WIDTH,
                HEIGHT,
                1,
            );
            let bytes = run(
                &mut command,
                Arc::new(AtomicBool::new(false)),
                Duration::from_secs(5),
                Duration::from_secs(5),
                |_| {},
                |pipe, _| {
                    let mut bytes = Vec::new();
                    pipe.take((WIDTH * HEIGHT + 1) as u64)
                        .read_to_end(&mut bytes)
                        .unwrap();
                    Ok((bytes, false))
                },
            )
            .unwrap();
            assert_eq!(bytes.len(), WIDTH * HEIGHT);
            let mean = bytes.iter().map(|&v| f64::from(v)).sum::<f64>() / bytes.len() as f64;
            assert_eq!(
                mean > 50.0,
                red,
                "source time {start}, grayscale mean {mean}"
            );
        }
    }
}

#[test]
#[ignore = "requires bundled FFmpeg; synthetic 1080p video benchmark"]
fn media_contract_tracking_1080p_bounded_pipeline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tracking-1080p.mov");
    let mut command = process::command(&ffmpeg());
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "rawvideo",
        "-pixel_format",
        "gray",
        "-video_size",
        "640x360",
        "-framerate",
        "10",
        "-i",
        "pipe:0",
        "-an",
        "-vf",
        "scale=1920:1080:flags=neighbor",
        "-c:v",
        "png",
        "-pix_fmt",
        "gray",
        "-threads",
        "1",
    ]);
    let mut child = command
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let job = process::ChildJob::attach(&child).unwrap();
    let mut pipe = child.stdin.take().unwrap();
    let mut image = vec![16; 640 * 360];
    for index in 0..100 {
        image.fill(16);
        let x = 200 + (20 - (index % 40_i32 - 20).abs()) as usize * 3;
        for py in 0..40 {
            for px in 0..48 {
                let n = (px as u32).wrapping_mul(73_856_093) ^ (py as u32).wrapping_mul(19_349_663);
                image[(120 + py) * 640 + x + px] = (40 + n % 180) as u8;
            }
        }
        pipe.write_all(&image).unwrap();
    }
    drop(pipe);
    assert!(process::wait_bounded(&mut child, Duration::from_secs(30))
        .unwrap()
        .success());
    drop(job);
    let started = Instant::now();
    let result = track(
        TrackingInput {
            source_path: path,
            ffmpeg_path: ffmpeg(),
            source_width: 1920,
            source_height: 1080,
            start: 0.0,
            end: 10.0,
            rect: TrackingRect {
                x: 200.0 / 640.0,
                y: 120.0 / 360.0,
                width: 48.0 / 640.0,
                height: 40.0 / 360.0,
            },
        },
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(result.lost_at, None);
    assert_eq!(result.keyframes.len(), 100);
    for (index, key) in result.keyframes.iter().enumerate() {
        let x = 200 + (20 - (index as i32 % 40 - 20).abs()) as usize * 3;
        assert!((key.x * 640.0 - x as f64).abs() < 0.001);
    }
    assert!(elapsed < Duration::from_secs(120));
    println!("Tracking benchmark: 10s 1920x1080 source, 100 frames at 640x360, {:.4}s elapsed, one 230400-byte analysis frame, <=320 template samples, <=32 candidates.", elapsed.as_secs_f64());
}
