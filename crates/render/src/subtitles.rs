use crate::{effects::frame_at, math::Layout, process};
use screenrecord_core::{Project, SubtitleCue, SubtitleStyle};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

fn color(value: &str) -> String {
    format!("&H00{}{}{}", &value[5..7], &value[3..5], &value[1..3])
}

fn header(style: &SubtitleStyle, layout: &Layout) -> String {
    let margin = (layout.width as f64 * 0.05).round() as u32;
    let vertical = (48.0 * layout.reference_scale).round() as u32;
    let alignment = if style.position == "top" { 8 } else { 2 };
    format!("[Script Info]\nScriptType: v4.00+\nPlayResX: {}\nPlayResY: {}\nScaledBorderAndShadow: yes\nYCbCr Matrix: None\nWrapStyle: 0\nKerning: yes\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Segoe UI,{:.6},{},{},{},{},0,0,0,0,100,100,0,0,4,0,{:.6},{alignment},{margin},{margin},{vertical},1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        layout.width, layout.height, style.font_size * layout.reference_scale,
        color(&style.color), color(&style.color), color(&style.background), color(&style.background), 8.0 * layout.reference_scale)
}

fn literal(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        match character {
            '{' => escaped.push_str("\\{"),
            '}' => escaped.push_str("\\}"),
            // A word joiner prevents literal backslashes from becoming ASS line breaks or tags.
            '\\' => escaped.push_str("\\\u{2060}"),
            '\n' => escaped.push_str("\\N"),
            '\r' => {}
            '\t' => escaped.push(' '),
            c if c.is_control() => {}
            c => escaped.push(c),
        }
    }
    escaped
}

fn timestamp(centiseconds: u64) -> String {
    format!(
        "{}:{:02}:{:02}.{:02}",
        centiseconds / 360_000,
        centiseconds / 6_000 % 60,
        centiseconds / 100 % 60,
        centiseconds % 100
    )
}

fn styled_text(cue: &SubtitleCue, style: &SubtitleStyle, word_index: Option<usize>) -> String {
    let Some(active) = word_index.filter(|_| style.highlight_words) else {
        return literal(&cue.text);
    };
    cue.words
        .iter()
        .enumerate()
        .map(|(index, word)| {
            if index == active {
                format!(
                    "{{\\1c{}&}}{}{{\\1c{}&}}",
                    color(&style.highlight_color),
                    literal(word.text.trim()),
                    color(&style.color)
                )
            } else {
                literal(word.text.trim())
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn event(text: &str, start: u64, end: u64) -> String {
    format!(
        "Dialogue: 0,{},{},Default,,0,0,0,,{}\n",
        timestamp(start),
        timestamp(end),
        text
    )
}

pub(crate) fn write_chunk(
    project: &Project,
    layout: &Layout,
    directory: &Path,
    source_start: f64,
    frames: u64,
    fps: u32,
) -> Result<bool, String> {
    if !project.edits.subtitle_style.enabled {
        return Ok(false);
    }
    let mut contents = header(&project.edits.subtitle_style, layout);
    let mut any = false;
    for raw_cue in &project.edits.subtitles {
        if raw_cue.end <= source_start || raw_cue.start >= source_start + frames as f64 / fps as f64
        {
            continue;
        }
        let Some(cue) =
            screenrecord_core::projected_subtitle(&project.edits, project.duration, raw_cue)
        else {
            continue;
        };
        let words: Vec<_> = cue
            .words
            .iter()
            .map(|word| {
                (
                    frame_at(word.start, source_start, fps),
                    frame_at(word.end, source_start, fps),
                )
            })
            .collect();
        for (range_start, range_end) in screenrecord_core::subtitle_ranges(&cue) {
            let start = frame_at(range_start, source_start, fps).min(frames);
            let end = frame_at(range_end, source_start, fps).min(frames);
            if start >= end {
                continue;
            }
            let mut boundaries = vec![start, end];
            if project.edits.subtitle_style.highlight_words {
                boundaries.extend(
                    words
                        .iter()
                        .flat_map(|(a, b)| [*a, *b])
                        .filter(|frame| *frame > start && *frame < end),
                );
            }
            boundaries.sort_unstable();
            boundaries.dedup();
            for interval in boundaries.windows(2) {
                let active = words
                    .iter()
                    .position(|(start, end)| *start <= interval[0] && interval[0] < *end);
                // Centisecond floors stay between adjacent frames at the supported 30/60 fps rates.
                contents.push_str(&event(
                    &styled_text(&cue, &project.edits.subtitle_style, active),
                    interval[0] * 100 / fps as u64,
                    interval[1] * 100 / fps as u64,
                ));
                any = true;
            }
        }
    }
    if any {
        fs::write(directory.join("subtitles.ass"), contents).map_err(|e| e.to_string())?;
    }
    Ok(any)
}

pub fn subtitle_preview(
    project: &Project,
    cue_id: &str,
    word_index: Option<usize>,
    ffmpeg: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let mut previews = subtitle_previews(project, cue_id, &[word_index], ffmpeg, cancel)?;
    Ok(previews.remove(0))
}

pub fn subtitle_previews(
    project: &Project,
    cue_id: &str,
    word_indices: &[Option<usize>],
    ffmpeg: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<Vec<u8>>, String> {
    if word_indices.is_empty() || word_indices.len() > 8 {
        return Err("Subtitle preview batches require 1 to 8 appearances.".into());
    }
    project.validate()?;
    let cue: &SubtitleCue = project
        .edits
        .subtitles
        .iter()
        .find(|cue| cue.id == cue_id)
        .ok_or("Subtitle cue does not exist.")?;
    if word_indices
        .iter()
        .flatten()
        .any(|index| *index >= cue.words.len())
    {
        return Err("Subtitle word index is outside the cue.".into());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Subtitle preview cancelled.".into());
    }
    let layout = crate::math::layout(project, "1080p")?;
    let directory = tempfile::Builder::new()
        .prefix("screenrecord-subtitle-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let mut contents = header(&project.edits.subtitle_style, &layout);
    if project.edits.subtitle_style.enabled {
        for (frame, index) in word_indices.iter().enumerate() {
            contents.push_str(&event(
                &styled_text(cue, &project.edits.subtitle_style, *index),
                frame as u64 * 100,
                (frame as u64 + 1) * 100,
            ));
        }
    }
    fs::write(directory.path().join("subtitles.ass"), contents).map_err(|e| e.to_string())?;
    let mut command = Command::new(ffmpeg);
    command.current_dir(directory.path()).args([
        "-hide_banner",
        "-v",
        "error",
        "-nostdin",
        "-n",
        "-filter_threads",
        "1",
        "-f",
        "lavfi",
        "-i",
        &format!(
            "color=c=black@0.0:s={}x{}:r=1:d={},format=rgba",
            layout.width,
            layout.height,
            word_indices.len()
        ),
        "-vf",
        "ass=f=subtitles.ass:alpha=1:shaping=complex,format=rgba",
        "-frames:v",
        &word_indices.len().to_string(),
        "-fps_mode",
        "passthrough",
        "-threads:v",
        "1",
        "-start_number",
        "0",
        "preview-%02d.png",
    ]);
    let (success, _, error) = process::capture(&mut command, Duration::from_secs(30), cancel)?;
    if !success {
        return Err(format!("Cannot render subtitle preview: {}", error.trim()));
    }
    let mut previews = Vec::with_capacity(word_indices.len());
    let mut bytes = 0;
    for frame in 0..word_indices.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Subtitle preview cancelled.".into());
        }
        let output = directory.path().join(format!("preview-{frame:02}.png"));
        let length = fs::metadata(&output).map_err(|e| e.to_string())?.len();
        if length > 16 * 1024 * 1024 {
            return Err("Subtitle preview exceeds the 16 MiB limit.".into());
        }
        bytes += length;
        if bytes > 64 * 1024 * 1024 {
            return Err("Subtitle preview batch exceeds the 64 MiB limit.".into());
        }
        previews.push(fs::read(output).map_err(|e| e.to_string())?);
    }
    Ok(previews)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subtitle_text_cannot_inject_ass_tags_or_events() {
        let escaped = literal("{\\pos(0,0)} C:\\new\\N\nDialogue: 0,0:00:00.00,0:00:10.00\n文字");
        assert!(escaped.starts_with("\\{\\\u{2060}pos(0,0)\\}"));
        assert!(escaped.contains("C:\\\u{2060}new\\\u{2060}N"));
        assert!(!escaped.contains('\n'));
        assert!(escaped.ends_with("\\N文字"));
    }
}
