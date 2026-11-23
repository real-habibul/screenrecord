use crate::{
    edited_duration, kept_intervals, EditSettings, SubtitleCue, SubtitleRange, TIME_EPSILON,
};

fn timestamp(value: &str) -> Result<f64, String> {
    let invalid = || "SRT timestamps must use HH:MM:SS,mmm.".to_string();
    let parts: Vec<_> = value.trim().split(':').collect();
    if parts.len() != 3 {
        return Err(invalid());
    }
    let (seconds, fraction) = parts[2]
        .split_once(',')
        .or_else(|| parts[2].split_once('.'))
        .ok_or_else(invalid)?;
    if parts[0].len() < 2
        || parts[1].len() != 2
        || seconds.len() != 2
        || fraction.len() != 3
        || [parts[0], parts[1], seconds, fraction]
            .iter()
            .any(|part| !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(invalid());
    }
    let hours: u32 = parts[0].parse().map_err(|_| invalid())?;
    let minutes: u32 = parts[1].parse().map_err(|_| invalid())?;
    let seconds: u32 = seconds.parse().map_err(|_| invalid())?;
    let millis: u32 = fraction.parse().map_err(|_| invalid())?;
    if hours > 24 || minutes >= 60 || seconds >= 60 {
        return Err(invalid());
    }
    Ok(f64::from(hours * 3600 + minutes * 60 + seconds) + f64::from(millis) / 1000.0)
}

fn plain_text(value: &str) -> String {
    let mut output = String::new();
    let mut remaining = value;
    while let Some(index) = remaining.find('<') {
        output.push_str(&remaining[..index]);
        remaining = &remaining[index..];
        let Some(end) = remaining.find('>') else {
            break;
        };
        let tag = remaining[1..end].trim().to_ascii_lowercase();
        if ["b", "/b", "i", "/i", "u", "/u", "font", "/font"].contains(&tag.as_str())
            || tag.starts_with("font ")
        {
        } else if ["br", "br/", "br /"].contains(&tag.as_str()) {
            output.push('\n');
        } else {
            output.push_str(&remaining[..=end]);
        }
        remaining = &remaining[end + 1..];
    }
    output.push_str(remaining);
    output
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .trim()
        .to_owned()
}

pub fn subtitles_from_srt(
    edits: &EditSettings,
    duration: f64,
    text: &str,
) -> Result<Vec<SubtitleCue>, String> {
    if text.len() > 2 * 1024 * 1024 {
        return Err("SRT files must not exceed 2 MiB.".into());
    }
    let normalized = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for line in normalized.lines().chain(std::iter::once("")) {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
        if blocks.len() > 10_000 {
            return Err("SRT import supports at most 10000 cues.".into());
        }
    }
    let kept = kept_intervals(edits, duration);
    let output_duration = edited_duration(edits, duration);
    let mut cues = Vec::new();
    for (index, block) in blocks.into_iter().enumerate() {
        let timing = usize::from(
            block
                .first()
                .is_some_and(|line| line.trim().bytes().all(|byte| byte.is_ascii_digit())),
        );
        let header = block
            .get(timing)
            .ok_or_else(|| format!("SRT cue {} is missing its timestamp.", index + 1))?;
        let (from, to) = header
            .split_once("-->")
            .ok_or_else(|| format!("SRT cue {} is missing its timestamp separator.", index + 1))?;
        let start = timestamp(from)?;
        let end = timestamp(to)?;
        if end - start + TIME_EPSILON < 0.01
            || end > output_duration + TIME_EPSILON
            || start >= output_duration
        {
            return Err(format!(
                "SRT cue {} must last at least 0.01 seconds within the edited video.",
                index + 1
            ));
        }
        let text = plain_text(&block[timing + 1..].join("\n"));
        let ranges: Vec<_> = kept
            .iter()
            .filter_map(|range| {
                let range_end = range.output_start + range.end - range.start;
                let from = start.max(range.output_start);
                let to = end.min(range_end);
                (to > from).then_some(SubtitleRange {
                    start: (range.start + from - range.output_start).max(range.start),
                    end: (range.start + to - range.output_start).min(range.end),
                })
            })
            .filter(|range| range.end > range.start)
            .collect();
        let first = ranges
            .first()
            .ok_or("SRT timing does not intersect the edited video.")?
            .start;
        let last = ranges.last().unwrap().end;
        cues.push(SubtitleCue {
            id: uuid::Uuid::new_v4().to_string(),
            start: first,
            end: last,
            text,
            words: vec![],
            source_ranges: ranges,
        });
    }
    if cues.is_empty() {
        return Err("The selected SRT file has no subtitle cues.".into());
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut check = edits.clone();
    check.subtitles = cues.clone();
    check.validate(duration)?;
    Ok(cues)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VideoClip;
    #[test]
    fn srt_import_maps_output_through_cuts_and_preserves_one_logical_cue() {
        let mut edits = EditSettings::new(6.0);
        edits.video_clips = vec![
            VideoClip {
                id: "a".into(),
                start: 0.0,
                end: 2.0,
            },
            VideoClip {
                id: "b".into(),
                start: 4.0,
                end: 6.0,
            },
        ];
        edits.subtitles = subtitles_from_srt(
            &edits,
            6.0,
            "\u{feff}1\r\n00:00:01,000 --> 00:00:03,000\r\n<i>Hello</i> &amp; 世界\r\n",
        )
        .unwrap();
        assert_eq!(edits.subtitles.len(), 1);
        assert_eq!(edits.subtitles[0].source_ranges.len(), 2);
        assert_eq!(edits.subtitles[0].text, "Hello & 世界");
        let exported = crate::subtitles_srt(&edits, 6.0).unwrap();
        assert!(exported.contains("00:00:01,000 --> 00:00:03,000\nHello & 世界"));
        edits.video_clips[0].start = 1.5;
        assert!(!crate::subtitles_srt(&edits, 6.0).unwrap().contains("Hello"));
    }
    #[test]
    fn malformed_overlapping_empty_and_outside_srt_are_rejected_without_changing_edits() {
        let edits = EditSettings::new(5.0);
        let before = serde_json::to_string(&edits).unwrap();
        for text in [
            "",
            "1\n00:99:00,000 --> 00:00:02,000\nText",
            "1\n00:00:01,000 --> 00:00:01,000\nText",
            "1\n00:00:04,000 --> 00:00:06,000\nText",
            "1\n00:00:01,000 --> 00:00:03,000\nA\n\n2\n00:00:02,000 --> 00:00:04,000\nB",
        ] {
            assert!(subtitles_from_srt(&edits, 5.0, text).is_err());
        }
        assert_eq!(serde_json::to_string(&edits).unwrap(), before);
        assert_eq!(
            plain_text("2 < 3 &lt;i&gt;literal&lt;/i&gt;"),
            "2 < 3 <i>literal</i>"
        );
    }
}
