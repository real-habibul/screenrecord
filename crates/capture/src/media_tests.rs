use super::*;

#[test]
#[ignore = "Executed only as an owned child by the synthetic encoder drain test."]
fn native_encoder_drain_fixture() {
    use windows::core::Interface;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
    struct Runtime;
    impl Drop for Runtime {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.unwrap();
    let _runtime = Runtime;
    let directory = PathBuf::from(
        std::env::var_os("SCREENRECORD_ENCODER_FIXTURE_DIR")
            .expect("The encoder child requires an owned output directory."),
    );
    for (stream_output, explicit_finish) in
        [(false, true), (false, false), (true, true), (true, false)]
    {
        let path = directory.join(format!("drain-{stream_output}-{explicit_finish}.mp4"));
        let stream = stream_output.then(|| InMemoryRandomAccessStream::new().unwrap());
        let video = VideoSettingsBuilder::new(320, 180)
            .sub_type(VideoSettingsSubType::H264)
            .frame_rate(30)
            .bitrate(2_000_000);
        let audio = AudioSettingsBuilder::default().disabled(true);
        let container = ContainerSettingsBuilder::default();
        let mut encoder = if let Some(stream) = &stream {
            VideoEncoder::new_from_stream(video, audio, container, stream.cast().unwrap())
        } else {
            VideoEncoder::new(video, audio, container, &path)
        }
        .unwrap();
        for index in 0..12 {
            let color = if index < 6 {
                [0, 0, 255, 255]
            } else {
                [255, 0, 0, 255]
            };
            encoder
                .send_frame_buffer(&color.repeat(320 * 180), index * 10_000_000 / 30)
                .unwrap();
        }
        if explicit_finish {
            encoder.finish().unwrap();
        } else {
            drop(encoder);
        }
        if let Some(stream) = stream {
            let size = stream.Size().unwrap();
            assert!(size > 0 && size <= 4 * 1024 * 1024);
            let reader =
                DataReader::CreateDataReader(&stream.GetInputStreamAt(0).unwrap()).unwrap();
            assert_eq!(
                reader.LoadAsync(size as u32).unwrap().join().unwrap(),
                size as u32
            );
            let mut bytes = vec![0; size as usize];
            reader.ReadBytes(&mut bytes).unwrap();
            fs::write(&path, bytes).unwrap();
        }
    }
}

#[test]
#[ignore = "Synthetic native encoder drain; requires SCREENRECORD_FFMPEG, no capture devices."]
fn media_contract_native_encoder_finish_and_drop_drain_every_queued_frame() {
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = ffmpeg();
    let log_path = directory.path().join("encoder.log");
    let log = File::create(&log_path).unwrap();
    let mut child = process::command(&std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "media_tests::native_encoder_drain_fixture",
            "--nocapture",
        ])
        .env("SCREENRECORD_ENCODER_FIXTURE_DIR", directory.path())
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let job = process::ChildJob::attach(&child)
        .inspect_err(|_| {
            let _ = child.kill();
            let _ = child.wait();
        })
        .unwrap();
    let result = process::wait_bounded(&mut child, Duration::from_secs(30));
    let _ = job.terminate();
    let _ = child.wait();
    assert!(
        result.as_ref().is_ok_and(|status| status.success()),
        "Encoder child {result:?}: {}",
        fs::read_to_string(log_path).unwrap()
    );
    for (stream_output, explicit_finish) in
        [(false, true), (false, false), (true, true), (true, false)]
    {
        let path = directory
            .path()
            .join(format!("drain-{stream_output}-{explicit_finish}.mp4"));
        let info =
            media::validate_video(&ffmpeg, &path, &directory.path().join("decode.log")).unwrap();
        assert_eq!((info.width, info.height), (320, 180));
        let pixels = process::read_output(
            &ffmpeg,
            &[
                "-v".into(),
                "error".into(),
                "-i".into(),
                path.to_string_lossy().into_owned(),
                "-map".into(),
                "0:v:0".into(),
                "-an".into(),
                "-vf".into(),
                "scale=8:8".into(),
                "-frames:v".into(),
                "20".into(),
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
        assert_eq!(
            pixels.len(),
            12 * 8 * 8 * 3,
            "Every queued frame must drain before EOS."
        );
        assert!(pixels[0] > 230 && pixels[2] < 20);
        let last = &pixels[pixels.len() - 3..];
        assert!(
            last[0] < 20 && last[2] > 230,
            "The final blue frame was lost."
        );
    }
    println!("Native encoder: file and stream constructors, each with finish and Drop, drained all 12 synthetic frames; full decode and final-frame colors verified.");
}

#[test]
#[ignore = "Offline multi-part recovery; requires SCREENRECORD_FFMPEG, no capture devices."]
fn media_contract_session_recovery_keeps_ordered_parts_and_reports_failed_resume() {
    let ffmpeg = ffmpeg();
    let directory = tempfile::tempdir().unwrap();
    let mut originals = Vec::new();
    for index in [0, 2] {
        let part = directory.path().join(format!("part-{index:05}"));
        fs::create_dir_all(part.join("segments")).unwrap();
        let segment = part.join("segments/screen-00000.mp4");
        video(&ffmpeg, &segment, 1.0);
        originals.push((segment.clone(), fs::read(&segment).unwrap()));
        let journal = Journal {
            version: 1,
            source_id: "monitor:1".into(),
            width: 320,
            height: 180,
            fps: 30,
            epoch_qpc: 10_000_000,
            camera_epoch_qpc: None,
            camera_offset_seconds: None,
            duration: 1.0,
            microphone: false,
            system_audio: false,
            segments: vec!["screen-00000.mp4".into()],
            segment_durations: vec![1.0],
        };
        write_journal(&part, &journal).unwrap();
        fs::write(
            part.join("cursor.ndjson"),
            "{\"time\":0.25,\"x\":0.5,\"y\":0.5,\"pressed\":true,\"visible\":true}\n",
        )
        .unwrap();
    }
    fs::write(directory.path().join("session-journal.json"),serde_json::to_vec(&serde_json::json!({"version":1,"dimensions":[320,180,30],"parts":[{"directory":"part-00000","duration":1.0},{"directory":"part-00001","duration":null},{"directory":"part-00002","duration":null}]})).unwrap()).unwrap();
    let recovered = session::recover_session(directory.path(), &ffmpeg).unwrap();
    assert_eq!(recovered.len(), 2);
    assert!(recovered[0].screen_path.ends_with("part-00000/screen.mp4"));
    assert!(recovered[1].screen_path.ends_with("part-00002/screen.mp4"));
    for part in &recovered {
        assert!((part.duration - 1.0).abs() < 0.05);
        assert_eq!(part.cursor[0].time, 0.25);
        media::validate_video(
            &ffmpeg,
            &part.screen_path,
            &directory.path().join("decoded.log"),
        )
        .unwrap();
    }
    assert!(
        fs::read_to_string(directory.path().join("recovery-warning.txt"))
            .unwrap()
            .contains("part-00001")
    );
    for (path, bytes) in originals {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
#[ignore = "Synthetic two-part FFmpeg recovery with unwritable output paths, no devices."]
fn media_contract_multipart_recovery_retries_writes_and_salvages_damaged_input() {
    let ffmpeg = ffmpeg();
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let first = dir.join("part-00000");
    let second = dir.join("part-00001");
    fs::create_dir_all(first.join("segments")).unwrap();
    fs::create_dir_all(second.join("segments")).unwrap();
    let mut journal = fixture_journal();
    journal.duration = 1.0;
    journal.segments.truncate(1);
    journal.segment_durations.truncate(1);
    journal.system_audio = false;
    journal.camera_epoch_qpc = None;
    for part in [&first, &second] {
        write_journal(part, &journal).unwrap();
    }
    fs::write(dir.join("session-journal.json"), br#"{"version":1,"dimensions":[320,180,30],"parts":[{"directory":"part-00000","duration":1.0},{"directory":"part-00001","duration":1.0}]}"#).unwrap();
    video(&ffmpeg, &first.join("screen.mp4"), 1.0);
    let original = second.join("segments/screen-00000.mp4");
    fs::copy(first.join("screen.mp4"), &original).unwrap();
    let original_bytes = fs::read(&original).unwrap();
    for destination in ["segments/concat.txt", "segment-validate.log"] {
        let blocked = second.join(destination);
        if blocked.is_file() {
            fs::remove_file(&blocked).unwrap();
        }
        fs::create_dir(&blocked).unwrap();
        let recovery = session::recover_session(dir, &ffmpeg);
        assert!(
            recovery.is_err(),
            "Published a partial recording after failing to write {destination}"
        );
        assert_eq!(fs::read(&original).unwrap(), original_bytes);
        assert!(!dir.join("recovery-warning.txt").exists());
        fs::remove_dir(blocked).unwrap();
    }
    journal.camera_epoch_qpc = Some(journal.epoch_qpc);
    write_journal(&second, &journal).unwrap();
    fs::copy(&original, second.join("camera.raw.mp4")).unwrap();
    let camera_log = second.join("camera-align.log");
    fs::create_dir(&camera_log).unwrap();
    assert!(session::recover_session(dir, &ffmpeg).is_err());
    assert!(!dir.join("recovery-warning.txt").exists());
    fs::remove_dir(camera_log).unwrap();
    let parts = session::recover_session(dir, &ffmpeg).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].duration + parts[1].duration, 2.0);
    assert!(parts[1].camera_path.is_some());
    fs::remove_file(second.join("screen.mp4")).unwrap();
    fs::write(&original, vec![0u8; 512]).unwrap();
    let salvaged = session::recover_session(dir, &ffmpeg).unwrap();
    assert_eq!(salvaged.len(), 1);
    assert!(salvaged[0].screen_path.ends_with("part-00000/screen.mp4"));
    assert!(fs::read_to_string(dir.join("recovery-warning.txt"))
        .unwrap()
        .contains("part-00001"));
}

#[test]
#[ignore = "Offline FFmpeg recovery with exclusive Windows sharing locks, no devices."]
fn media_contract_recovery_retries_locked_tracks_then_restores_all_inputs() {
    use std::os::windows::fs::OpenOptionsExt;

    let ffmpeg = ffmpeg();
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let mut journal = fixture_journal();
    journal.microphone = true;
    fs::create_dir(dir.join("segments")).unwrap();
    let first = dir.join("segments/screen-00000.mp4");
    video(&ffmpeg, &first, 1.0);
    fs::copy(&first, dir.join("segments/screen-00001.mp4")).unwrap();
    fs::copy(&first, dir.join("camera.raw.mp4")).unwrap();
    for name in ["microphone.wav", "system.wav"] {
        let mut audio = wav::PcmWriter::create(&dir.join(name)).unwrap();
        audio.write_frames(&[0; 800]).unwrap();
        audio.finalize().unwrap();
    }
    fs::write(
        dir.join("cursor.ndjson"),
        br#"{"time":0.25,"x":0.5,"y":0.5,"pressed":true,"visible":true}"#,
    )
    .unwrap();
    write_journal(dir, &journal).unwrap();
    let mut partial = Vec::new();
    for name in [
        "segments/screen-00001.mp4",
        "camera.raw.mp4",
        "microphone.wav",
        "system.wav",
        "cursor.ndjson",
    ] {
        let input = dir.join(name);
        let lock = File::options()
            .read(true)
            .share_mode(0)
            .open(&input)
            .unwrap();
        assert_eq!(File::open(&input).unwrap_err().raw_os_error(), Some(32));
        if recover(dir, &ffmpeg).is_ok() {
            partial.push(name);
        }
        drop(lock);
        for cached in ["screen.mp4", "camera-source.mp4", "recovery-warning.txt"] {
            let path = dir.join(cached);
            if path.is_file() {
                fs::remove_file(path).unwrap();
            }
        }
    }
    assert!(
        partial.is_empty(),
        "Recovery published without locked tracks: {partial:?}"
    );
    let result = recover(dir, &ffmpeg).unwrap();
    assert_eq!(result.duration, 2.0);
    assert_eq!(result.cursor.len(), 1);
    assert_eq!(result.cursor[0].time, 0.25);
    assert!(result.microphone_path.unwrap().ends_with("microphone.wav"));
    assert!(result.system_audio_path.unwrap().ends_with("system.wav"));
    assert!(result.camera_path.unwrap().is_file());
    fs::remove_file(dir.join("cursor.ndjson")).unwrap();
    fs::remove_file(dir.join("microphone.wav")).unwrap();
    fs::write(dir.join("system.wav"), b"truncated").unwrap();
    let salvaged = recover(dir, &ffmpeg).unwrap();
    assert!(salvaged.cursor.is_empty());
    assert!(salvaged.microphone_path.is_none());
    assert!(salvaged.system_audio_path.is_none());
    assert_eq!(salvaged.duration, 2.0);
}

fn ffmpeg() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("SCREENRECORD_FFMPEG")
            .expect("Set SCREENRECORD_FFMPEG to the bundled FFmpeg executable."),
    );
    assert!(path.is_file());
    path
}

fn video(ffmpeg: &Path, path: &Path, duration: f64) {
    process::run_logged(
        ffmpeg,
        &[
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-n".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=size=320x180:rate=30".into(),
            "-t".into(),
            duration.to_string(),
            "-an".into(),
            "-vf".into(),
            "format=nv12".into(),
            "-c:v".into(),
            "h264_mf".into(),
            "-hw_encoding".into(),
            "0".into(),
            "-b:v".into(),
            "1000k".into(),
            "-bf".into(),
            "0".into(),
            "-g".into(),
            "30".into(),
            "-movflags".into(),
            "+frag_keyframe+empty_moov+default_base_moof".into(),
            path.to_string_lossy().into_owned(),
        ],
        &path.with_extension("generate.log"),
    )
    .unwrap();
}

fn packet_hashes(ffmpeg: &Path, path: &Path) -> Vec<String> {
    let bytes = process::read_output(
        &ffmpeg.with_file_name("ffprobe.exe"),
        &[
            "-v".into(),
            "error".into(),
            "-select_streams".into(),
            "v:0".into(),
            "-show_packets".into(),
            "-show_entries".into(),
            "packet=data_hash,pts_time,duration_time".into(),
            "-show_data_hash".into(),
            "sha256".into(),
            "-of".into(),
            "json".into(),
            path.to_string_lossy().into_owned(),
        ],
        Duration::from_secs(10),
    )
    .unwrap();
    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["packets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|packet| {
            format!(
                "{}:{}:{}",
                packet["data_hash"], packet["pts_time"], packet["duration_time"]
            )
        })
        .collect()
}

#[test]
#[ignore = "Offline media contract test; requires SCREENRECORD_FFMPEG, no capture devices."]
fn media_contract_camera_remux_preserves_packets_offset_legacy_and_partial_media() {
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = ffmpeg();
    let raw = directory.path().join("camera.raw.mp4");
    video(&ffmpeg, &raw, 3.0);
    let original = fs::read(&raw).unwrap();
    let expected = packet_hashes(&ffmpeg, &raw);
    let mut published = None;
    let timing = || camera::CameraTiming {
        camera_epoch: 10_000_000,
        screen_epoch: 15_000_000,
        duration: 4.0,
        existing_offset: None,
        recovering: false,
        require_full_validation: false,
    };
    let track = camera::finalize_recording(&raw, &ffmpeg, timing(), |offset| {
        published = Some(offset);
        Ok(())
    })
    .unwrap();
    assert_eq!(track.offset, 0.5);
    assert_eq!(published, Some(0.5));
    assert!(track.warning.unwrap().contains("2.500"));
    assert_eq!(packet_hashes(&ffmpeg, &track.path), expected);
    assert_eq!(fs::read(&raw).unwrap(), original);
    let mut recovery_timing = timing();
    recovery_timing.existing_offset = published;
    recovery_timing.recovering = true;
    let recovered = camera::finalize_recording(&raw, &ffmpeg, recovery_timing, |_| {
        panic!("Existing source must not be republished.")
    })
    .unwrap();
    assert_eq!(recovered.offset, 0.5);
    fs::rename(&track.path, directory.path().join("camera.mp4")).unwrap();
    let legacy = camera::finalize_recording(&raw, &ffmpeg, timing(), |_| {
        panic!("Legacy media must retain offset zero.")
    })
    .unwrap();
    assert_eq!(legacy.offset, 0.0);
    assert_eq!(legacy.path.file_name().unwrap(), "camera.mp4");
    println!("Camera stream-copy: {} packet hashes unchanged; offset 0.5; raw unchanged; partial retained; legacy offset 0.", expected.len());
}

fn fixture_journal() -> Journal {
    Journal {
        version: 1,
        source_id: "monitor:1".into(),
        width: 320,
        height: 180,
        fps: 30,
        epoch_qpc: 15_000_000,
        camera_epoch_qpc: Some(10_000_000),
        camera_offset_seconds: None,
        duration: 2.0,
        microphone: false,
        system_audio: true,
        segments: vec!["screen-00000.mp4".into(), "screen-00001.mp4".into()],
        segment_durations: vec![1.0, 1.0],
    }
}

#[test]
#[ignore = "Offline media contract test; requires SCREENRECORD_FFMPEG, no capture devices."]
fn media_contract_recovery_rebuilds_invalid_finals_and_keeps_good_screen_without_audio() {
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = ffmpeg();
    let path = directory.path();
    fs::create_dir(path.join("segments")).unwrap();
    let segment = path.join("segments/screen-00000.mp4");
    video(&ffmpeg, &segment, 1.0);
    let original = fs::read(&segment).unwrap();
    fs::copy(&segment, path.join("segments/screen-00001.mp4")).unwrap();
    fs::copy(&segment, path.join("camera.raw.mp4")).unwrap();
    fs::write(path.join("screen.mp4"), vec![0u8; 512]).unwrap();
    fs::write(path.join("camera.mp4"), vec![0u8; 512]).unwrap();
    let mut audio = wav::PcmWriter::create(&path.join("system.wav")).unwrap();
    audio.write_frames(&[0u8; 800]).unwrap();
    audio.finalize().unwrap();
    File::options()
        .write(true)
        .open(path.join("system.wav"))
        .unwrap()
        .set_len(100)
        .unwrap();
    write_journal(path, &fixture_journal()).unwrap();
    let result = recover(path, &ffmpeg).unwrap();
    assert_eq!(result.duration, 2.0);
    assert_eq!(result.camera_offset_seconds, 0.5);
    assert!(result.system_audio_path.is_none());
    assert!(result.camera_path.is_some());
    media::validate_video(
        &ffmpeg,
        &result.screen_path,
        &path.join("assert-decode.log"),
    )
    .unwrap();
    assert_eq!(fs::read(&segment).unwrap(), original);
    assert_eq!(fs::read(path.join("camera.raw.mp4")).unwrap(), original);
    assert!(path.join("screen.invalid-0000.mp4").exists());
    assert!(path.join("camera.invalid-0000.mp4").exists());
    let warning = fs::read_to_string(path.join("recovery-warning.txt")).unwrap();
    assert!(
        warning.contains("system audio")
            && warning.contains("webcam ends")
            && warning.contains("invalid screen")
    );
    println!("Recovery: decoded rebuilt 2-second screen; invalid finals preserved; corrupt WAV omitted; 0.5-second camera tail retained with warning.");
}

#[test]
#[ignore = "Offline media contract test; requires SCREENRECORD_FFMPEG, no capture devices."]
fn media_contract_recovery_keeps_decodable_prefix_and_rejects_incomplete_publication() {
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = ffmpeg();
    let path = directory.path();
    fs::create_dir(path.join("segments")).unwrap();
    video(&ffmpeg, &path.join("segments/screen-00000.mp4"), 1.0);
    fs::write(path.join("segments/screen-00001.mp4"), vec![0u8; 512]).unwrap();
    let mut journal = fixture_journal();
    journal.camera_epoch_qpc = None;
    journal.system_audio = false;
    write_journal(path, &journal).unwrap();
    let result = recover(path, &ffmpeg).unwrap();
    assert_eq!(result.duration, 1.0);
    let first_recovery = fs::read(&result.screen_path).unwrap();
    assert_eq!(recover(path, &ffmpeg).unwrap().duration, 1.0);
    assert_eq!(fs::read(&result.screen_path).unwrap(), first_recovery);
    assert!(!path.join("screen.invalid-0000.mp4").exists());
    assert!(fs::read_to_string(path.join("recovery-warning.txt"))
        .unwrap()
        .contains("first 1.000 seconds"));
    assert_eq!(
        fs::metadata(path.join("segments/screen-00001.mp4"))
            .unwrap()
            .len(),
        512
    );
    let raw = path.join("camera.raw.mp4");
    fs::copy(path.join("segments/screen-00000.mp4"), &raw).unwrap();
    let timing = camera::CameraTiming {
        camera_epoch: 10_000_000,
        screen_epoch: 15_000_000,
        duration: 0.5,
        existing_offset: None,
        recovering: false,
        require_full_validation: false,
    };
    assert!(camera::finalize_recording(&raw, &ffmpeg, timing, |_| Err(
        "Injected journal write failure".into()
    ))
    .is_err());
    assert!(!path.join("camera-source.mp4").exists());
    assert!(path.join("camera-source-finalizing.mp4").exists());
    assert!(raw.exists());
    println!("Damaged last chunk: recovered first second, preserved bad chunk. Failed journal publication leaves no misleading final camera path.");
}
