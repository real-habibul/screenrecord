use crate::{bounded, EditSettings, TIME_EPSILON};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_REFRAME_KEYFRAMES: usize = 3000;
pub const MIN_REFRAME_KEYFRAME_GAP: f64 = 0.001;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReframeKeyframe {
    pub id: String,
    pub time: f64,
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReframeSample {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

pub fn reframe_at(keys: &[ReframeKeyframe], time: f64) -> Option<ReframeSample> {
    if !time.is_finite() || keys.is_empty() {
        return None;
    }
    let right = keys.partition_point(|key| key.time <= time);
    let left = &keys[right.saturating_sub(1)];
    let next = keys.get(right).unwrap_or(left);
    let amount = if next.time > left.time {
        ((time - left.time) / (next.time - left.time)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some(ReframeSample {
        x: left.x + (next.x - left.x) * amount,
        y: left.y + (next.y - left.y) * amount,
        scale: left.scale + (next.scale - left.scale) * amount,
    })
}

impl EditSettings {
    pub(crate) fn validate_reframe(&self, duration: f64) -> Result<(), String> {
        if self.reframe_keyframes.len() > MAX_REFRAME_KEYFRAMES {
            return Err("A project can contain at most 3000 reframe keyframes.".into());
        }
        let mut ids = HashSet::new();
        let mut previous = None;
        for key in &self.reframe_keyframes {
            if key.id.trim().is_empty()
                || key.id.len() > 128
                || key.id.chars().any(char::is_control)
                || !ids.insert(&key.id)
            {
                return Err("Reframe keyframes require unique nonempty identifiers.".into());
            }
            bounded(key.time, 0.0, duration, "Reframe time")?;
            bounded(key.x, 0.0, 1.0, "Reframe horizontal focus")?;
            bounded(key.y, 0.0, 1.0, "Reframe vertical focus")?;
            bounded(key.scale, 1.0, 3.0, "Reframe scale")?;
            if previous
                .is_some_and(|last| key.time - last + TIME_EPSILON < MIN_REFRAME_KEYFRAME_GAP)
            {
                return Err(
                    "Reframe keyframes must be ordered at least 0.001 seconds apart.".into(),
                );
            }
            previous = Some(key.time);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str, time: f64, x: f64, scale: f64) -> ReframeKeyframe {
        ReframeKeyframe {
            id: id.into(),
            time,
            x,
            y: 0.5,
            scale,
        }
    }

    #[test]
    fn interpolation_holds_endpoints_and_interpolates_in_source_time() {
        let keys = vec![key("a", 2.0, 0.2, 1.0), key("b", 6.0, 0.8, 3.0)];
        assert_eq!(reframe_at(&[], 0.0), None);
        assert_eq!(reframe_at(&keys, f64::NAN), None);
        assert_eq!(reframe_at(&keys, 0.0).unwrap().x, 0.2);
        let middle = reframe_at(&keys, 4.0).unwrap();
        assert!((middle.x - 0.5).abs() < 1e-12);
        assert_eq!(middle.scale, 2.0);
        assert_eq!(reframe_at(&keys, 6.0).unwrap().x, 0.8);
        assert_eq!(reframe_at(&keys, 10.0).unwrap().scale, 3.0);
    }

    #[test]
    fn validation_rejects_duplicate_order_nonfinite_geometry_and_limits() {
        let mut edits = EditSettings::new(10.0);
        let valid = vec![key("a", 0.0, 0.0, 1.0), key("b", 0.001, 1.0, 3.0)];
        edits.reframe_keyframes = valid.clone();
        edits.validate(10.0).unwrap();
        for keys in [
            vec![key("a", 1.0, 0.5, 1.0), key("a", 2.0, 0.5, 1.0)],
            vec![key("a", 2.0, 0.5, 1.0), key("b", 1.0, 0.5, 1.0)],
            vec![key("a", 1.0, 0.5, 1.0), key("b", 1.0001, 0.5, 1.0)],
            vec![key("a", f64::NAN, 0.5, 1.0)],
            vec![key("a", 11.0, 0.5, 1.0)],
            vec![key("a", 1.0, f64::INFINITY, 1.0)],
            vec![key("a", 1.0, -0.1, 1.0)],
            vec![key("a", 1.0, 0.5, 3.01)],
        ] {
            edits.reframe_keyframes = keys;
            assert!(edits.validate(10.0).is_err());
        }
        edits.reframe_keyframes = (0..=MAX_REFRAME_KEYFRAMES)
            .map(|i| key(&format!("key{i}"), i as f64 * 0.002, 0.5, 1.0))
            .collect();
        assert!(edits.validate(10.0).is_err());
    }

    #[test]
    fn trim_cuts_and_appearance_presets_preserve_source_keyframes() {
        let mut edits = EditSettings::new(10.0);
        edits.reframe_keyframes = vec![key("before", 0.0, 0.2, 1.0), key("after", 10.0, 0.8, 2.0)];
        edits.trim_start = 2.0;
        edits.trim_end = 8.0;
        edits.video_clips = vec![
            crate::VideoClip {
                id: "left".into(),
                start: 0.0,
                end: 4.0,
            },
            crate::VideoClip {
                id: "right".into(),
                start: 6.0,
                end: 10.0,
            },
        ];
        edits.validate(10.0).unwrap();
        let before = serde_json::to_value(&edits.reframe_keyframes).unwrap();
        crate::AppearanceSettings::from_edits(&EditSettings::new(1.0)).apply_to(&mut edits, false);
        assert_eq!(
            serde_json::to_value(&edits.reframe_keyframes).unwrap(),
            before
        );
        assert!((reframe_at(&edits.reframe_keyframes, 6.0).unwrap().x - 0.56).abs() < 1e-12);
    }
}
