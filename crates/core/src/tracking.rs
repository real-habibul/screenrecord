use crate::{bounded, EditSettings, Redaction, TIME_EPSILON};
use serde::{Deserialize, Serialize};

pub const MAX_MASK_TRACKING_SECONDS: f64 = 300.0;
pub const MAX_MASK_TRACKING_KEYFRAMES: usize = 3002;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskKeyframe {
    pub time: f64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub confidence: f64,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskTracking {
    pub keyframes: Vec<MaskKeyframe>,
    pub reviewed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskRectangle {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub lost: bool,
}

impl MaskRectangle {
    fn from_key(key: &MaskKeyframe) -> Self {
        Self {
            x: key.x,
            y: key.y,
            width: key.width,
            height: key.height,
            lost: key.kind == "lost",
        }
    }
    fn full() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            lost: true,
        }
    }
}

pub fn redaction_rectangle_at(region: &Redaction, time: f64) -> MaskRectangle {
    let Some(keys) = region
        .tracking
        .as_ref()
        .map(|tracking| &tracking.keyframes)
        .filter(|keys| !keys.is_empty())
    else {
        return MaskRectangle {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
            lost: false,
        };
    };
    if !time.is_finite() || time < keys[0].time {
        return MaskRectangle::full();
    }
    let index = keys
        .partition_point(|key| key.time <= time)
        .saturating_sub(1);
    let left = &keys[index];
    if left.kind == "lost" {
        return MaskRectangle::full();
    }
    let Some(right) = keys.get(index + 1) else {
        return MaskRectangle::from_key(left);
    };
    if right.kind == "lost" {
        return MaskRectangle::full();
    }
    if right.kind == "manual" {
        return MaskRectangle::from_key(left);
    }
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    MaskRectangle {
        x,
        y,
        width: (left.x + left.width).max(right.x + right.width) - x,
        height: (left.y + left.height).max(right.y + right.height) - y,
        lost: false,
    }
}

pub(crate) fn validate_tracking(region: &Redaction, tracking: &MaskTracking) -> Result<(), String> {
    let keys = &tracking.keyframes;
    if region.end - region.start > MAX_MASK_TRACKING_SECONDS + TIME_EPSILON
        || keys.is_empty()
        || keys.len() > MAX_MASK_TRACKING_KEYFRAMES
    {
        return Err(
            "Tracked masks require 1-3002 keyframes and a range of at most 300 seconds.".into(),
        );
    }
    if (keys[0].time - region.start).abs() > TIME_EPSILON {
        return Err("Mask tracking must begin at the region start.".into());
    }
    let mut lost = false;
    let mut previous = None;
    for key in keys {
        bounded(key.time, region.start, region.end, "Mask keyframe time")?;
        if previous.is_some_and(|time| key.time <= time) {
            return Err("Mask keyframes must be strictly chronological.".into());
        }
        previous = Some(key.time);
        bounded(key.x, 0.0, 1.0, "Mask keyframe X")?;
        bounded(key.y, 0.0, 1.0, "Mask keyframe Y")?;
        bounded(key.width, 0.001, 1.0, "Mask keyframe width")?;
        bounded(key.height, 0.001, 1.0, "Mask keyframe height")?;
        bounded(key.confidence, 0.0, 1.0, "Mask tracking confidence")?;
        if key.x + key.width > 1.0 + TIME_EPSILON
            || key.y + key.height > 1.0 + TIME_EPSILON
            || !["auto", "manual", "lost"].contains(&key.kind.as_str())
        {
            return Err("Invalid mask keyframe rectangle or kind.".into());
        }
        if key.kind == "lost" {
            if key.x != 0.0 || key.y != 0.0 || key.width != 1.0 || key.height != 1.0 {
                return Err("Lost mask tracking must cover the full screen.".into());
            }
            lost = true;
        } else if key.kind == "manual" {
            lost = false;
        } else if lost {
            return Err(
                "Lost mask tracking requires a manual correction before automatic keyframes."
                    .into(),
            );
        }
    }
    Ok(())
}

pub fn validate_mask_tracking_review(edits: &EditSettings, duration: f64) -> Result<(), String> {
    let kept = crate::kept_intervals(edits, duration);
    if edits.redactions.iter().any(|region| {
        region
            .tracking
            .as_ref()
            .is_some_and(|tracking| !tracking.reviewed)
            && kept
                .iter()
                .any(|range| region.start < range.end && region.end > range.start)
    }) {
        return Err("Review tracked masks in the Masks panel before exporting.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_preview_reference_matches_fractional_boundaries_loss_and_manual_corrections() {
        #[derive(Deserialize)]
        struct Case {
            time: f64,
            #[serde(flatten)]
            rect: MaskRectangle,
        }
        #[derive(Deserialize)]
        struct Reference {
            region: Redaction,
            cases: Vec<Case>,
        }
        let reference: Reference = serde_json::from_str(include_str!(
            "../../../testdata/mask-tracking-reference.json"
        ))
        .unwrap();
        validate_tracking(
            &reference.region,
            reference.region.tracking.as_ref().unwrap(),
        )
        .unwrap();
        for case in reference.cases {
            let actual = redaction_rectangle_at(&reference.region, case.time);
            assert_eq!(actual.lost, case.rect.lost, "time {}", case.time);
            for (actual, expected) in [
                (actual.x, case.rect.x),
                (actual.y, case.rect.y),
                (actual.width, case.rect.width),
                (actual.height, case.rect.height),
            ] {
                assert!(
                    (actual - expected).abs() < 1e-12,
                    "time {}: {actual} != {expected}",
                    case.time
                );
            }
        }
    }
    fn key(time: f64, x: f64, kind: &str) -> MaskKeyframe {
        MaskKeyframe {
            time,
            x,
            y: 0.2,
            width: 0.2,
            height: 0.1,
            confidence: 0.95,
            kind: kind.into(),
        }
    }
    fn region() -> Redaction {
        Redaction {
            id: "mask".into(),
            start: 0.0,
            end: 2.0,
            x: 0.1,
            y: 0.2,
            width: 0.2,
            height: 0.1,
            mode: "blur".into(),
            strength: 16.0,
            tracking: Some(MaskTracking {
                reviewed: false,
                keyframes: vec![key(0.0, 0.1, "manual"), key(1.0, 0.5, "auto")],
            }),
        }
    }
    #[test]
    fn moving_masks_cover_endpoint_union_and_hold_at_manual_corrections() {
        let mut mask = region();
        let rect = redaction_rectangle_at(&mask, 0.5);
        assert!((rect.x - 0.1).abs() < 1e-9 && (rect.width - 0.6).abs() < 1e-9);
        assert_eq!(redaction_rectangle_at(&mask, 1.0).x, 0.5);
        mask.tracking.as_mut().unwrap().keyframes[1].kind = "manual".into();
        assert_eq!(redaction_rectangle_at(&mask, 0.9).width, 0.2);
        assert_eq!(redaction_rectangle_at(&mask, 1.0).x, 0.5);
    }
    #[test]
    fn loss_is_solid_fullscreen_until_an_explicit_manual_correction() {
        let mut mask = region();
        let mut lost = key(0.5, 0.0, "lost");
        lost.y = 0.0;
        lost.width = 1.0;
        lost.height = 1.0;
        mask.tracking.as_mut().unwrap().keyframes.insert(1, lost);
        assert!(validate_tracking(&mask, mask.tracking.as_ref().unwrap()).is_err());
        mask.tracking.as_mut().unwrap().keyframes[2].kind = "manual".into();
        validate_tracking(&mask, mask.tracking.as_ref().unwrap()).unwrap();
        for time in [0.0, 0.49, 0.5, 0.99] {
            assert!(redaction_rectangle_at(&mask, time).lost);
        }
        assert!(!redaction_rectangle_at(&mask, 1.0).lost);
        assert!(redaction_rectangle_at(&mask, f64::NAN).lost);
    }
    #[test]
    fn review_required_for_retained_tracked_masks_but_saving_is_allowed() {
        let mut edits = EditSettings::new(2.0);
        edits.redactions.push(region());
        edits.validate(2.0).unwrap();
        assert!(validate_mask_tracking_review(&edits, 2.0).is_err());
        edits.redactions[0].tracking.as_mut().unwrap().reviewed = true;
        validate_mask_tracking_review(&edits, 2.0).unwrap();
        edits.redactions[0].tracking.as_mut().unwrap().reviewed = false;
        edits.redactions[0].end = 0.5;
        edits.redactions[0]
            .tracking
            .as_mut()
            .unwrap()
            .keyframes
            .truncate(1);
        edits.trim_start = 0.5;
        validate_mask_tracking_review(&edits, 2.0).unwrap();
    }
    #[test]
    fn tracked_masks_roundtrip_and_invalid_save_preserves_previous_manifest() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        project.edits.redactions.push(region());
        crate::save_project_atomic(&project).unwrap();
        let path = std::path::Path::new(&project.directory).join("project.json");
        let original = std::fs::read(&path).unwrap();
        let restored = crate::load_project(root.path(), &project.id).unwrap();
        let tracking = restored.edits.redactions[0].tracking.as_ref().unwrap();
        assert!(!tracking.reviewed);
        assert_eq!(tracking.keyframes[1].time, 1.0);
        project.edits.redactions[0]
            .tracking
            .as_mut()
            .unwrap()
            .keyframes[0]
            .time = 0.5;
        assert!(crate::save_project_atomic(&project).is_err());
        assert_eq!(std::fs::read(path).unwrap(), original);
        assert_eq!(
            std::fs::read(&project.screen_path).unwrap(),
            b"media-path-fixture"
        );
    }
    #[test]
    fn keyframe_validation_rejects_empty_unanchored_nonfinite_outside_and_unbounded_data() {
        let original = region();
        let mut mask = original.clone();
        mask.tracking.as_mut().unwrap().keyframes.clear();
        assert!(validate_tracking(&mask, mask.tracking.as_ref().unwrap()).is_err());
        for case in 0..5 {
            let mut mask = original.clone();
            let tracking = mask.tracking.as_mut().unwrap();
            match case {
                0 => tracking.keyframes[0].time = 0.1,
                1 => tracking.keyframes[1].time = 0.0,
                2 => tracking.keyframes[1].confidence = f64::NAN,
                3 => tracking.keyframes[1].x = 0.9,
                _ => mask.end = 301.0,
            }
            assert!(validate_tracking(&mask, mask.tracking.as_ref().unwrap()).is_err());
        }
    }
}
