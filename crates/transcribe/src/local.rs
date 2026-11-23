pub use crate::local_setup::{install, status, LocalStatus};
use crate::{
    cancelled, check_cancel, chunks,
    cues::{self, Word},
    extract,
    local_assets::MODEL,
    local_setup, retained_ranges, source_words, validate_audio, TranscriptionInput, CANCELLED,
    MAX_CUES, MAX_TEXT_BYTES,
};
use screenrecord_core::SubtitleCue;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, BufReader},
    process::Command,
};
use zeroize::Zeroize;

const MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;

pub async fn transcribe(
    mut input: TranscriptionInput,
    directory: PathBuf,
    cancel: Arc<AtomicBool>,
    mut progress: impl Fn(f64) + Send + 'static,
) -> Result<Vec<SubtitleCue>, String> {
    input.api_key.zeroize();
    check_cancel(&cancel)?;
    validate_audio(&input)?;
    let token = Arc::clone(&cancel);
    let runtime = tokio::task::spawn_blocking(move || local_setup::verified(&directory, &token))
        .await
        .map_err(|_| "Local subtitle verification stopped unexpectedly.".to_owned())??;
    let temporary = tempfile::Builder::new()
        .prefix("screenrecord-local-subtitles-")
        .tempdir()
        .map_err(|_| "Cannot prepare temporary local subtitle audio.".to_owned())?;
    let ranges = retained_ranges(&input)?;
    let count = ranges
        .iter()
        .map(|&(start, end)| chunks(start, end).len())
        .sum::<usize>();
    let rate = extract::sample_rate(&input, temporary.path(), &cancel).await?;
    let mut complete = 0;
    let mut word_count = 0;
    let mut output = Vec::new();
    progress(0.0);
    for (start, end) in ranges {
        let mut words = Vec::new();
        for chunk in chunks(start, end) {
            check_cancel(&cancel)?;
            let audio = temporary.path().join("speech.wav");
            let duration = extract::extract(&input, &chunk, rate, &audio, &cancel).await?;
            let json = temporary.path().join("words.json");
            let progress_ref = &mut progress;
            let local_words = infer(
                &runtime.directory,
                &audio,
                &json,
                &input.language,
                duration,
                &cancel,
                move |fraction| {
                    progress_ref((complete as f64 + 0.1 + 0.85 * fraction) / count as f64)
                },
            )
            .await?;
            let mut timestamp_chunk = chunk;
            timestamp_chunk.start =
                extract::sample_boundary(chunk.start, rate) as f64 / f64::from(rate);
            let previous = words.len();
            words.extend(source_words(local_words, &timestamp_chunk, start, end));
            word_count += words.len() - previous;
            if word_count > 200_000 {
                return Err("The local transcription exceeds the supported word count.".into());
            }
            for path in [audio, json] {
                tokio::fs::remove_file(path)
                    .await
                    .map_err(|_| "Cannot remove temporary local subtitle files.".to_owned())?;
            }
            complete += 1;
            progress(complete as f64 / count as f64);
        }
        output.extend(cues::build(words, start, end)?);
        if output.len() > MAX_CUES
            || output.iter().map(|cue| cue.text.len()).sum::<usize>() > MAX_TEXT_BYTES
        {
            return Err(
                "Subtitles exceed 10,000 cues or 1 MiB of text. Select a shorter range.".into(),
            );
        }
    }
    check_cancel(&cancel)?;
    temporary
        .close()
        .map_err(|_| "Local subtitles finished, but temporary audio cleanup failed.".to_owned())?;
    progress(1.0);
    Ok(output)
}

async fn infer(
    runtime: &Path,
    audio: &Path,
    json: &Path,
    language: &str,
    duration: f64,
    cancel: &AtomicBool,
    progress: impl FnMut(f64),
) -> Result<Vec<Word>, String> {
    let mut command = Command::new(runtime.join("whisper-cli.exe"));
    let system =
        std::env::var_os("SystemRoot").ok_or("Windows system directory is unavailable.")?;
    let threads = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .clamp(1, 8);
    command
        .creation_flags(0x0800_0000)
        .kill_on_drop(true)
        .current_dir(runtime)
        .env_clear()
        .env("SystemRoot", &system)
        .env("WINDIR", &system)
        .env("PATH", Path::new(&system).join("System32"))
        .args([
            "--no-gpu",
            "--no-flash-attn",
            "--threads",
            &threads.to_string(),
            "--language",
            language,
            "--model",
        ])
        .arg(runtime.join(MODEL.name))
        .arg("--file")
        .arg(audio)
        .args([
            "--output-json",
            "--max-len",
            "1",
            "--split-on-word",
            "--print-progress",
            "--output-file",
        ])
        .arg(json.with_extension(""))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    run_cli(&mut command, json, duration, cancel, progress).await?;
    let file = tokio::fs::File::open(json)
        .await
        .map_err(|_| "Local transcription returned no word timestamps.".to_owned())?;
    let mut bytes = Vec::new();
    file.take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Cannot read local subtitle timestamps.".to_owned())?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err("Local subtitle output exceeds the supported size.".into());
    }
    parse_words(&bytes, duration)
}

async fn run_cli(
    command: &mut Command,
    json: &Path,
    duration: f64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f64),
) -> Result<(), String> {
    check_cancel(cancel)?;
    let mut child = command.spawn().map_err(|_| "Cannot start the verified local subtitle runtime. Install the Microsoft Visual C++ x64 runtime if Windows reports a missing dependency.".to_owned())?;
    let job = match extract::Job::attach(&child) {
        Ok(job) => Some(job),
        Err(error) => {
            if let Ok(Some(status)) = child.try_wait() {
                if !status.success() {
                    return Err(
                        "Local subtitle runtime could not initialize on this computer.".into(),
                    );
                }
                None
            } else {
                let _ = child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
                return Err(error);
            }
        }
    };
    let stderr = child
        .stderr
        .take()
        .ok_or("Local subtitle progress pipe is unavailable.")?;
    let mut reader = BufReader::new(stderr);
    let mut buffer = [0; 4096];
    let mut line = Vec::new();
    let deadline = tokio::time::sleep(Duration::from_secs_f64(
        (duration * 12.0 + 120.0).clamp(120.0, 7500.0),
    ));
    tokio::pin!(deadline);
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    let mut ended = false;
    let result = loop {
        tokio::select! {
            biased;
            _ = cancelled(cancel) => break Err(CANCELLED.to_owned()),
            _ = &mut deadline => break Err("Local transcription exceeded its time limit. Try a shorter range.".into()),
            _ = poll.tick() => {
                if tokio::fs::metadata(json).await.is_ok_and(|metadata| metadata.len() > MAX_JSON_BYTES) { break Err("Local subtitle output exceeds the supported size.".into()); }
                match child.try_wait() { Ok(Some(status)) => break if status.success() { Ok(()) } else { Err("Local subtitle runtime failed. Verify setup and try a shorter recording.".into()) }, Err(_) => break Err("Cannot inspect the local subtitle process.".into()), Ok(None) => {} }
            }
            read = reader.read(&mut buffer), if !ended => match read {
                Ok(0) => ended = true,
                Ok(count) => for &byte in &buffer[..count] { if byte == b'\n' || byte == b'\r' { if let Some(value) = progress_value(&line) { progress(value); } line.clear(); } else if line.len() < 4096 { line.push(byte); } },
                Err(_) => break Err("Local subtitle progress pipe failed.".into()),
            }
        }
    };
    if result.is_err() {
        if let Some(job) = &job {
            job.terminate();
        }
        let _ = child.start_kill();
        if !matches!(
            tokio::time::timeout(Duration::from_secs(2), child.wait()).await,
            Ok(Ok(_))
        ) {
            return Err(
                "Local subtitle process is still closing. Restart the app before trying again."
                    .into(),
            );
        }
    }
    result
}

fn progress_value(bytes: &[u8]) -> Option<f64> {
    let line = std::str::from_utf8(bytes).ok()?;
    if !line.contains("progress") {
        return None;
    }
    let number = line
        .split('=')
        .nth(1)?
        .trim()
        .strip_suffix('%')?
        .trim()
        .parse::<u32>()
        .ok()?;
    (number <= 100).then_some(number as f64 / 100.0)
}

#[derive(Deserialize)]
struct Transcript {
    transcription: Vec<Segment>,
}
#[derive(Deserialize)]
struct Segment {
    text: String,
    offsets: Offsets,
}
#[derive(Deserialize)]
struct Offsets {
    from: f64,
    to: f64,
}

fn parse_words(bytes: &[u8], duration: f64) -> Result<Vec<Word>, String> {
    let result: Transcript = serde_json::from_slice(bytes)
        .map_err(|_| "Local transcription returned invalid word timing data.".to_owned())?;
    if result.transcription.len() > 20_000 {
        return Err("Local transcription returned too many words for one chunk.".into());
    }
    let mut words = Vec::new();
    let mut previous = 0.0;
    let mut leading: Option<(f64, String)> = None;
    for segment in result.transcription {
        let text = segment.text.trim();
        if text.is_empty() {
            continue;
        }
        let start = segment.offsets.from / 1000.0;
        let end = segment.offsets.to / 1000.0;
        if !start.is_finite()
            || !end.is_finite()
            || start < previous
            || start < 0.0
            || start > duration
            || (start == duration && end > start)
            || end < start
            || end > duration + 0.1
            || text.split_whitespace().count() != 1
            || text.chars().count() > 500
            || text.chars().any(char::is_control)
        {
            return Err(
                "Local transcription returned unsupported word timing. Subtitles were not changed."
                    .into(),
            );
        }
        previous = start;
        if end == start {
            if let Some(word) = words
                .last_mut()
                .filter(|word: &&mut Word| (word.end - start).abs() <= 1e-9)
            {
                word.text.push(' ');
                word.text.push_str(text);
            } else if words.is_empty() {
                if let Some((at, pending)) = &mut leading {
                    if (*at - start).abs() > 1e-9 {
                        return Err(
                            "Local transcription could not resolve a word timestamp.".into()
                        );
                    }
                    pending.push(' ');
                    pending.push_str(text);
                } else {
                    leading = Some((start, text.into()));
                }
            } else {
                return Err("Local transcription could not resolve a word timestamp.".into());
            }
            continue;
        }
        let text = if let Some((at, pending)) = leading.take() {
            if (at - start).abs() > 1e-9 {
                return Err("Local transcription could not resolve a word timestamp.".into());
            }
            format!("{pending} {text}")
        } else {
            text.into()
        };
        words.push(Word {
            start,
            end: end.min(duration),
            text,
        });
    }
    if leading.is_some() {
        return Err("Local transcription returned text without positive word timing.".into());
    }
    Ok(words)
}

#[cfg(test)]
mod tests;
