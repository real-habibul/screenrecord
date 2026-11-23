use super::*;
use crate::{
    cues::{self, Word},
    provider::Provider,
};
use std::{process::Stdio, time::Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const KEY: &str = "fixture-token-not-a-real-key";

fn response(words: serde_json::Value) -> String {
    serde_json::json!({"results":{"channels":[{"alternatives":[{"transcript":"Fixture speech", "words":words}]}]}}).to_string()
}

async fn mock(
    status: u16,
    body: String,
    extra: &str,
    wait: Duration,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/listen", listener.local_addr().unwrap());
    let extra = extra.to_owned();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut data = Vec::new();
        let header_end = loop {
            let byte = socket.read_u8().await.unwrap();
            data.push(byte);
            assert!(data.len() < 16384);
            if data.ends_with(b"\r\n\r\n") {
                break data.len();
            }
        };
        let headers = String::from_utf8(data).unwrap();
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|value| value.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        let mut audio = vec![0u8; length];
        socket.read_exact(&mut audio).await.unwrap();
        assert!(header_end > 0 && !audio.is_empty());
        tokio::time::sleep(wait).await;
        let length = if extra.contains("X-Fixture-No-Length") {
            String::new()
        } else {
            format!("Content-Length: {}\r\n", body.len())
        };
        let message = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\n{length}Connection: close\r\n{extra}\r\n{body}");
        let _ = socket.write_all(message.as_bytes()).await;
        headers
    });
    (endpoint, task)
}

fn wav(path: &std::path::Path, seconds: f64) {
    let frames = (seconds * 16000.0) as usize;
    let samples: Vec<_> = (0..frames)
        .map(|frame| {
            ((frame as f64 * std::f64::consts::TAU * 440.0 / 16000.0).sin() * 500.0) as i16
        })
        .collect();
    pcm_wav(path, 16_000, &samples);
}

fn pcm_wav(path: &std::path::Path, rate: u32, samples: &[i16]) {
    let size = (samples.len() * 2) as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&size.to_le_bytes());
    for value in samples {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

fn input_with_ranges(ranges: Vec<(f64, f64)>) -> TranscriptionInput {
    TranscriptionInput {
        audio_path: "missing.wav".into(),
        ffmpeg_path: "missing.exe".into(),
        start: 1.0,
        end: 1305.0,
        ranges,
        language: "auto".into(),
        api_key: KEY.into(),
    }
}

#[test]
fn retained_ranges_reject_ambiguous_order_overlap_nonfinite_and_outside_trim() {
    for ranges in [
        vec![(0.9, 2.0)],
        vec![(1.0, 1306.0)],
        vec![(2.0, 3.0), (1.0, 1.5)],
        vec![(1.0, 3.0), (2.9, 4.0)],
        vec![(1.0, 1.0)],
        vec![(1.0, 1.09)],
        vec![(f64::NAN, 3.0)],
        vec![(1.0, f64::INFINITY)],
        vec![(1.0, 2.0); 513],
    ] {
        assert!(retained_ranges(&input_with_ranges(ranges)).is_err());
    }
    assert_eq!(
        retained_ranges(&input_with_ranges(vec![])).unwrap(),
        [(1.0, 1305.0)]
    );
    assert_eq!(
        retained_ranges(&input_with_ranges(vec![(1.0, 1.05), (2.0, 2.05)]))
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        retained_ranges(&input_with_ranges(vec![
            (1.0, 2.0),
            (2.0, 3.0),
            (3.000000001, 4.0)
        ]))
        .unwrap(),
        vec![(1.0, 3.0), (3.000000001, 4.0)]
    );
    assert_eq!(
        retained_ranges(&input_with_ranges(vec![(1.0, 1.001), (2.0, 2.2)])).unwrap(),
        vec![(2.0, 2.2)]
    );
}

#[test]
fn context_is_confined_to_each_retained_range_and_cue_tails_stay_inside_it() {
    let ranges = retained_ranges(&input_with_ranges(vec![(10.0, 700.0), (701.0, 1305.0)])).unwrap();
    for (start, end) in ranges {
        let chunks = chunks(start, end);
        assert_eq!(chunks[0].start, start);
        assert_eq!(chunks.last().unwrap().end, end);
        assert!(chunks.iter().all(|chunk| chunk.start >= start
            && chunk.end <= end
            && chunk.end - chunk.start <= 600.0));
        assert!(
            (chunks
                .iter()
                .map(|chunk| chunk.owned_end - chunk.owned_start)
                .sum::<f64>()
                - (end - start))
                .abs()
                < 1e-9
        );
    }
    let chunk = chunks(3.0, 3.2)[0];
    let words = source_words(
        vec![Word {
            start: 0.19,
            end: 0.205,
            text: "kept".into(),
        }],
        &chunk,
        3.0,
        3.2,
    )
    .collect();
    let cues = cues::build(words, 3.0, 3.2).unwrap();
    assert_eq!(cues[0].end, 3.2);
}

#[test]
fn chunks_cover_the_source_once_with_context_and_a_hard_600_second_bound() {
    let chunks = chunks(12.25, 12.25 + 1801.0);
    assert_eq!(chunks.len(), 4);
    assert_eq!(chunks.first().unwrap().start, 12.25);
    assert_eq!(chunks.last().unwrap().end, 1813.25);
    assert!(chunks.iter().all(|chunk| chunk.end - chunk.start <= 600.0));
    for pair in chunks.windows(2) {
        assert_eq!(pair[0].owned_end, pair[1].owned_start);
        assert_eq!(pair[0].end - pair[1].start, 4.0);
    }
    let owned: f64 = chunks
        .iter()
        .map(|chunk| chunk.owned_end - chunk.owned_start)
        .sum();
    assert_eq!(owned, 1801.0);
}

#[test]
fn context_words_at_a_chunk_boundary_map_to_source_time_once() {
    let pieces = chunks(10.0, 1210.0);
    let first = vec![
        Word {
            start: 595.0,
            end: 595.5,
            text: "sebelum".into(),
        },
        Word {
            start: 595.8,
            end: 596.2,
            text: "batas".into(),
        },
    ];
    let second = vec![
        Word {
            start: 1.0,
            end: 1.5,
            text: "sebelum".into(),
        },
        Word {
            start: 1.8,
            end: 2.2,
            text: "batas".into(),
        },
        Word {
            start: 2.4,
            end: 2.8,
            text: "sesudah".into(),
        },
    ];
    let words: Vec<_> = source_words(first, &pieces[0], 10.0, 1210.0)
        .chain(source_words(second, &pieces[1], 10.0, 1210.0))
        .collect();
    assert_eq!(
        words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>(),
        vec!["sebelum", "batas", "sesudah"]
    );
    assert_eq!(words[0].start, 605.0);
    assert!((words[1].start - 605.8).abs() < 1e-9);
    assert!((words[2].start - 606.4).abs() < 1e-9);
}

#[test]
fn public_job_future_is_send_and_requires_no_runtime_until_polled() {
    fn assert_send(_: impl std::future::Future + Send) {}
    assert_send(transcribe(
        TranscriptionInput {
            audio_path: "missing.wav".into(),
            ffmpeg_path: "missing.exe".into(),
            start: 0.0,
            end: 1.0,
            ranges: vec![],
            language: "auto".into(),
            api_key: KEY.into(),
        },
        Arc::new(AtomicBool::new(false)),
        |_| {},
    ));
}

#[test]
fn provider_parser_rejects_missing_invalid_or_unordered_timestamps_without_echoing_content() {
    for words in [
        serde_json::json!([{ "word": "secret", "start": -1.0, "end": 1.0 }]),
        serde_json::json!([{ "word": "secret", "start": 0.0, "end": 2.2 }]),
        serde_json::json!([{ "word": "secret", "start": 0.1, "end": 0.1 }]),
        serde_json::json!([{ "word": "secret", "start": 0.9, "end": 1.0 }, {"word":"secret", "start":0.2,"end":0.4}]),
        serde_json::json!([]),
    ] {
        let error = provider::parse(response(words).as_bytes(), 2.0).unwrap_err();
        assert!(!error.contains("secret") && !error.contains(KEY));
    }
    assert!(provider::parse(KEY.as_bytes(), 2.0)
        .unwrap_err()
        .contains("invalid transcription"));
    let silent = br#"{"results":{"channels":[{"alternatives":[{"transcript":"","words":[]}]}]}}"#;
    assert!(provider::parse(silent, 2.0).unwrap().is_empty());
}

#[test]
fn cue_grouping_preserves_unicode_words_and_produces_nonoverlapping_source_times() {
    let words: Vec<_> = (0..60)
        .map(|index| Word {
            start: 10.0 + index as f64 * 0.3,
            end: 10.25 + index as f64 * 0.3,
            text: format!("kata{index}é"),
        })
        .collect();
    let cues = cues::build(words.clone(), 10.0, 30.0).unwrap();
    assert_eq!(
        cues.iter()
            .flat_map(|cue| cue.text.split_whitespace())
            .collect::<Vec<_>>(),
        words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(cues.iter().all(|cue| cue.start >= 10.0
        && cue.end <= 30.0
        && cue.end - cue.start <= 6.0
        && cue.text.lines().count() <= 3));
    assert!(cues.windows(2).all(|pair| pair[0].end <= pair[1].start));
    assert!(cues
        .iter()
        .all(|cue| uuid::Uuid::parse_str(&cue.id).is_ok()));
    let mut edits = screenrecord_core::EditSettings::new(30.0);
    edits.subtitles = cues;
    edits.validate(30.0).unwrap();
}

#[test]
fn cue_overlap_duplicates_and_tiny_words_do_not_create_invalid_cues() {
    let result = cues::build(
        vec![
            Word {
                start: 1.0,
                end: 1.4,
                text: "Hello".into(),
            },
            Word {
                start: 1.1,
                end: 1.45,
                text: "hello".into(),
            },
            Word {
                start: 1.45,
                end: 1.7,
                text: "dunia.".into(),
            },
            Word {
                start: 1.8,
                end: 1.801,
                text: "Ya.".into(),
            },
        ],
        1.0,
        2.0,
    )
    .unwrap();
    assert_eq!(result[0].text, "Hello dunia.");
    assert_eq!(result[1].text, "Ya.");
    assert!(result.iter().all(|cue| cue.end - cue.start + 1e-9 >= 0.01));
}

#[test]
fn a_prolonged_word_preserves_its_actual_timing_without_losing_text() {
    let result = cues::build(
        vec![
            Word {
                start: 0.0,
                end: 8.0,
                text: "Wah".into(),
            },
            Word {
                start: 9.0,
                end: 9.3,
                text: "selesai.".into(),
            },
        ],
        0.0,
        10.0,
    )
    .unwrap();
    assert_eq!(result[0].text, "Wah");
    assert_eq!(result[0].end, 8.0);
    assert_eq!(result[0].words[0].end, 8.0);
    assert_eq!(result[1].text, "selesai.");
    let mut edits = screenrecord_core::EditSettings::new(10.0);
    edits.subtitles = result;
    edits.validate(10.0).unwrap();
}

#[test]
fn extraction_pcm_contract_rejects_empty_or_truncated_payload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.wav");
    wav(&path, 0.1);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        extract::pcm_duration(&bytes, bytes.len() as u64).unwrap(),
        0.1
    );
    assert!(extract::pcm_duration(&bytes, 45).is_err());
    wav(&path, 0.0);
    let empty = std::fs::read(&path).unwrap();
    assert!(extract::pcm_duration(&empty, empty.len() as u64).is_err());
}

#[test]
fn native_sample_boundary_preserves_exact_and_positive_subsample_cuts() {
    let boundary = 1.1_f64;
    assert_eq!(extract::sample_boundary(boundary, 48_000), 52_800);
    assert_eq!(
        extract::sample_boundary(f64::from_bits(boundary.to_bits() - 1), 48_000),
        52_800
    );
    assert_eq!(
        extract::sample_boundary(f64::from_bits(boundary.to_bits() + 1), 48_000),
        52_801
    );
}

#[tokio::test]
async fn http_upload_has_private_optout_auth_and_source_timestamps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("speech.wav");
    wav(&path, 2.0);
    let (endpoint, request) = mock(200, response(serde_json::json!([{ "word":"halo", "punctuated_word":"Halo.", "start":0.2,"end":0.9 }])), "", Duration::ZERO).await;
    let words = Provider::new(KEY, &endpoint, "id")
        .unwrap()
        .words(&path, 2.0, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(words[0].text, "Halo.");
    assert_eq!(words[0].start, 0.2);
    let headers = request.await.unwrap();
    assert!(
        headers.contains("mip_opt_out=true")
            && headers.contains("model=nova-3")
            && headers.contains("language=multi")
            && headers.contains("language_hint=id")
    );
    assert!(headers
        .to_ascii_lowercase()
        .contains(&format!("authorization: token {KEY}")));
    assert!(!headers.contains("speech.wav") && !headers.contains("screenrecord"));
}

#[tokio::test]
async fn http_errors_are_sanitized_and_redirects_never_forward_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("speech.wav");
    wav(&path, 0.1);
    for status in [401, 403, 402, 413, 429, 503] {
        let (endpoint, request) = mock(
            status,
            format!("sensitive transcript {KEY}"),
            "",
            Duration::ZERO,
        )
        .await;
        let error = Provider::new(KEY, &endpoint, "auto")
            .unwrap()
            .words(&path, 0.1, &AtomicBool::new(false))
            .await
            .unwrap_err();
        assert!(!error.contains(KEY) && !error.contains("sensitive"));
        request.await.unwrap();
    }
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (endpoint, request) = mock(
        307,
        String::new(),
        &format!(
            "Location: http://{}/collect\r\n",
            destination.local_addr().unwrap()
        ),
        Duration::ZERO,
    )
    .await;
    let error = Provider::new(KEY, &endpoint, "auto")
        .unwrap()
        .words(&path, 0.1, &AtomicBool::new(false))
        .await
        .unwrap_err();
    assert!(error.contains("redirect"));
    request.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), destination.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_interrupts_waiting_http_and_prevents_pre_cancelled_upload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("speech.wav");
    wav(&path, 0.1);
    let (endpoint, server) = mock(
        200,
        response(serde_json::json!([])),
        "",
        Duration::from_secs(30),
    )
    .await;
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        signal.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let provider = Provider::new(KEY, &endpoint, "auto").unwrap();
    assert_eq!(
        provider.words(&path, 0.1, &cancel).await.unwrap_err(),
        CANCELLED
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    server.abort();
    let _ = server.await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = Provider::new(
        KEY,
        &format!("http://{}", listener.local_addr().unwrap()),
        "auto",
    )
    .unwrap();
    assert_eq!(
        provider.words(&path, 0.1, &cancel).await.unwrap_err(),
        CANCELLED
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn oversized_response_is_rejected_before_parsing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("speech.wav");
    wav(&path, 0.1);
    for extra in ["", "X-Fixture-No-Length: true\r\n"] {
        let (endpoint, request) =
            mock(200, " ".repeat(8 * 1024 * 1024 + 1), extra, Duration::ZERO).await;
        let error = Provider::new(KEY, &endpoint, "auto")
            .unwrap()
            .words(&path, 0.1, &AtomicBool::new(false))
            .await
            .unwrap_err();
        assert!(error.contains("supported size"));
        request.await.unwrap();
    }
}

#[tokio::test]
async fn oversized_audio_is_rejected_before_any_connection() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("oversized.wav");
    std::fs::File::create(&audio)
        .unwrap()
        .set_len(provider::MAX_AUDIO_BYTES + 1)
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = Provider::new(
        KEY,
        &format!("http://{}", listener.local_addr().unwrap()),
        "auto",
    )
    .unwrap();
    assert!(provider
        .words(&audio, 600.0, &AtomicBool::new(false))
        .await
        .unwrap_err()
        .contains("20 MiB"));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_interrupts_a_stalled_audio_upload() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("upload.wav");
    std::fs::File::create(&audio)
        .unwrap()
        .set_len(19 * 1024 * 1024)
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (accepted, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.read_u8().await.unwrap();
        accepted.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    tokio::spawn(async move {
        received.await.unwrap();
        signal.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let error = Provider::new(KEY, &endpoint, "auto")
        .unwrap()
        .words(&audio, 600.0, &cancel)
        .await
        .unwrap_err();
    assert_eq!(error, CANCELLED);
    assert!(started.elapsed() < Duration::from_secs(1));
    server.abort();
    let _ = server.await;
}

#[test]
#[ignore = "Owned subprocess fixture; never run directly."]
fn extraction_child_fixture() {
    assert_eq!(
        std::env::var("SCREENRECORD_TRANSCRIBE_FIXTURE").as_deref(),
        Ok("wait")
    );
    std::thread::sleep(Duration::from_secs(30));
}

#[tokio::test]
async fn extraction_cancel_terminates_the_owned_process() {
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "tests::extraction_child_fixture",
            "--nocapture",
        ])
        .env("SCREENRECORD_TRANSCRIBE_FIXTURE", "wait")
        .creation_flags(0x0800_0000)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        signal.store(true, Ordering::Release);
    });
    let started = Instant::now();
    assert_eq!(
        extract::run_fixture(&mut command, &cancel)
            .await
            .unwrap_err(),
        CANCELLED
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
#[ignore = "Offline FFmpeg test; set SCREENRECORD_FFMPEG. Uses synthetic audio and localhost only."]
async fn media_contract_ffmpeg_extraction_and_mock_transcription_preserve_source_offset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("speech.wav");
    wav(&path, 3.0);
    let (endpoint, server) = mock(200, response(serde_json::json!([{ "word":"halo", "punctuated_word":"Halo", "start":0.2,"end":0.6 }, {"word":"dunia","punctuated_word":"dunia.","start":0.65,"end":1.2}])), "", Duration::ZERO).await;
    let input = TranscriptionInput {
        audio_path: path,
        ffmpeg_path: std::env::var_os("SCREENRECORD_FFMPEG")
            .expect("Set SCREENRECORD_FFMPEG")
            .into(),
        start: 1.0,
        end: 2.5,
        ranges: vec![],
        language: "id".into(),
        api_key: KEY.into(),
    };
    let cues = transcribe_at(input, Arc::new(AtomicBool::new(false)), |_| {}, &endpoint)
        .await
        .unwrap();
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].text, "Halo dunia.");
    assert!((cues[0].start - 1.2).abs() < 1e-9);
    assert!((cues[0].end - 2.2).abs() < 1e-9);
    server.await.unwrap();
}

#[tokio::test]
#[ignore = "Offline FFmpeg test; set SCREENRECORD_FFMPEG. Synthetic PCM and localhost only."]
async fn media_contract_cut_audio_never_reaches_upload_or_resampler_context() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    let samples: Vec<i16> = (0..57_600)
        .map(|index| match index {
            9_600..21_600 => 500,
            38_400..52_800 => 1500,
            _ => 30_000,
        })
        .collect();
    pcm_wav(&source, 48_000, &samples);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/listen", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut payloads = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(socket.read_u8().await.unwrap());
                assert!(headers.len() < 16384);
            }
            let headers = String::from_utf8(headers).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            assert!(length < 20_000);
            let mut audio = vec![0; length];
            socket.read_exact(&mut audio).await.unwrap();
            payloads.push(audio);
            let body = response(serde_json::json!([{ "word":"kept", "start":0.01,"end":0.09 }]));
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        payloads
    });
    let input = TranscriptionInput {
        audio_path: source,
        ffmpeg_path: std::env::var_os("SCREENRECORD_FFMPEG")
            .expect("Set SCREENRECORD_FFMPEG")
            .into(),
        start: 0.1,
        end: 1.2,
        ranges: vec![(0.2, 0.45), (0.8, 1.1)],
        language: "id".into(),
        api_key: KEY.into(),
    };
    let result = transcribe_at(input, Arc::new(AtomicBool::new(false)), |_| {}, &endpoint)
        .await
        .unwrap();
    let payloads = server.await.unwrap();
    assert_eq!(result.len(), 2);
    assert!((result[0].start - 0.21).abs() < 1e-9 && (result[0].end - 0.29).abs() < 1e-9);
    assert!((result[1].start - 0.81).abs() < 1e-9 && (result[1].end - 0.89).abs() < 1e-9);
    for (payload, expected_samples, value) in [
        (&payloads[0], 4_000, 500i16),
        (&payloads[1], 4_800, 1500i16),
    ] {
        assert_eq!(
            extract::pcm_duration(payload, payload.len() as u64).unwrap(),
            expected_samples as f64 / 16_000.0
        );
        let mut position = 12;
        loop {
            let length = u32::from_le_bytes(payload[position + 4..position + 8].try_into().unwrap())
                as usize;
            if &payload[position..position + 4] == b"data" {
                assert_eq!(length, expected_samples * 2);
                for (index, sample) in payload[position + 8..position + 8 + length]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .enumerate()
                {
                    assert_eq!(
                        i16::from_le_bytes(*sample),
                        value,
                        "Upload sample {index} contained a removed sample or resampler contamination."
                    );
                }
                break;
            }
            position += 8 + length + length % 2;
        }
    }
}

#[tokio::test]
#[ignore = "Offline FFmpeg test; set SCREENRECORD_FFMPEG. Synthetic fractional PCM cuts and localhost only."]
async fn media_contract_fractional_cuts_exclude_native_samples_before_resampling() {
    let directory = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/listen", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut payloads = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(socket.read_u8().await.unwrap());
                assert!(headers.len() < 16_384);
            }
            let length = String::from_utf8(headers)
                .unwrap()
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            assert!(length < 20_000);
            let mut audio = vec![0; length];
            socket.read_exact(&mut audio).await.unwrap();
            payloads.push(audio);
            let body = response(serde_json::json!([{ "word":"kept", "start":0.01,"end":0.09 }]));
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        payloads
    });
    let (start, end) = (1.201234, 1.452345);
    for rate in [44_100, 48_000] {
        let source = directory.path().join(format!("source-{rate}.wav"));
        let samples: Vec<i16> = (0..rate * 2)
            .map(|index| {
                let time = f64::from(index) / f64::from(rate);
                if time >= start && time < end {
                    500
                } else {
                    30_000
                }
            })
            .collect();
        pcm_wav(&source, rate, &samples);
        let input = TranscriptionInput {
            audio_path: source,
            ffmpeg_path: std::env::var_os("SCREENRECORD_FFMPEG")
                .expect("Set SCREENRECORD_FFMPEG")
                .into(),
            start: 0.0,
            end: 2.0,
            ranges: vec![(start, end)],
            language: "id".into(),
            api_key: KEY.into(),
        };
        let result = transcribe_at(input, Arc::new(AtomicBool::new(false)), |_| {}, &endpoint)
            .await
            .unwrap();
        let first_source_sample = (start * f64::from(rate)).ceil() / f64::from(rate);
        assert_eq!(result.len(), 1);
        assert!((result[0].start - first_source_sample - 0.01).abs() < 1e-9);
    }
    for payload in server.await.unwrap() {
        let mut at = 12;
        loop {
            let length = u32::from_le_bytes(payload[at + 4..at + 8].try_into().unwrap()) as usize;
            if &payload[at..at + 4] == b"data" {
                assert!((4_017..=4_018).contains(&(length / 2)));
                for sample in payload[at + 8..at + 8 + length].as_chunks::<2>().0 {
                    assert_eq!(
                        i16::from_le_bytes(*sample),
                        500,
                        "A sample outside the retained interval entered the upload."
                    );
                }
                break;
            }
            at += 8 + length + length % 2;
        }
    }
}
