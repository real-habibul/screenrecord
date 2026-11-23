use crate::{EditSettings, TIME_EPSILON};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoClip {
    pub id: String,
    pub start: f64,
    pub end: f64,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeptInterval {
    pub start: f64,
    pub end: f64,
    pub output_start: f64,
}

impl EditSettings {
    pub(crate) fn validate_video_clips(&self, duration: f64) -> Result<(), String> {
        if self.video_clips.len() > 512 {
            return Err("A project supports at most 512 video clips.".into());
        }
        let mut ids = HashSet::new();
        let mut previous_end = 0.0;
        for clip in &self.video_clips {
            if clip.id.is_empty()
                || clip.id.len() > 128
                || clip.id.chars().any(char::is_control)
                || !ids.insert(&clip.id)
            {
                return Err("Video clips require unique nonempty identifiers.".into());
            }
            if !clip.start.is_finite()
                || !clip.end.is_finite()
                || clip.start < 0.0
                || clip.end > duration + TIME_EPSILON
                || clip.end - clip.start + TIME_EPSILON < 0.1
                || clip.start + TIME_EPSILON < previous_end
            {
                return Err("Video clips must be chronological, nonoverlapping and at least 0.1 seconds within the source.".into());
            }
            previous_end = clip.end;
        }
        if edited_duration(self, duration) + TIME_EPSILON < 0.1 {
            return Err("Keep at least 0.1 seconds of video inside the selected trim.".into());
        }
        Ok(())
    }
}

pub fn kept_intervals(edits: &EditSettings, duration: f64) -> Vec<KeptInterval> {
    let ranges: Vec<_> = if edits.video_clips.is_empty() {
        vec![(edits.trim_start, edits.trim_end.min(duration))]
    } else {
        edits
            .video_clips
            .iter()
            .map(|clip| {
                (
                    clip.start.max(edits.trim_start),
                    clip.end.min(edits.trim_end).min(duration),
                )
            })
            .collect()
    };
    let mut output = Vec::new();
    let mut time = 0.0;
    for (start, end) in ranges {
        if end > start {
            output.push(KeptInterval {
                start,
                end,
                output_start: time,
            });
            time += end - start;
        }
    }
    output
}
pub fn edited_duration(edits: &EditSettings, duration: f64) -> f64 {
    kept_intervals(edits, duration)
        .iter()
        .map(|range| range.end - range.start)
        .sum()
}
pub fn source_at_output(intervals: &[KeptInterval], output: f64) -> Option<f64> {
    for interval in intervals {
        let end = interval.output_start + interval.end - interval.start;
        if output < end - TIME_EPSILON {
            return Some(interval.start + (output - interval.output_start).max(0.0));
        }
    }
    intervals.last().map(|interval| interval.end)
}
pub fn output_at_source(intervals: &[KeptInterval], source: f64) -> f64 {
    for interval in intervals {
        if source < interval.start {
            return interval.output_start;
        }
        if source < interval.end {
            return interval.output_start + source - interval.start;
        }
    }
    intervals.last().map_or(0.0, |interval| {
        interval.output_start + interval.end - interval.start
    })
}
pub fn project_interval(intervals: &[KeptInterval], start: f64, end: f64) -> Vec<(f64, f64)> {
    intervals
        .iter()
        .filter_map(|range| {
            let from = start.max(range.start);
            let to = end.min(range.end);
            (to > from).then_some((
                range.output_start + from - range.start,
                range.output_start + to - range.start,
            ))
        })
        .collect()
}

pub fn retained_subtitle(edits: &EditSettings, duration: f64, cue: &crate::SubtitleCue) -> bool {
    crate::projected_subtitle(edits, duration, cue).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapping_skips_gaps_and_split_does_not_change_duration() {
        let mut edits = EditSettings::new(10.0);
        edits.video_clips = vec![
            VideoClip {
                id: "a".into(),
                start: 1.0,
                end: 3.0,
            },
            VideoClip {
                id: "b".into(),
                start: 6.0,
                end: 9.0,
            },
        ];
        edits.validate(10.0).unwrap();
        let ranges = kept_intervals(&edits, 10.0);
        assert_eq!(edited_duration(&edits, 10.0), 5.0);
        assert_eq!(source_at_output(&ranges, 2.0), Some(6.0));
        assert_eq!(source_at_output(&ranges, 4.0), Some(8.0));
        assert_eq!(output_at_source(&ranges, 4.0), 2.0);
        assert_eq!(
            project_interval(&ranges, 2.0, 7.0),
            vec![(1.0, 2.0), (2.0, 3.0)]
        );
        edits.video_clips[1].start = 3.0;
        assert_eq!(edited_duration(&edits, 10.0), 8.0);
    }
    #[test]
    fn rejects_invalid_edls_and_empty_effective_output() {
        let mut edits = EditSettings::new(10.0);
        edits.video_clips = vec![VideoClip {
            id: "a".into(),
            start: 1.0,
            end: 2.0,
        }];
        edits.trim_start = 3.0;
        assert!(edits.validate(10.0).is_err());
        edits.trim_start = 0.0;
        edits.video_clips[0].end = f64::NAN;
        assert!(edits.validate(10.0).is_err());
        edits.video_clips[0].end = 1.09;
        assert!(edits.validate(10.0).is_err());
        edits.video_clips[0].end = 2.0;
        edits.video_clips.push(VideoClip {
            id: "b".into(),
            start: 1.5,
            end: 3.0,
        });
        assert!(edits.validate(10.0).is_err());
    }

    #[test]
    fn subtitles_and_srt_never_repeat_text_from_a_removed_span() {
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
        edits.subtitles = vec![
            crate::SubtitleCue {
                id: "cross".into(),
                start: 0.5,
                end: 2.5,
                text: "Removed words must not leak".into(),
                words: vec![],
                source_ranges: vec![],
            },
            crate::SubtitleCue {
                id: "kept".into(),
                start: 3.0,
                end: 3.5,
                text: "Kept words".into(),
                words: vec![],
                source_ranges: vec![],
            },
        ];
        let srt = crate::subtitles_srt(&edits, 4.0).unwrap();
        assert!(!srt.contains("Removed"));
        assert!(srt.contains("00:00:02,000 --> 00:00:02,500\nKept words"));
        edits.video_clips[1].start = 1.0;
        assert!(retained_subtitle(&edits, 4.0, &edits.subtitles[0]));
    }
}
