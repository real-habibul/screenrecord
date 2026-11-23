use super::*;

#[test]
fn local_parser_retains_cli_word_offsets_and_sanitizes_failures() {
    let bytes = br#"{"transcription":[{"text":" Hello","offsets":{"from":120,"to":360}},{"text":" world.","offsets":{"from":400,"to":810}}]}"#;
    let words = parse_words(bytes, 1.0).unwrap();
    assert_eq!(words.len(), 2);
    assert_eq!(
        (words[0].start, words[0].end, words[0].text.as_str()),
        (0.12, 0.36, "Hello")
    );
    for bytes in [
        br#"{"transcription":[{"text":"sensitive text","offsets":{"from":0,"to":500}}]}"#
            .as_slice(),
        br#"{"transcription":[{"text":"sensitive","offsets":{"from":500,"to":0}}]}"#.as_slice(),
    ] {
        let error = parse_words(bytes, 1.0).unwrap_err();
        assert!(!error.contains("sensitive"));
    }
    assert!(parse_words(br#"{"transcription":[]}"#, 1.0)
        .unwrap()
        .is_empty());
}

#[test]
fn zero_duration_cli_segments_merge_only_with_adjacent_timed_words() {
    let trailing = br#"{"transcription":[{"text":" subtitle","offsets":{"from":1500,"to":3040}},{"text":" test.","offsets":{"from":3040,"to":3040}}]}"#;
    let words = parse_words(trailing, 3.2).unwrap();
    assert_eq!(
        (words[0].start, words[0].end, words[0].text.as_str()),
        (1.5, 3.04, "subtitle test.")
    );
    let leading = br#"{"transcription":[{"text":" Hello","offsets":{"from":100,"to":100}},{"text":" there","offsets":{"from":100,"to":500}}]}"#;
    assert_eq!(parse_words(leading, 1.0).unwrap()[0].text, "Hello there");
    let separate = br#"{"transcription":[{"text":" a","offsets":{"from":100,"to":300}},{"text":" b","offsets":{"from":500,"to":500}}]}"#;
    assert!(parse_words(separate, 1.0).is_err());
}

#[test]
fn progress_accepts_only_bounded_numeric_progress() {
    assert_eq!(
        progress_value(b"whisper_print_progress_callback: progress =  35%"),
        Some(0.35)
    );
    for line in [
        b"private words = 25%".as_slice(),
        b"progress = 101%",
        b"progress = -1%",
        b"progress = secret",
    ] {
        assert_eq!(progress_value(line), None);
    }
}

#[test]
fn local_setup_missing_and_traversal_are_read_only() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("absent");
    assert!(!status(&missing).unwrap().ready);
    assert!(!missing.exists());
    std::fs::write(
        root.path().join("active.json"),
        br#"{"version":1,"directory":"../escape"}"#,
    )
    .unwrap();
    let state = status(root.path()).unwrap();
    assert!(!state.ready && state.error.is_some());
    assert_eq!(
        std::fs::read(root.path().join("active.json")).unwrap(),
        br#"{"version":1,"directory":"../escape"}"#
    );
}

#[tokio::test]
async fn already_cancelled_local_setup_does_not_create_files_or_connect() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("absent");
    assert_eq!(
        install(target.clone(), Arc::new(AtomicBool::new(true)), |_| {})
            .await
            .unwrap_err(),
        CANCELLED
    );
    assert!(!target.exists());
}

#[test]
fn generated_word_metadata_matches_text_and_preserves_provider_timing() {
    let words = vec![
        Word {
            start: 10.2,
            end: 10.6,
            text: "Halo".into(),
        },
        Word {
            start: 10.7,
            end: 11.0,
            text: "dunia.".into(),
        },
    ];
    let cues = cues::build(words, 10.0, 12.0).unwrap();
    assert_eq!(
        cues[0]
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        cues[0].text
    );
    assert_eq!((cues[0].words[0].start, cues[0].words[0].end), (10.2, 10.6));
    assert!(cues[0].source_ranges.is_empty());
    let bad = vec![
        Word {
            start: 0.0,
            end: 0.6,
            text: "one".into(),
        },
        Word {
            start: 0.5,
            end: 0.8,
            text: "two".into(),
        },
    ];
    assert!(cues::build(bad, 0.0, 1.0)
        .unwrap_err()
        .contains("overlapping"));
}

#[test]
fn local_futures_are_send_even_with_non_sync_progress_callbacks() {
    fn assert_send(_: impl std::future::Future + Send) {}
    let counter = std::cell::Cell::new(0.0);
    assert_send(install(
        PathBuf::from("missing"),
        Arc::new(AtomicBool::new(false)),
        move |value| counter.set(value),
    ));
    let counter = std::cell::Cell::new(0.0);
    assert_send(transcribe(
        TranscriptionInput {
            audio_path: "missing.wav".into(),
            ffmpeg_path: "missing.exe".into(),
            start: 0.0,
            end: 1.0,
            ranges: vec![],
            language: "auto".into(),
            api_key: String::new(),
        },
        PathBuf::from("missing"),
        Arc::new(AtomicBool::new(false)),
        move |value| counter.set(value),
    ));
}

#[test]
#[ignore = "Owned subprocess fixture; never run directly."]
fn local_process_fixture() {
    let pid = std::env::var_os("SCREENRECORD_LOCAL_PID").unwrap();
    std::fs::write(pid, std::process::id().to_string()).unwrap();
    if let Some(path) = std::env::var_os("SCREENRECORD_LOCAL_OVERSIZE") {
        std::fs::File::create(path)
            .unwrap()
            .set_len(MAX_JSON_BYTES + 1)
            .unwrap();
    }
    std::thread::sleep(Duration::from_secs(30));
}

#[tokio::test]
async fn local_cancel_and_output_overflow_kill_and_reap_only_the_owned_child() {
    use std::sync::atomic::Ordering;
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    for oversized in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let pid_file = root.path().join("pid");
        let json = root.path().join("words.json");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "local::tests::local_process_fixture",
                "--nocapture",
            ])
            .env("SCREENRECORD_LOCAL_PID", &pid_file)
            .creation_flags(0x0800_0000)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if oversized {
            command.env("SCREENRECORD_LOCAL_OVERSIZE", &json);
        }
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let operation = run_cli(&mut command, &json, 1.0, &cancel, |_| {});
        let trigger = async {
            if !oversized {
                tokio::time::sleep(Duration::from_millis(150)).await;
                cancel.store(true, Ordering::Release);
            }
        };
        let (result, _) = tokio::join!(operation, trigger);
        let error = result.unwrap_err();
        assert!(if oversized {
            error.contains("size")
        } else {
            error == CANCELLED
        });
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        unsafe {
            if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut code = 0;
                GetExitCodeProcess(handle, &mut code).unwrap();
                let _ = CloseHandle(handle);
                assert_ne!(code, 259);
            }
        }
    }
}

#[tokio::test]
#[ignore = "Offline pinned Whisper model test; set SCREENRECORD_LOCAL_ASR_DIRECTORY, SCREENRECORD_LOCAL_ASR_FIXTURE and SCREENRECORD_FFMPEG. No downloads or device access."]
async fn local_model_contract_synthetic_speech_and_retained_ranges() {
    use sha2::{Digest, Sha256};
    let directory = PathBuf::from(
        std::env::var_os("SCREENRECORD_LOCAL_ASR_DIRECTORY")
            .expect("Set SCREENRECORD_LOCAL_ASR_DIRECTORY"),
    );
    let audio = PathBuf::from(
        std::env::var_os("SCREENRECORD_LOCAL_ASR_FIXTURE")
            .expect("Set SCREENRECORD_LOCAL_ASR_FIXTURE"),
    );
    let ffmpeg =
        PathBuf::from(std::env::var_os("SCREENRECORD_FFMPEG").expect("Set SCREENRECORD_FFMPEG"));
    let digest = Sha256::digest(std::fs::read(&audio).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "9df03002d3e088213b03a6201df20f131baf20c49016767ff51ab23ae8641089",
        "Use the documented SAPI fixture, never private recorded audio."
    );
    for ranges in [vec![], vec![(0.0, 3.2), (6.2, 8.318322)]] {
        let input = TranscriptionInput {
            audio_path: audio.clone(),
            ffmpeg_path: ffmpeg.clone(),
            start: 0.0,
            end: 8.318322,
            ranges: ranges.clone(),
            language: "en".into(),
            api_key: String::new(),
        };
        let started = std::time::Instant::now();
        let result = transcribe(
            input,
            directory.clone(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap();
        let text = result
            .iter()
            .map(|cue| cue.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if ranges.is_empty() {
            assert!(
                text.contains("local subtitle test")
                    && text.contains("video stays on this computer")
                    && text.contains("edit every word")
            );
        } else {
            assert!(!text.contains("video") && !text.contains("computer"));
            assert!(result.iter().all(|cue| ranges
                .iter()
                .any(|&(start, end)| cue.start >= start && cue.end <= end)));
        }
        let words = result.iter().map(|cue| cue.words.len()).sum::<usize>();
        let mut edits = screenrecord_core::EditSettings::new(8.318322);
        edits.subtitles = result;
        edits.validate(8.318322).unwrap();
        eprintln!(
            "Local offline fixture: {} ranges, {words} words, {:.3}s",
            ranges.len(),
            started.elapsed().as_secs_f64()
        );
    }
    assert_eq!(Sha256::digest(std::fs::read(audio).unwrap()), digest);
}
