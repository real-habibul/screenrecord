use screenrecord_core::{SubtitleCue, SubtitleWord};

#[derive(Debug, Clone)]
pub(crate) struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub(crate) fn build(
    mut words: Vec<Word>,
    start: f64,
    end: f64,
) -> Result<Vec<SubtitleCue>, String> {
    words.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut cues = Vec::new();
    let mut current: Vec<Word> = Vec::new();
    for word in words {
        if word.start < start
            || word.end > end
            || !word.start.is_finite()
            || !word.end.is_finite()
            || word.end <= word.start
            || word.text.trim().is_empty()
            || word.text != word.text.trim()
            || word.text.chars().any(char::is_control)
        {
            return Err("Subtitle timing falls outside the selected range.".into());
        }
        let previous = current
            .last()
            .map(|item| (item.end, item.text.as_str()))
            .or_else(|| {
                cues.last()
                    .and_then(|cue: &SubtitleCue| cue.words.last())
                    .map(|item| (item.end, item.text.as_str()))
            });
        if let Some((previous_end, previous_text)) = previous {
            if word.start < previous_end {
                if same_word(&word.text, previous_text) {
                    continue;
                }
                return Err(
                    "The transcription returned overlapping words. Subtitles were not changed."
                        .into(),
                );
            }
        }
        let characters = current
            .iter()
            .map(|word| word.text.chars().count() + 1)
            .sum::<usize>()
            + word.text.chars().count();
        if !current.is_empty()
            && (word.end - current[0].start > 6.0
                || characters > 84
                || word.start - current.last().unwrap().end > 0.8)
        {
            push(&mut cues, &mut current);
        }
        current.push(word);
        if current
            .last()
            .is_some_and(|word| word.text.ends_with(['.', '!', '?']))
        {
            push(&mut cues, &mut current);
        }
    }
    push(&mut cues, &mut current);
    for index in 0..cues.len() {
        let lower = if index == 0 {
            start
        } else {
            cues[index - 1].end
        };
        let upper = cues.get(index + 1).map_or(end, |cue| cue.start);
        let cue = &mut cues[index];
        if cue.end - cue.start < 0.01 {
            cue.end = (cue.start + 0.01).min(upper);
            cue.start = (cue.end - 0.01).max(lower);
        }
        if cue.end - cue.start + 1e-9 < 0.01
            || cue.text.chars().count() > 500
            || cue.text.lines().count() > 3
        {
            return Err("The returned words cannot form valid readable subtitle cues.".into());
        }
    }
    Ok(cues)
}

fn same_word(first: &str, second: &str) -> bool {
    first
        .trim_matches(|character: char| !character.is_alphanumeric())
        .eq_ignore_ascii_case(second.trim_matches(|character: char| !character.is_alphanumeric()))
}

fn push(cues: &mut Vec<SubtitleCue>, words: &mut Vec<Word>) {
    if words.is_empty() {
        return;
    }
    let start = words[0].start;
    let end = words.iter().map(|word| word.end).fold(start, f64::max);
    let text = words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let words = words
        .drain(..)
        .map(|word| SubtitleWord {
            start: word.start,
            end: word.end,
            text: word.text,
        })
        .collect();
    cues.push(SubtitleCue {
        id: uuid::Uuid::new_v4().to_string(),
        start,
        end,
        text,
        words,
        source_ranges: vec![],
    });
}
