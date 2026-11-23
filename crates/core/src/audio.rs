use crate::bounded;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_RECORDED_GAIN: f64 = 2.0;
pub const MAX_VOLUME_KEYFRAMES: usize = 2048;
pub const MIN_VOLUME_KEYFRAME_GAP: f64 = 0.001;
// Older projects can contain normalization gains up to 8x, including in Undo history.
const MAX_STORED_RECORDED_GAIN: f64 = 8.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DuckingSettings {
    pub enabled: bool,
    pub system_enabled: bool,
    pub amount_db: f64,
}

impl Default for DuckingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            system_enabled: false,
            amount_db: 12.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeKeyframe {
    pub id: String,
    pub time: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VolumeKeyframes {
    pub microphone: Vec<VolumeKeyframe>,
    pub system: Vec<VolumeKeyframe>,
    pub music: Vec<VolumeKeyframe>,
}

impl VolumeKeyframes {
    fn is_empty(&self) -> bool {
        self.microphone.is_empty() && self.system.is_empty() && self.music.is_empty()
    }

    fn validate(&self, duration: f64) -> Result<(), String> {
        for keys in [&self.microphone, &self.system, &self.music] {
            if keys.len() > MAX_VOLUME_KEYFRAMES {
                return Err("An audio track can contain at most 2048 volume keyframes.".into());
            }
            let mut ids = HashSet::with_capacity(keys.len());
            let mut previous = None;
            for key in keys {
                if key.id.trim().is_empty()
                    || key.id.len() > 128
                    || key.id.chars().any(char::is_control)
                    || !ids.insert(&key.id)
                {
                    return Err("Volume keyframes require unique nonempty identifiers.".into());
                }
                bounded(key.time, 0.0, duration, "Volume keyframe time")?;
                bounded(key.volume, 0.0, 1.0, "Volume keyframe level")?;
                if previous.is_some_and(|last| {
                    key.time - last + crate::TIME_EPSILON < MIN_VOLUME_KEYFRAME_GAP
                }) {
                    return Err(
                        "Volume keyframes must be ordered at least 0.001 seconds apart.".into(),
                    );
                }
                previous = Some(key.time);
            }
        }
        Ok(())
    }
}

pub fn volume_at(keys: &[VolumeKeyframe], time: f64) -> f64 {
    let right = keys.partition_point(|key| key.time <= time);
    let Some(left) = keys.get(right.saturating_sub(1)) else {
        return 1.0;
    };
    match keys.get(right) {
        Some(next) if next.time > left.time => {
            let fraction = ((time - left.time) / (next.time - left.time)).clamp(0.0, 1.0);
            left.volume + (next.volume - left.volume) * fraction
        }
        _ => left.volume,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AudioProcessing {
    pub microphone_gain: f64,
    pub system_gain: f64,
    pub ducking: DuckingSettings,
    #[serde(skip_serializing_if = "VolumeKeyframes::is_empty")]
    pub volume_keyframes: VolumeKeyframes,
}

impl Default for AudioProcessing {
    fn default() -> Self {
        Self {
            microphone_gain: 1.0,
            system_gain: 1.0,
            ducking: DuckingSettings::default(),
            volume_keyframes: VolumeKeyframes::default(),
        }
    }
}

impl AudioProcessing {
    pub(crate) fn validate(&self, duration: f64) -> Result<(), String> {
        bounded(
            self.microphone_gain,
            0.01,
            MAX_STORED_RECORDED_GAIN,
            "Microphone source gain",
        )?;
        bounded(
            self.system_gain,
            0.01,
            MAX_STORED_RECORDED_GAIN,
            "Computer audio source gain",
        )?;
        bounded(self.ducking.amount_db, 0.0, 24.0, "Audio ducking amount")?;
        self.volume_keyframes.validate(duration)
    }
}

#[cfg(test)]
mod tests {
    use super::{volume_at, VolumeKeyframe, MAX_VOLUME_KEYFRAMES};
    use crate::{load_project, save_project_atomic, save_project_edits, EditSettings};

    fn point(id: &str, time: f64, volume: f64) -> VolumeKeyframe {
        VolumeKeyframe {
            id: id.into(),
            time,
            volume,
        }
    }

    #[test]
    fn source_volume_points_interpolate_and_keep_their_position_after_trim_and_reopen() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        let points = vec![point("quiet", 1.0, 0.25), point("full", 2.0, 1.0)];
        assert_eq!(volume_at(&[], 1.0), 1.0);
        assert_eq!(volume_at(&points, 0.0), 0.25);
        assert_eq!(volume_at(&points, 1.5), 0.625);
        assert_eq!(volume_at(&points, 10.0), 1.0);
        project.edits.audio_processing.volume_keyframes.music = points;
        project.edits.trim_start = 1.5;
        save_project_atomic(&project).unwrap();
        let saved = load_project(root.path(), &project.id).unwrap();
        let keys = saved.edits.audio_processing.volume_keyframes;
        assert!(keys.microphone.is_empty() && keys.system.is_empty());
        assert_eq!(keys.music[0].time, 1.0);
        assert_eq!(volume_at(&keys.music, 1.5), 0.625);
    }

    #[test]
    fn rejects_out_of_range_unordered_duplicate_or_oversized_volume_curves() {
        let mut edits = EditSettings::new(10.0);
        for points in [
            vec![point("same", 1.0, 1.0), point("same", 2.0, 0.0)],
            vec![point("", 0.5, 1.0)],
            vec![point("a", f64::NAN, 1.0)],
            vec![point("a", 10.5, 0.0)],
            vec![point("a", 0.0, f64::INFINITY)],
            vec![point("a", 0.0, 1.01)],
            vec![point("a", 0.0, -0.1)],
            vec![point("a", 2.0, 1.0), point("b", 1.0, 0.0)],
            vec![point("a", 1.0, 1.0), point("b", 1.0001, 1.0)],
            (0..=MAX_VOLUME_KEYFRAMES)
                .map(|i| point(&i.to_string(), i as f64 * 0.002, 0.5))
                .collect(),
        ] {
            edits.audio_processing.volume_keyframes.system = points;
            assert!(edits.validate(10.0).is_err());
        }
        let legacy = serde_json::to_value(EditSettings::new(10.0)).unwrap();
        assert!(legacy["audioProcessing"].get("volumeKeyframes").is_none());
        let legacy: EditSettings = serde_json::from_value(legacy).unwrap();
        assert!(legacy.audio_processing.volume_keyframes.is_empty());
    }

    #[test]
    fn old_music_ducking_settings_do_not_lower_computer_audio() {
        let mut settings: super::DuckingSettings =
            serde_json::from_value(serde_json::json!({"enabled": true, "amountDb": 9})).unwrap();
        assert!(settings.enabled);
        assert!(!settings.system_enabled);
        settings.enabled = false;
        settings.system_enabled = true;
        let serialized = serde_json::to_value(settings).unwrap();
        assert_eq!(serialized["systemEnabled"], true);
        assert_eq!(serialized["enabled"], false);
    }

    #[test]
    fn saving_one_source_gain_preserves_the_other_tracks_legacy_gain() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        project.edits.audio_processing.microphone_gain = 6.25;
        project.edits.microphone_volume = 0.25;
        save_project_atomic(&project).unwrap();

        let mut edits = load_project(root.path(), &project.id).unwrap().edits;
        edits.audio_processing.system_gain = super::MAX_RECORDED_GAIN;
        save_project_edits(root.path(), &project.id, &project.name, edits).unwrap();
        let saved = load_project(root.path(), &project.id).unwrap().edits;
        assert_eq!(saved.audio_processing.microphone_gain, 6.25);
        assert_eq!(saved.microphone_volume, 0.25);
        assert_eq!(saved.audio_processing.system_gain, 2.0);
    }

    #[test]
    fn legacy_audio_defaults_and_processing_limits_are_checked() {
        let mut value = serde_json::to_value(EditSettings::new(3.0)).unwrap();
        value.as_object_mut().unwrap().remove("audioProcessing");
        let mut edits: EditSettings = serde_json::from_value(value).unwrap();
        assert_eq!(edits.audio_processing.microphone_gain, 1.0);
        assert!(!edits.audio_processing.ducking.enabled);
        edits.audio_processing.microphone_gain = f64::NAN;
        assert!(edits.validate(3.0).is_err());
        edits.audio_processing.microphone_gain = 8.0;
        edits.audio_processing.system_gain = 0.01;
        edits.audio_processing.ducking.amount_db = 24.0;
        edits.validate(3.0).unwrap();
        edits.audio_processing.ducking.amount_db = 24.01;
        assert!(edits.validate(3.0).is_err());
    }
}
