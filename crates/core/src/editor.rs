use crate::{bounded, EditSettings, TIME_EPSILON};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicClip {
    pub id: String,
    pub name: String,
    pub path: String,
    pub source_duration: f64,
    pub start: f64,
    pub source_start: f64,
    pub duration: f64,
    pub volume: f64,
    pub fade_in: f64,
    pub fade_out: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleCue {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default)]
    pub words: Vec<SubtitleWord>,
    #[serde(default)]
    pub source_ranges: Vec<SubtitleRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleWord {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleRange {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleStyle {
    pub enabled: bool,
    pub font_size: f64,
    pub color: String,
    pub background: String,
    pub position: String,
    #[serde(default)]
    pub highlight_words: bool,
    #[serde(default = "highlight_color")]
    pub highlight_color: String,
}
fn highlight_color() -> String {
    "#E9B451".into()
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            enabled: true,
            font_size: 42.0,
            color: "#FFFFFF".into(),
            background: "#000000".into(),
            position: "bottom".into(),
            highlight_words: false,
            highlight_color: highlight_color(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Redaction {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub mode: String,
    pub strength: f64,
    #[serde(default)]
    pub tracking: Option<crate::MaskTracking>,
}

fn identifier<'a>(id: &'a str, seen: &mut HashSet<&'a str>) -> Result<(), String> {
    if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) || !seen.insert(id) {
        return Err("Track items require unique, nonempty identifiers.".into());
    }
    Ok(())
}

fn interval(start: f64, end: f64, duration: f64, minimum: f64) -> Result<(), String> {
    bounded(start, 0.0, duration, "Item start")?;
    bounded(end, 0.0, duration + TIME_EPSILON, "Item end")?;
    if end - start + TIME_EPSILON < minimum {
        return Err(format!("Track items must last at least {minimum} seconds."));
    }
    Ok(())
}

fn color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

impl EditSettings {
    pub(crate) fn validate_editor_tracks(&self, duration: f64) -> Result<(), String> {
        if self.music.len() > 32 || self.subtitles.len() > 10_000 || self.redactions.len() > 100 {
            return Err(
                "Project limit: 32 music clips, 10000 subtitle cues, and 100 redactions.".into(),
            );
        }
        let mut ids = HashSet::new();
        for clip in &self.music {
            identifier(&clip.id, &mut ids)?;
            if clip.name.trim().is_empty()
                || clip.name.len() > 512
                || clip.name.chars().any(char::is_control)
                || clip.path.is_empty()
                || clip.path.len() > 32768
                || clip.path.contains('\0')
            {
                return Err("Invalid imported music name or path.".into());
            }
            bounded(
                clip.source_duration,
                0.1,
                24.0 * 3600.0,
                "Music source duration",
            )?;
            bounded(
                clip.source_start,
                0.0,
                clip.source_duration,
                "Music source start",
            )?;
            interval(clip.start, clip.start + clip.duration, duration, 0.1)?;
            if clip.source_start + clip.duration > clip.source_duration + TIME_EPSILON {
                return Err("Music clip exceeds its source duration.".into());
            }
            bounded(clip.volume, 0.0, 2.0, "Music volume")?;
            bounded(clip.fade_in, 0.0, clip.duration, "Music fade in")?;
            bounded(clip.fade_out, 0.0, clip.duration, "Music fade out")?;
            if clip.fade_in + clip.fade_out > clip.duration + TIME_EPSILON {
                return Err("Music fades must fit within the clip.".into());
            }
        }
        ids.clear();
        let mut cues: Vec<_> = self.subtitles.iter().collect();
        cues.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut text_bytes = 0usize;
        let mut word_count = 0usize;
        let mut coverage = Vec::new();
        for cue in cues {
            identifier(&cue.id, &mut ids)?;
            interval(cue.start, cue.end, duration, 0.01)?;
            crate::captions::validate_subtitle_metadata(cue)?;
            coverage.extend(crate::subtitle_ranges(cue));
            word_count += cue.words.len();
            if cue.text.trim().is_empty()
                || cue.text.chars().count() > 500
                || cue.text.split('\n').count() > 3
                || cue.text.chars().any(|c| c.is_control() && c != '\n')
            {
                return Err(
                    "Subtitles require 1-500 characters and at most three lines per cue.".into(),
                );
            }
            text_bytes += cue.text.len();
        }
        coverage.sort_by(|a, b| a.0.total_cmp(&b.0));
        if coverage.len() > 20_000
            || coverage
                .windows(2)
                .any(|pair| pair[1].0 + TIME_EPSILON < pair[0].1)
        {
            return Err(
                "Subtitle coverage must not overlap or exceed 20000 source intervals.".into(),
            );
        }
        if word_count > 200_000 {
            return Err("Subtitle word timing exceeds the 200000 word limit.".into());
        }
        if text_bytes > 1024 * 1024 {
            return Err("Subtitle text exceeds the 1 MiB limit.".into());
        }
        bounded(
            self.subtitle_style.font_size,
            12.0,
            96.0,
            "Subtitle font size",
        )?;
        if !color(&self.subtitle_style.color)
            || !color(&self.subtitle_style.background)
            || !color(&self.subtitle_style.highlight_color)
            || !["top", "bottom"].contains(&self.subtitle_style.position.as_str())
        {
            return Err("Invalid subtitle color or position.".into());
        }
        ids.clear();
        let mut changes = Vec::with_capacity(self.redactions.len() * 2);
        let mut tracking_frames = 0usize;
        for region in &self.redactions {
            identifier(&region.id, &mut ids)?;
            interval(region.start, region.end, duration, 0.1)?;
            bounded(region.x, 0.0, 1.0, "Redaction X")?;
            bounded(region.y, 0.0, 1.0, "Redaction Y")?;
            bounded(region.width, 0.001, 1.0, "Redaction width")?;
            bounded(region.height, 0.001, 1.0, "Redaction height")?;
            if region.x + region.width > 1.0 + TIME_EPSILON
                || region.y + region.height > 1.0 + TIME_EPSILON
                || !["blur", "solid"].contains(&region.mode.as_str())
            {
                return Err(
                    "Redaction must fit inside the original screen and use blur or solid mode."
                        .into(),
                );
            }
            bounded(region.strength, 1.0, 50.0, "Blur strength")?;
            if let Some(tracking) = &region.tracking {
                crate::tracking::validate_tracking(region, tracking)?;
                tracking_frames += tracking.keyframes.len();
            }
            changes.push((region.start, 1i32));
            changes.push((region.end, -1i32));
        }
        changes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        if tracking_frames > 20_000 {
            return Err("Mask tracking exceeds the 20000 keyframe project limit.".into());
        }
        let mut active = 0;
        for (_, delta) in changes {
            active += delta;
            if active > 16 {
                return Err("At most 16 redactions may be active at the same time.".into());
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 15 * 1024 * 1024 {
            return Err(
                "Editor metadata exceeds the project limit. Reduce the number of track items."
                    .into(),
            );
        }
        Ok(())
    }
}

pub fn subtitles_srt(edits: &EditSettings, duration: f64) -> Result<String, String> {
    edits.validate(duration)?;
    let timestamp = |seconds: f64| {
        let ms = (seconds.max(0.0) * 1000.0).round() as u64;
        format!(
            "{:02}:{:02}:{:02},{:03}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            ms % 1000
        )
    };
    let mut output = String::new();
    let intervals = crate::kept_intervals(edits, duration);
    let mut blocks = Vec::new();
    for cue in edits
        .subtitles
        .iter()
        .filter_map(|cue| crate::projected_subtitle(edits, duration, cue))
    {
        let mut fragments: Vec<(f64, f64)> = Vec::new();
        for (start, end) in crate::subtitle_ranges(&cue)
            .into_iter()
            .flat_map(|(start, end)| crate::project_interval(&intervals, start, end))
        {
            if let Some(previous) = fragments
                .last_mut()
                .filter(|previous| (previous.1 - start).abs() < 1e-9)
            {
                previous.1 = end;
            } else {
                fragments.push((start, end));
            }
        }
        for (start, end) in fragments {
            if (end * 1000.0).round() <= (start * 1000.0).round() {
                continue;
            }
            blocks.push((start, end, cue.text.clone()));
        }
    }
    blocks.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (index, (start, end, text)) in blocks.into_iter().enumerate() {
        output.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            timestamp(start),
            timestamp(end),
            text
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip() -> MusicClip {
        MusicClip {
            id: "music".into(),
            name: "Song".into(),
            path: "music.wav".into(),
            source_duration: 20.0,
            start: 1.0,
            source_start: 2.0,
            duration: 8.0,
            volume: 0.5,
            fade_in: 2.0,
            fade_out: 2.0,
        }
    }
    fn region(id: &str, start: f64, end: f64) -> Redaction {
        Redaction {
            id: id.into(),
            start,
            end,
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.2,
            mode: "solid".into(),
            strength: 12.0,
            tracking: None,
        }
    }
    #[test]
    fn long_timed_transcript_fits_the_manifest_metadata_budget() {
        let mut edits = EditSettings::new(12_000.0);
        edits.subtitles = (0..4_000)
            .map(|i| {
                let start = i as f64 * 3.0;
                let words: Vec<_> = (0..10)
                    .map(|j| SubtitleWord {
                        start: start + j as f64 * 0.25,
                        end: start + (j + 1) as f64 * 0.25,
                        text: format!("word{j}"),
                    })
                    .collect();
                SubtitleCue {
                    id: format!("cue{i}"),
                    start,
                    end: start + 2.5,
                    text: words
                        .iter()
                        .map(|word| word.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                    words,
                    source_ranges: vec![],
                }
            })
            .collect();
        assert!(serde_json::to_vec(&edits).unwrap().len() > 1_900_000);
        edits.validate(12_000.0).unwrap();
        assert!(subtitles_srt(&edits, 12_000.0).unwrap().contains("word9"));
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        project.duration = 12_000.0;
        project.edits = edits;
        crate::save_project_atomic(&project).unwrap();
        let restored = crate::load_project_metadata(root.path(), &project.id).unwrap();
        assert_eq!(restored.edits.subtitles.len(), 4_000);
        assert_eq!(
            restored
                .edits
                .subtitles
                .iter()
                .map(|cue| cue.words.len())
                .sum::<usize>(),
            40_000
        );
        assert_eq!(
            crate::load_project_summary(root.path(), &project.id)
                .unwrap()
                .duration,
            12_000.0
        );
        assert_eq!(
            std::fs::read(&project.screen_path).unwrap(),
            b"media-path-fixture"
        );
    }
    #[test]
    fn legacy_edits_default_to_empty_tracks() {
        let mut value = serde_json::to_value(EditSettings::new(10.0)).unwrap();
        for key in ["music", "subtitles", "subtitleStyle", "redactions"] {
            value.as_object_mut().unwrap().remove(key);
        }
        let edits: EditSettings = serde_json::from_value(value).unwrap();
        edits.validate(10.0).unwrap();
        assert!(
            edits.music.is_empty() && edits.subtitles.is_empty() && edits.redactions.is_empty()
        );
    }
    #[test]
    fn music_rejects_source_overrun_fades_duplicate_ids_and_nonfinite() {
        let mut edits = EditSettings::new(10.0);
        edits.music.push(clip());
        edits.validate(10.0).unwrap();
        edits.music[0].source_start = 15.0;
        assert!(edits.validate(10.0).is_err());
        edits.music[0] = clip();
        edits.music[0].fade_out = 7.0;
        assert!(edits.validate(10.0).is_err());
        edits.music[0] = clip();
        edits.music.push(clip());
        assert!(edits.validate(10.0).is_err());
        edits.music.pop();
        edits.music[0].volume = f64::NAN;
        assert!(edits.validate(10.0).is_err());
    }
    #[test]
    fn subtitle_overlap_and_srt_trim_are_checked() {
        let mut edits = EditSettings::new(10.0);
        edits.trim_start = 2.0;
        edits.trim_end = 8.0;
        edits.subtitles = vec![
            SubtitleCue {
                id: "a".into(),
                start: 1.0,
                end: 3.0,
                text: "Halo\ndunia".into(),
                words: vec![],
                source_ranges: vec![],
            },
            SubtitleCue {
                id: "b".into(),
                start: 7.5,
                end: 9.0,
                text: "Selesai".into(),
                words: vec![],
                source_ranges: vec![],
            },
        ];
        let srt = subtitles_srt(&edits, 10.0).unwrap();
        assert!(srt.contains("00:00:00,000 --> 00:00:01,000\nHalo\ndunia"));
        assert!(srt.contains("00:00:05,500 --> 00:00:06,000\nSelesai"));
        edits.subtitles[1].start = 2.99;
        assert!(edits.validate(10.0).is_err());
    }
    #[test]
    fn redaction_boundary_and_concurrency_are_checked() {
        let mut edits = EditSettings::new(10.0);
        edits.redactions = (0..16).map(|i| region(&i.to_string(), 0.0, 5.0)).collect();
        edits.redactions.push(region("next", 5.0, 10.0));
        edits.validate(10.0).unwrap();
        edits.redactions.push(region("overlap", 4.9, 5.1));
        assert!(edits.validate(10.0).is_err());
        edits.redactions.pop();
        edits.redactions[0].width = 0.95;
        assert!(edits.validate(10.0).is_err());
    }
}
