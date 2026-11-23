use super::*;
use screenrecord_core::{ExportPreset, WHATSAPP_MAX_BYTES};

fn compact_options(directory: &Path, name: &str) -> ExportOptions {
    ExportOptions {
        output_path: directory.join(name).to_string_lossy().into_owned(),
        resolution: "auto".into(),
        fps: 30,
        preset: ExportPreset::Whatsapp,
    }
}

fn faststart(path: &str) {
    let data = fs::read(path).unwrap();
    let mut position = 0;
    let mut moov = None;
    let mut mdat = None;
    while position + 8 <= data.len() {
        let size = u32::from_be_bytes(data[position..position + 4].try_into().unwrap()) as usize;
        let (size, header) = if size == 1 {
            (
                u64::from_be_bytes(data[position + 8..position + 16].try_into().unwrap()) as usize,
                16,
            )
        } else {
            (size, 8)
        };
        assert!(size >= header && position + size <= data.len());
        match &data[position + 4..position + 8] {
            b"moov" => moov = Some(position),
            b"mdat" => mdat = Some(position),
            _ => {}
        }
        position += size;
    }
    assert_eq!(position, data.len());
    assert!(
        moov.unwrap() < mdat.unwrap(),
        "MP4 metadata must precede media data"
    );
}

#[test]
fn whatsapp_exports_keep_aspect_audio_faststart_and_report_actual_file_size() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let original = fs::read(&project.screen_path).unwrap();
    project.edits.aspect_ratio = "16:9".into();
    project.edits.background_mode = "videoBlur".into();
    let mut standard = options(directory.path(), "standard.mp4");
    standard.fps = 30;
    export(
        &project,
        &standard,
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let standard_bytes = fs::metadata(&standard.output_path).unwrap().len();
    run(media_command(&ffmpeg)
        .arg("-i")
        .arg(&standard.output_path)
        .args(["-f", "null", "-"]));
    for (aspect, width, height) in [("16:9", 1280, 720), ("9:16", 720, 1280), ("1:1", 720, 720)] {
        project.edits.aspect_ratio = aspect.into();
        let options = compact_options(
            directory.path(),
            &format!("whatsapp-{}.mp4", aspect.replace(':', "-")),
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        export(
            &project,
            &options,
            &ffmpeg,
            Arc::new(AtomicBool::new(false)),
            move |event| sink.lock().unwrap().push(event),
        )
        .unwrap();
        let bytes = fs::metadata(&options.output_path).unwrap().len();
        assert!(bytes <= WHATSAPP_MAX_BYTES);
        let info = probe_media(&ffmpeg, Path::new(&options.output_path)).unwrap();
        let video = info.video().unwrap();
        assert_eq!((video.width, video.height), (Some(width), Some(height)));
        assert_eq!(video.codec_name.as_deref(), Some("h264"));
        assert_eq!(video.pix_fmt.as_deref(), Some("yuv420p"));
        assert_eq!(video.avg_frame_rate.as_deref(), Some("30/1"));
        assert_eq!(video.nb_frames.as_deref(), Some("72"));
        assert!(info.has_audio());
        let audio = audio_samples(&ffmpeg, &options.output_path);
        let energy = audio
            .as_chunks::<4>()
            .0
            .iter()
            .map(|sample| f32::from_le_bytes(*sample) as f64)
            .map(|value| value * value)
            .sum::<f64>()
            / (audio.len() / 4) as f64;
        assert!(energy.sqrt() > 0.01);
        faststart(&options.output_path);
        run(media_command(&ffmpeg)
            .arg("-i")
            .arg(&options.output_path)
            .args(["-f", "null", "-"]));
        let events = events.lock().unwrap();
        let completed = events.last().unwrap();
        assert_eq!(completed.state, "completed");
        assert_eq!(completed.output_bytes, Some(bytes));
        assert!(events
            .iter()
            .filter(|event| event.state != "completed")
            .all(|event| event.output_bytes.is_none()));
        assert!(events
            .windows(2)
            .all(|pair| pair[1].progress >= pair[0].progress));
        if aspect == "16:9" {
            assert!(
                bytes < standard_bytes,
                "Compact {bytes} should be smaller than Standard {standard_bytes}"
            );
            eprintln!("Same project complete exports: Standard {standard_bytes} bytes; WhatsApp {bytes} bytes, 1280x720, 30 fps, stereo AAC.");
        }
    }
    assert_eq!(fs::read(&project.screen_path).unwrap(), original);
}

#[test]
fn whatsapp_size_retry_is_bounded_reuses_audio_and_never_publishes_oversize() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let project = fixture(&ffmpeg, directory.path());
    let baseline = compact_options(directory.path(), "baseline.mp4");
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    let encoder = process::software_encoder_for_bitrate(
        &ffmpeg,
        process::VideoSpec {
            width: 1280,
            height: 720,
            fps: 30,
        },
        &AtomicBool::new(false),
        Some(2_400_000),
    )
    .unwrap();
    let first = export_with_limits(
        &project,
        &baseline,
        &ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        ExportRun {
            chunk_frames: 900,
            max_bytes: Some(WHATSAPP_MAX_BYTES),
            encoder: Some(encoder),
        },
    )
    .unwrap();
    let cap = first * 95 / 100;
    eprintln!(
        "Compact retry setup: encoder={encoder:?}, first={first}, cap={cap}, lower={:?}",
        compact::retry_bitrate(2_400_000, first, cap, 2.4, true)
    );
    let retry = compact_options(directory.path(), "retry.mp4");
    let progress = Arc::new(Mutex::new(Vec::new()));
    let sink = progress.clone();
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(move |event| {
        sink.lock().unwrap().push(event.progress)
    })));
    let bytes = export_with_limits(
        &project,
        &retry,
        &ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        ExportRun {
            chunk_frames: 900,
            max_bytes: Some(cap),
            encoder: Some(encoder),
        },
    )
    .unwrap();
    assert!(bytes <= cap && bytes < first);
    let values = progress.lock().unwrap();
    assert!(values.iter().any(|value| *value > 0.8));
    assert!(values.windows(2).all(|pair| pair[1] >= pair[0]));
    drop(values);
    assert_eq!(
        audio_samples(&ffmpeg, &baseline.output_path),
        audio_samples(&ffmpeg, &retry.output_path)
    );
    faststart(&retry.output_path);
    for path in [&baseline.output_path, &retry.output_path] {
        run(media_command(&ffmpeg)
            .arg("-i")
            .arg(path)
            .args(["-f", "null", "-"]));
    }
    for cancel_retry in [true, false] {
        let guarded = compact_options(
            directory.path(),
            if cancel_retry {
                "cancel-retry.mp4"
            } else {
                "race-retry.mp4"
            },
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let callback_cancel = cancel.clone();
        let touched = Arc::new(AtomicBool::new(false));
        let callback_touched = touched.clone();
        let output = guarded.output_path.clone();
        let callback: process::Progress = Arc::new(Mutex::new(Box::new(move |event| {
            if event.progress > 0.8 && !callback_touched.swap(true, Ordering::Relaxed) {
                if cancel_retry {
                    callback_cancel.store(true, Ordering::Relaxed);
                } else {
                    fs::write(&output, b"existing destination").unwrap();
                }
            }
        })));
        let error = export_with_limits(
            &project,
            &guarded,
            &ffmpeg,
            &cancel,
            &callback,
            ExportRun {
                chunk_frames: 900,
                max_bytes: Some(cap),
                encoder: Some(encoder),
            },
        )
        .unwrap_err();
        assert!(
            touched.load(Ordering::Relaxed),
            "Retry phase was not reached"
        );
        if cancel_retry {
            assert!(error.contains("cancelled"), "{error}");
            assert!(!Path::new(&guarded.output_path).exists());
        } else {
            assert!(error.contains("without overwriting"), "{error}");
            assert_eq!(
                fs::read(&guarded.output_path).unwrap(),
                b"existing destination"
            );
        }
    }
    progress.lock().unwrap().clear();
    let mut silent = project.clone();
    silent.microphone_path = None;
    silent.system_audio_path = None;
    let failed = compact_options(directory.path(), "still-too-large.mp4");
    let error = export_with_limits(
        &silent,
        &failed,
        &ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        ExportRun {
            chunk_frames: 900,
            max_bytes: Some(74_000),
            encoder: Some(encoder),
        },
    )
    .unwrap_err();
    assert!(error.contains("Shorten") && error.contains("Standard"));
    assert!(
        progress.lock().unwrap().iter().any(|value| *value > 0.8),
        "The bounded lower-rate attempt must actually run"
    );
    assert!(!Path::new(&failed.output_path).exists());
    assert!(!fs::read_dir(directory.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".screenrecord-export-")));
    eprintln!("Compact real size retry: first {first} bytes, injected cap {cap}, final {bytes} bytes; AAC samples identical; 74000-byte failure published nothing; retry cancellation and publication race preserved destinations.");
}

#[test]
fn whatsapp_bitrate_profiles_encode_complete_media_on_available_encoders() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.microphone_path = None;
    project.system_audio_path = None;
    let options = compact_options(directory.path(), "rates.mp4");
    let cancel = AtomicBool::new(false);
    let work = directory.path().join("rate-work");
    fs::create_dir(&work).unwrap();
    let graph = graph::RenderGraph::prepare(&project, &options, &ffmpeg, &work, &cancel).unwrap();
    let chunks = graph.chunks(900).unwrap();
    assert_eq!(chunks.len(), 1);
    let mut tested = Vec::new();
    for encoder in [
        process::Encoder::Nvidia,
        process::Encoder::Intel,
        process::Encoder::Amd,
        process::Encoder::MediaFoundation,
        process::Encoder::SoftwareMf,
        process::Encoder::SoftwareX264,
    ] {
        let mut probe = media_command(&ffmpeg);
        probe
            .args([
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=1280x720:r=30:d=0.1",
                "-frames:v",
                "3",
                "-an",
            ])
            .args(encoder.args_for_bitrate(Some(320_000)))
            .args(["-f", "null", "-"]);
        if !process::capture(&mut probe, Duration::from_secs(8), &cancel).is_ok_and(|(ok, _, _)| ok)
        {
            eprintln!("Compact encoder unavailable on this machine: {encoder:?}");
            continue;
        }
        let mut sizes = Vec::new();
        for bitrate in [320_000, 2_400_000] {
            run(&mut graph
                .video_command_with_rate(
                    &project,
                    &options,
                    &ffmpeg,
                    (encoder, Some(bitrate)),
                    chunks[0],
                    &cancel,
                )
                .unwrap());
            run(&mut graph
                .mux_command(&project, &options, &ffmpeg, &chunks)
                .unwrap());
            let output = work.join("encoded.mp4");
            let info = probe_media(&ffmpeg, &output).unwrap();
            validate_output(
                &info,
                &graph.layout,
                graph.frame_count,
                30,
                graph.duration,
                false,
            )
            .unwrap();
            assert_eq!(info.video().unwrap().pix_fmt.as_deref(), Some("yuv420p"));
            let stream: serde_json::Value =
                serde_json::from_slice(&run(Command::new(ffprobe_path(&ffmpeg))
                    .args(["-v", "error", "-show_streams", "-of", "json"])
                    .arg(&output)))
                .unwrap();
            for field in ["color_space", "color_transfer", "color_primaries"] {
                assert_eq!(stream["streams"][0][field], "bt709");
            }
            assert_eq!(stream["streams"][0]["color_range"], "tv");
            run(media_command(&ffmpeg)
                .arg("-i")
                .arg(&output)
                .args(["-f", "null", "-"]));
            faststart(&output.to_string_lossy());
            sizes.push(fs::metadata(output).unwrap().len());
        }
        assert!(
            sizes[0] < sizes[1],
            "{encoder:?} did not reduce file size: {sizes:?}"
        );
        eprintln!("Compact {encoder:?}: 320k={} bytes, 2400k={} bytes; complete 72-frame decode and BT.709 tags", sizes[0], sizes[1]);
        tested.push(encoder);
    }
    assert!(
        tested.iter().any(|encoder| encoder.is_software()),
        "At least one software rate profile must encode successfully"
    );
}
