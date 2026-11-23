use crate::{kept_intervals, EditSettings, SubtitleCue, SubtitleRange, TIME_EPSILON};

pub fn subtitle_ranges(cue: &SubtitleCue) -> Vec<(f64, f64)> {
    if cue.source_ranges.is_empty() {
        vec![(cue.start, cue.end)]
    } else {
        cue.source_ranges
            .iter()
            .map(|range| (range.start, range.end))
            .collect()
    }
}

fn merge(ranges: impl IntoIterator<Item = (f64, f64)>) -> Vec<(f64, f64)> {
    let mut result: Vec<(f64, f64)> = Vec::new();
    for (start, end) in ranges {
        if end <= start {
            continue;
        }
        if let Some(previous) = result.last_mut() {
            if start <= previous.1 {
                previous.1 = previous.1.max(end);
                continue;
            }
        }
        result.push((start, end));
    }
    result
}

pub fn projected_subtitle(
    edits: &EditSettings,
    duration: f64,
    cue: &SubtitleCue,
) -> Option<SubtitleCue> {
    let declared = merge(subtitle_ranges(cue).into_iter().map(|(start, end)| {
        (
            start.max(edits.trim_start),
            end.min(edits.trim_end).min(duration),
        )
    }));
    let kept = kept_intervals(edits, duration);
    let coverage = merge(declared.iter().flat_map(|&(start, end)| {
        kept.iter()
            .map(move |range| (start.max(range.start), end.min(range.end)))
    }));
    if coverage.is_empty() {
        return None;
    }
    let mut result = cue.clone();
    if cue.words.is_empty() {
        let original: f64 = declared.iter().map(|(start, end)| end - start).sum();
        let remaining: f64 = coverage.iter().map(|(start, end)| end - start).sum();
        if !edits.video_clips.is_empty() && remaining + TIME_EPSILON < original {
            return None;
        }
    } else {
        result.words.retain(|word| {
            coverage
                .iter()
                .any(|&(start, end)| word.start >= start && word.end <= end)
        });
        if result.words.is_empty() {
            return None;
        }
        result.text = result
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
    }
    let first = result
        .words
        .first()
        .map_or(coverage[0].0, |word| word.start);
    let last = result
        .words
        .last()
        .map_or(coverage.last().unwrap().1, |word| word.end);
    result.start = first.min(last - 0.01).max(cue.start);
    result.end = last.max(result.start + 0.01).min(cue.end);
    result.source_ranges = coverage
        .into_iter()
        .filter_map(|(start, end)| {
            let start = start.max(result.start);
            let end = end.min(result.end);
            (end > start).then_some(SubtitleRange { start, end })
        })
        .collect();
    Some(result)
}

pub(crate) fn validate_subtitle_metadata(cue: &SubtitleCue) -> Result<(), String> {
    if cue.source_ranges.len() > 512 || cue.words.len() > 500 {
        return Err("A subtitle supports up to 512 source ranges and 500 timed words.".into());
    }
    let mut previous = cue.start;
    for range in &cue.source_ranges {
        if !range.start.is_finite()
            || !range.end.is_finite()
            || range.start < cue.start
            || range.end > cue.end + TIME_EPSILON
            || range.end <= range.start
            || range.start + TIME_EPSILON < previous
        {
            return Err(
                "Subtitle source ranges must be ordered, nonoverlapping and inside the cue.".into(),
            );
        }
        previous = range.end;
    }
    let ranges = merge(subtitle_ranges(cue));
    previous = cue.start;
    for word in &cue.words {
        if !word.start.is_finite()
            || !word.end.is_finite()
            || word.start < cue.start
            || word.end > cue.end + TIME_EPSILON
            || word.end <= word.start
            || word.start + TIME_EPSILON < previous
            || word.text.trim().is_empty()
            || word.text.trim() != word.text
            || word.text.chars().any(char::is_control)
            || !ranges
                .iter()
                .any(|&(start, end)| word.start >= start && word.end <= end)
        {
            return Err("Timed words must be ordered, nonoverlapping, nonempty and inside the subtitle coverage.".into());
        }
        previous = word.end;
    }
    if !cue.words.is_empty()
        && cue
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            != cue.text
    {
        return Err(
            "Timed word text must match the subtitle text. Clear word timings after editing text."
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SubtitleWord, VideoClip};
    #[test]
    fn timed_projection_omits_removed_words_without_mutating_raw_captions() {
        let mut edits = EditSettings::new(4.0);
        edits.video_clips = vec![
            VideoClip {
                id: "a".into(),
                start: 0.0,
                end: 1.0,
            },
            VideoClip {
                id: "b".into(),
                start: 2.0,
                end: 4.0,
            },
        ];
        let cue = SubtitleCue {
            id: "speech".into(),
            start: 0.0,
            end: 4.0,
            text: "Keep secret finish".into(),
            source_ranges: vec![],
            words: vec![
                SubtitleWord {
                    start: 0.2,
                    end: 0.5,
                    text: "Keep".into(),
                },
                SubtitleWord {
                    start: 0.9,
                    end: 2.1,
                    text: "secret".into(),
                },
                SubtitleWord {
                    start: 2.5,
                    end: 3.0,
                    text: "finish".into(),
                },
            ],
        };
        edits.subtitles.push(cue.clone());
        edits.validate(4.0).unwrap();
        let projected = projected_subtitle(&edits, 4.0, &cue).unwrap();
        assert_eq!(projected.text, "Keep finish");
        assert_eq!(cue.text, "Keep secret finish");
        assert_eq!(projected.words.len(), 2);
        let srt = crate::subtitles_srt(&edits, 4.0).unwrap();
        assert!(!srt.contains("secret"));
        assert!(srt.contains("Keep finish"));
        let mut imported = cue.clone();
        imported.words.clear();
        imported.text = "Imported edited caption".into();
        imported.source_ranges = vec![
            SubtitleRange {
                start: 0.0,
                end: 1.0,
            },
            SubtitleRange {
                start: 2.0,
                end: 4.0,
            },
        ];
        assert!(projected_subtitle(&edits, 4.0, &imported).is_some());
        imported.source_ranges.clear();
        assert!(projected_subtitle(&edits, 4.0, &imported).is_none());
        let mut bad = cue;
        bad.words[0].text = "Changed".into();
        assert!(validate_subtitle_metadata(&bad).is_err());
    }
}
