mod cues;
mod extract;
pub mod local;
mod local_assets;
mod local_setup;
mod provider;

use screenrecord_core::SubtitleCue;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use zeroize::Zeroize;

pub struct TranscriptionInput {
    pub audio_path: PathBuf,
    pub ffmpeg_path: PathBuf,
    pub start: f64,
    pub end: f64,
    pub ranges: Vec<(f64, f64)>,
    pub language: String,
    pub api_key: String,
}

impl Drop for TranscriptionInput {
    fn drop(&mut self) {
        self.api_key.zeroize();
    }
}

const ENDPOINT: &str = "https://api.deepgram.com/v1/listen";
const MAX_CUES: usize = 10_000;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub(crate) const CANCELLED: &str = "Subtitle generation cancelled.";

/// Call only after an explicit Generate action. Temporary audio is removed when the job exits.
pub async fn transcribe(
    input: TranscriptionInput,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(f64) + Send + 'static,
) -> Result<Vec<SubtitleCue>, String> {
    transcribe_at(input, cancel, progress, ENDPOINT).await
}

async fn transcribe_at(
    mut input: TranscriptionInput,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(f64) + Send + 'static,
    endpoint: &str,
) -> Result<Vec<SubtitleCue>, String> {
    check_cancel(&cancel)?;
    validate(&input)?;
    let client = provider::Provider::new(&input.api_key, endpoint, &input.language)?;
    input.api_key.zeroize();
    let directory = tempfile::Builder::new()
        .prefix("screenrecord-transcribe-")
        .tempdir()
        .map_err(|_| "Cannot create temporary subtitle audio.".to_owned())?;
    let ranges = retained_ranges(&input)?;
    let total_chunks = ranges
        .iter()
        .map(|&(start, end)| chunks(start, end).len())
        .sum::<usize>();
    let mut completed = 0;
    let mut word_count = 0;
    let mut result = Vec::new();
    progress(0.0);
    let sample_rate = extract::sample_rate(&input, directory.path(), &cancel).await?;
    for (start, end) in ranges {
        let mut words = Vec::new();
        for chunk in chunks(start, end) {
            check_cancel(&cancel)?;
            let audio = directory.path().join("speech.wav");
            let extracted_duration =
                extract::extract(&input, &chunk, sample_rate, &audio, &cancel).await?;
            progress((completed as f64 + 0.2) / total_chunks as f64);
            let received = client.words(&audio, extracted_duration, &cancel).await?;
            let previous = words.len();
            let mut timestamp_chunk = chunk;
            timestamp_chunk.start =
                extract::sample_boundary(chunk.start, sample_rate) as f64 / f64::from(sample_rate);
            words.extend(source_words(received, &timestamp_chunk, start, end));
            word_count += words.len() - previous;
            if word_count > 200_000 {
                return Err("The transcription exceeds the supported word count.".into());
            }
            tokio::fs::remove_file(&audio)
                .await
                .map_err(|_| "Cannot remove temporary subtitle audio.".to_owned())?;
            progress((completed as f64 + 0.9) / total_chunks as f64);
            completed += 1;
        }
        check_cancel(&cancel)?;
        result.extend(cues::build(words, start, end)?);
        if result.len() > MAX_CUES
            || result.iter().map(|cue| cue.text.len()).sum::<usize>() > MAX_TEXT_BYTES
        {
            return Err(
                "Subtitles exceed the supported 10,000 cues or 1 MiB of text. Select a shorter range."
                    .into(),
            );
        }
    }
    directory.close().map_err(|_| {
        "Subtitle generation finished, but temporary audio cleanup failed.".to_owned()
    })?;
    progress(1.0);
    Ok(result)
}

fn source_words(
    words: Vec<cues::Word>,
    chunk: &Chunk,
    start: f64,
    end: f64,
) -> impl Iterator<Item = cues::Word> + '_ {
    words.into_iter().filter_map(move |mut word| {
        word.start += chunk.start;
        word.end += chunk.start;
        let middle = (word.start + word.end) / 2.0;
        if middle < chunk.owned_start || middle >= chunk.owned_end {
            return None;
        }
        word.start = word.start.max(start);
        word.end = word.end.min(end);
        (word.end > word.start).then_some(word)
    })
}

fn validate(input: &TranscriptionInput) -> Result<(), String> {
    validate_audio(input)?;
    if !(16..=512).contains(&input.api_key.len())
        || !input.api_key.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err("Enter a valid Deepgram API key.".into());
    }
    Ok(())
}

fn validate_audio(input: &TranscriptionInput) -> Result<(), String> {
    if !input.start.is_finite()
        || !input.end.is_finite()
        || input.start < 0.0
        || input.end - input.start + screenrecord_core::TIME_EPSILON < 0.1
        || input.end > 86_400.0
    {
        return Err("Choose a subtitle range between 0.1 seconds and 24 hours.".into());
    }
    retained_ranges(input)?;
    if !input.audio_path.is_file() || !input.audio_path.is_absolute() {
        return Err("The selected recording audio is unavailable.".into());
    }
    if !input.ffmpeg_path.is_file() || !input.ffmpeg_path.is_absolute() {
        return Err("The local audio processor is unavailable.".into());
    }
    if !["auto", "id", "en"].contains(&input.language.as_str()) {
        return Err("Choose Auto, Indonesian, or English for subtitle language.".into());
    }
    Ok(())
}

fn retained_ranges(input: &TranscriptionInput) -> Result<Vec<(f64, f64)>, String> {
    if input.ranges.is_empty() {
        return Ok(vec![(input.start, input.end)]);
    }
    if input.ranges.len() > 512 {
        return Err("Subtitle generation supports up to 512 retained video clips.".into());
    }
    let mut previous_end = input.start;
    let mut ranges: Vec<(f64, f64)> = Vec::new();
    for &(start, end) in &input.ranges {
        if !start.is_finite()
            || !end.is_finite()
            || start < input.start
            || end > input.end
            || start + screenrecord_core::TIME_EPSILON < previous_end
            || end <= start
        {
            return Err(
                "Subtitle ranges must be ordered, nonoverlapping, and inside the selected trim."
                    .into(),
            );
        }
        previous_end = previous_end.max(end);
        if let Some(previous) = ranges.last_mut().filter(|previous| start <= previous.1) {
            previous.1 = previous.1.max(end);
        } else {
            ranges.push((start, end));
        }
    }
    let duration = ranges.iter().map(|(start, end)| end - start).sum::<f64>();
    if duration + screenrecord_core::TIME_EPSILON < 0.1 {
        return Err("Keep at least 0.1 seconds of audio for subtitle generation.".into());
    }
    // A trim-edge fragment shorter than a cue cannot yield a source-confined subtitle.
    ranges.retain(|(start, end)| end - start + screenrecord_core::TIME_EPSILON >= 0.01);
    Ok(ranges)
}

#[derive(Debug, Clone, Copy)]
struct Chunk {
    start: f64,
    end: f64,
    owned_start: f64,
    owned_end: f64,
}

fn chunks(start: f64, end: f64) -> Vec<Chunk> {
    let mut result = Vec::new();
    let mut owned_start = start;
    while owned_start < end {
        let owned_end = (owned_start + 596.0).min(end);
        result.push(Chunk {
            start: (owned_start - 2.0).max(start),
            end: (owned_end + 2.0).min(end),
            owned_start,
            owned_end,
        });
        owned_start = owned_end;
    }
    result
}

pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

pub(crate) async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(test)]
mod tests;
