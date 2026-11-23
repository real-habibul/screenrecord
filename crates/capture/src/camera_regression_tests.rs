use super::*;

fn packet_line(chosen: i64, device: i64, graph: i64) -> String {
    format!("[dshow] passing through packet of type video size 100 timestamp {chosen} orig timestamp {device} graph timestamp {graph} diff 0 Camera")
}

#[test]
fn graph_clock_rejects_nominal_frame_timing_and_maps_an_independent_epoch() {
    let graph_origin = 17_900_000_000_000_000i64;
    let qpc_origin = 80_000_000_000i64;
    let mut chosen_graph = SourceClock::default();
    let mut chosen_device = SourceClock::default();
    for index in 0..8 {
        let elapsed = index * 10_000_000 * 10 / 288;
        let graph = graph_origin + elapsed;
        let device = graph_origin + index * 10_000_000 / 30;
        let receipt = qpc_origin + elapsed + (7 - index) * 10_000;
        chosen_graph.observe(&packet_line(graph, device, graph), receipt);
        chosen_device.observe(&packet_line(device, device, graph), receipt);
    }
    assert_eq!(chosen_graph.ready_epoch(), Some(qpc_origin));
    assert!(
        chosen_device.ready_epoch().is_none(),
        "A nominal 30 fps clock must not be mistaken for actual 28.8 fps delivery."
    );
    let screen_epoch = qpc_origin + 7_902_571;
    let offset = (screen_epoch - chosen_graph.ready_epoch().unwrap()) as f64 / 10_000_000.0;
    assert_eq!(offset, 0.7902571);
    let nominal_duration = 1080.0 / 30.0;
    let graph_duration = 1080.0 / 28.8;
    assert_eq!(graph_duration - nominal_duration, 1.5);
    assert!((graph_duration - offset - 36.7097429).abs() < 1e-9);
}

#[test]
fn graph_clock_freezes_after_eight_observations_without_following_pipe_delay() {
    let mut clock = SourceClock::default();
    for index in 0..8 {
        let graph = 400_000_000 + index * 350_000;
        clock.observe(
            &packet_line(graph, graph, graph),
            900_000_000 + index * 350_000,
        );
    }
    assert_eq!(clock.ready_epoch(), Some(900_000_000));
    let graph = 800_000_000;
    clock.observe(&packet_line(graph, graph, graph), 1_400_000_000);
    assert_eq!(clock.ready_epoch(), Some(900_000_000));
}

#[test]
fn completion_parser_requires_exact_end_and_preserves_ffmpeg_errors() {
    let mut output = CameraOutputState::default();
    for line in [
        "frame=90",
        "out_time_us=3125000",
        "total_size=100000",
        "frame=2",
        "out_time_us=N/A",
        "[info] progress=end",
        "progress=continue",
        "E2026 [ERR] [VCAMDS] Failed to open NBX hive",
    ] {
        output.observe(line);
    }
    assert_eq!(output.progress.frame, 90);
    assert_eq!(output.progress.output_us, 3_125_000);
    assert_eq!(output.progress.total_bytes, 100_000);
    assert!(!output.progress.complete && !output.failed);
    output.observe("progress=end");
    assert!(output.progress.complete);
    output.observe("[h264_mf @ encoder] [error] Error submitting a frame");
    output.observe("progress=end");
    assert!(
        output.failed,
        "A final progress block must not erase a real encoder error."
    );
}

fn replace_with_lingering_child(camera: &mut CameraWorker) {
    assert!(camera.child.is_none());
    let mut child = process::fixture_child(true);
    let job = process::ChildJob::attach(&child)
        .inspect_err(|_| {
            let _ = child.kill();
            let _ = child.wait();
        })
        .unwrap();
    camera.child = Some(child);
    camera._job = Arc::new(job);
}

#[test]
fn unfinished_or_error_shutdown_remains_failure_after_owned_termination() {
    for (complete, failed) in [(false, false), (true, true)] {
        let directory = tempfile::tempdir().unwrap();
        let mut camera = CameraWorker::fixture(directory.path());
        camera.stop().unwrap();
        replace_with_lingering_child(&mut camera);
        {
            let mut output = camera.output.lock().unwrap();
            output.progress.complete = complete;
            output.failed = failed;
        }
        let started = Instant::now();
        assert!(camera
            .stop()
            .unwrap_err()
            .contains("did not finish writing cleanly"));
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(!camera.requires_full_validation());
        assert!(camera.child.is_none());
        crate::cleanup::ensure_recovery_ready(directory.path()).unwrap();
    }
}

fn bundled_ffmpeg() -> PathBuf {
    PathBuf::from(std::env::var_os("SCREENRECORD_FFMPEG").expect("Set SCREENRECORD_FFMPEG."))
}

fn timing(epoch: i64, duration: f64) -> CameraTiming {
    CameraTiming {
        camera_epoch: epoch,
        screen_epoch: epoch + 1_370_000,
        duration,
        existing_offset: None,
        recovering: false,
        require_full_validation: true,
    }
}

#[test]
#[ignore = "Synthetic FFmpeg camera pipeline and owned teardown fixture; no capture devices."]
fn media_contract_slow_graph_camera_preserves_event_time_and_validates_delayed_teardown() {
    let ffmpeg = bundled_ffmpeg();
    let directory = tempfile::tempdir().unwrap();
    let mut command = process::command(&ffmpeg);
    command.args(["-f", "lavfi", "-i", "color=red:size=320x180:rate=30:duration=3,drawbox=x=0:y=0:w=iw:h=ih:color=blue:t=fill:enable='gte(n,60)',settb=expr=1/10000000,setpts='N*10000000/28.8+if(gte(N,30),800000,0)'"]);
    let epoch = crate::qpc_now();
    let mut camera = CameraWorker::spawn(directory.path(), command, Some(epoch)).unwrap();
    let waiting = Instant::now();
    while camera.child.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(waiting.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(10));
    }
    camera.stop().unwrap();
    assert!(!camera.requires_full_validation());
    let info = media::probe_video(&ffmpeg, &camera.raw_path).unwrap();
    assert!(
        (3.17..3.27).contains(&info.duration),
        "Graph-clock span, not nominal 3 seconds: {}",
        info.duration
    );
    let pixels = process::read_output(
        &ffmpeg,
        &[
            "-v".into(),
            "error".into(),
            "-i".into(),
            camera.raw_path.to_string_lossy().into_owned(),
            "-map".into(),
            "0:v:0".into(),
            "-an".into(),
            "-vf".into(),
            "scale=8:8".into(),
            "-frames:v".into(),
            "120".into(),
            "-fps_mode".into(),
            "passthrough".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgb24".into(),
            "pipe:1".into(),
        ],
        Duration::from_secs(15),
    )
    .unwrap();
    let color = |frame: usize| &pixels[frame * 8 * 8 * 3..frame * 8 * 8 * 3 + 3];
    assert!(
        color(61)[0] > 230 && color(61)[2] < 20,
        "The event must not advance to nominal frame time 2 seconds."
    );
    assert!(
        color(67)[0] < 20 && color(67)[2] > 230,
        "Blue must appear after its graph time 2.1633 seconds."
    );
    let original = std::fs::read(&camera.raw_path).unwrap();
    replace_with_lingering_child(&mut camera);
    let stopped = Instant::now();
    camera.stop().unwrap();
    assert!(stopped.elapsed() < Duration::from_secs(4));
    assert!(camera.requires_full_validation());
    crate::cleanup::ensure_recovery_ready(directory.path()).unwrap();
    let published = std::cell::Cell::new(false);
    let result = finalize_recording(
        &camera.raw_path,
        &ffmpeg,
        timing(epoch, info.duration - 0.137),
        |offset| {
            assert!((offset - 0.137).abs() < 1e-9);
            published.set(true);
            Ok(())
        },
    )
    .unwrap();
    assert!(published.get());
    let validation_log = directory.path().join("camera-validate.log");
    assert!(
        validation_log.is_file(),
        "Forced teardown must execute full decode validation."
    );
    assert!(std::fs::read_to_string(validation_log).unwrap().is_empty());
    assert!(result.warning.is_none());
    assert_eq!(std::fs::read(&camera.raw_path).unwrap(), original);
    assert!(result.path.is_file());
    assert!(!directory.path().join("recovery-warning.txt").exists());
    println!("Production H264_MF camera pipeline retained slow/irregular graph timestamps and color-event timing; forced completed teardown was reaped, fully decoded, and published with a 137 ms offset.");
}

#[test]
#[ignore = "Synthetic invalid camera file and owned teardown fixture; no capture devices."]
fn media_contract_completed_but_invalid_camera_never_publishes() {
    let ffmpeg = bundled_ffmpeg();
    let directory = tempfile::tempdir().unwrap();
    let mut camera = CameraWorker::fixture(directory.path());
    camera.stop().unwrap();
    std::fs::write(&camera.raw_path, [0u8; 1024]).unwrap();
    replace_with_lingering_child(&mut camera);
    camera.stop().unwrap();
    assert!(camera.requires_full_validation());
    let published = std::cell::Cell::new(false);
    assert!(
        finalize_recording(&camera.raw_path, &ffmpeg, timing(10_000_000, 1.0), |_| {
            published.set(true);
            Ok(())
        })
        .is_err()
    );
    assert!(!published.get());
    assert!(!directory.path().join("camera-source.mp4").exists());
    assert_eq!(std::fs::read(&camera.raw_path).unwrap(), [0u8; 1024]);
}
