use crate::{Project, TIME_EPSILON};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemAudioScope {
    #[default]
    System,
    Application,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl CaptureRegion {
    pub fn validate_bounds(&self, source_width: u32, source_height: u32) -> Result<(), String> {
        if self.width < 2
            || self.height < 2
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
            || self
                .x
                .checked_add(self.width)
                .is_none_or(|right| right > source_width)
            || self
                .y
                .checked_add(self.height)
                .is_none_or(|bottom| bottom > source_height)
        {
            return Err("Choose an area inside the source with an even width and height of at least 2 pixels.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInterval {
    pub start: f64,
    pub end: f64,
}

impl Project {
    pub(crate) fn validate_camera_intervals(&self) -> Result<(), String> {
        let Some(intervals) = &self.camera_intervals else {
            return Ok(());
        };
        if intervals.len() > 512 || (!intervals.is_empty() && self.camera_path.is_none()) {
            return Err(
                "Camera availability requires recorded media and at most 512 intervals.".into(),
            );
        }
        let mut previous = 0.0;
        for interval in intervals {
            if !interval.start.is_finite()
                || !interval.end.is_finite()
                || interval.start < 0.0
                || interval.start + TIME_EPSILON < previous
                || interval.end <= interval.start
                || interval.end > self.duration + TIME_EPSILON
            {
                return Err("Camera availability must contain ordered nonoverlapping intervals inside the recording.".into());
            }
            previous = interval.end;
        }
        Ok(())
    }
}

pub fn camera_visible_at(project: &Project, time: f64) -> bool {
    project.camera_path.is_some()
        && project.camera_intervals.as_ref().is_none_or(|intervals| {
            intervals
                .iter()
                .any(|interval| time >= interval.start && time < interval.end)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RecordingOptions;

    #[test]
    fn existing_recordings_default_to_full_source_and_system_mix() {
        let json = serde_json::json!({"sourceId":"monitor:1","fps":30,"systemAudio":true});
        let mut options: RecordingOptions = serde_json::from_value(json.clone()).unwrap();
        assert!(options.region.is_none());
        assert_eq!(options.system_audio_scope, SystemAudioScope::System);
        options.validate().unwrap();
        options.system_audio_scope = SystemAudioScope::Application;
        assert!(options.validate().is_err());
        options.source_id = "app:42:73".into();
        options.validate().unwrap();
        options.system_audio = false;
        assert!(options.validate().is_err());
        let mut unknown = json;
        unknown["systemAudioScope"] = "other".into();
        assert!(serde_json::from_value::<RecordingOptions>(unknown).is_err());
    }

    #[test]
    fn capture_region_validates_edges_even_dimensions_and_overflow() {
        let valid = CaptureRegion {
            x: 5,
            y: 3,
            width: 8,
            height: 4,
        };
        valid.validate_bounds(13, 7).unwrap();
        for region in [
            CaptureRegion { x: 6, ..valid },
            CaptureRegion { y: 4, ..valid },
            CaptureRegion {
                x: u32::MAX,
                ..valid
            },
            CaptureRegion { width: 0, ..valid },
            CaptureRegion { height: 3, ..valid },
        ] {
            assert!(region.validate_bounds(13, 7).is_err());
        }
    }
    #[test]
    fn camera_availability_roundtrips_and_gates_half_open_gaps() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        project.camera_path = Some(project.screen_path.clone());
        assert!(camera_visible_at(&project, 1.0));
        project.camera_intervals = Some(vec![
            CameraInterval {
                start: 0.0,
                end: 1.0,
            },
            CameraInterval {
                start: 2.0,
                end: 3.0,
            },
        ]);
        project.validate().unwrap();
        assert!(camera_visible_at(&project, 0.0));
        assert!(!camera_visible_at(&project, 1.0));
        assert!(camera_visible_at(&project, 2.0));
        assert!(!camera_visible_at(&project, 3.0));
        crate::save_project_atomic(&project).unwrap();
        let loaded = crate::load_project(root.path(), &project.id).unwrap();
        assert_eq!(loaded.camera_intervals.as_ref().unwrap().len(), 2);
        assert!(!camera_visible_at(&loaded, 1.5));
        project.camera_intervals.as_mut().unwrap()[1].start = 0.5;
        assert!(project.validate().is_err());
        project.camera_intervals = Some(vec![]);
        assert!(!camera_visible_at(&project, 0.0));
    }
}
