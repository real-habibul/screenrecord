use crate::{CameraSettings, EditSettings, SubtitleStyle};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppearanceSettings {
    pub aspect_ratio: String,
    #[serde(default = "crate::fit_mode_default")]
    pub fit_mode: String,
    pub background: String,
    #[serde(default = "crate::background_mode_default")]
    pub background_mode: String,
    #[serde(default = "crate::background_blur_default")]
    pub background_blur: f64,
    pub padding: u32,
    pub corner_radius: u32,
    pub cursor_size: f64,
    pub cursor_smoothing: f64,
    pub show_cursor: bool,
    #[serde(default = "crate::show_clicks_default")]
    pub show_clicks: bool,
    pub camera: CameraSettings,
    pub subtitle_style: SubtitleStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppearancePreset {
    pub id: String,
    pub name: String,
    pub settings: AppearanceSettings,
}

impl AppearanceSettings {
    pub fn from_edits(edits: &EditSettings) -> Self {
        Self {
            aspect_ratio: edits.aspect_ratio.clone(),
            fit_mode: edits.fit_mode.clone(),
            background: edits.background.clone(),
            background_mode: edits.background_mode.clone(),
            background_blur: edits.background_blur,
            padding: edits.padding,
            corner_radius: edits.corner_radius,
            cursor_size: edits.cursor_size,
            cursor_smoothing: edits.cursor_smoothing,
            show_cursor: edits.show_cursor,
            show_clicks: edits.show_clicks,
            camera: edits.camera.clone(),
            subtitle_style: edits.subtitle_style.clone(),
        }
    }
    pub fn apply_to(&self, edits: &mut EditSettings, has_camera: bool) {
        edits.aspect_ratio = self.aspect_ratio.clone();
        edits.fit_mode = self.fit_mode.clone();
        edits.background = self.background.clone();
        edits.background_mode = self.background_mode.clone();
        edits.background_blur = self.background_blur;
        edits.padding = self.padding;
        edits.corner_radius = self.corner_radius;
        edits.cursor_size = self.cursor_size;
        edits.cursor_smoothing = self.cursor_smoothing;
        edits.show_cursor = self.show_cursor;
        edits.show_clicks = self.show_clicks;
        edits.camera = self.camera.clone();
        edits.camera.enabled &= has_camera;
        edits.subtitle_style = self.subtitle_style.clone();
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut edits = EditSettings::new(1.0);
        self.apply_to(&mut edits, true);
        edits.validate(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CropSettings, Redaction, SubtitleCue, VideoClip};

    #[test]
    fn legacy_preset_enables_click_indicators_and_new_preset_preserves_toggle() {
        let mut edits = EditSettings::new(3.0);
        edits.show_clicks = false;
        let preset = AppearanceSettings::from_edits(&edits);
        let mut applied = EditSettings::new(3.0);
        preset.apply_to(&mut applied, false);
        assert!(!applied.show_clicks && applied.show_cursor);
        let mut legacy = serde_json::to_value(preset).unwrap();
        legacy.as_object_mut().unwrap().remove("showClicks");
        let restored: AppearanceSettings = serde_json::from_value(legacy).unwrap();
        restored.apply_to(&mut applied, false);
        assert!(applied.show_clicks);
    }

    #[test]
    fn video_background_defaults_bounds_and_preset_contract() {
        let mut legacy = serde_json::to_value(EditSettings::new(10.0)).unwrap();
        legacy.as_object_mut().unwrap().remove("backgroundMode");
        legacy.as_object_mut().unwrap().remove("backgroundBlur");
        let mut edits: EditSettings = serde_json::from_value(legacy).unwrap();
        assert_eq!(edits.background_mode, "solid");
        assert_eq!(edits.background_blur, 24.0);
        for mode in ["solid", "videoBlur"] {
            edits.background_mode = mode.into();
            for blur in [4.0, 64.0] {
                edits.background_blur = blur;
                edits.validate(10.0).unwrap();
            }
        }
        let preset = AppearanceSettings::from_edits(&edits);
        let mut stored = serde_json::to_value(&preset).unwrap();
        let restored: AppearanceSettings = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(restored.background_mode, "videoBlur");
        assert_eq!(restored.background_blur, 64.0);
        stored.as_object_mut().unwrap().remove("backgroundMode");
        stored.as_object_mut().unwrap().remove("backgroundBlur");
        let legacy_preset: AppearanceSettings = serde_json::from_value(stored).unwrap();
        assert_eq!(legacy_preset.background_mode, "solid");
        assert_eq!(legacy_preset.background_blur, 24.0);
        let mut injected = serde_json::to_value(&preset).unwrap();
        injected["reframeKeyframes"] = serde_json::json!([]);
        assert!(serde_json::from_value::<AppearanceSettings>(injected).is_err());
        for blur in [3.9, 64.1, f64::NAN, f64::INFINITY] {
            edits.background_blur = blur;
            assert!(edits.validate(10.0).is_err());
        }
        edits.background_blur = 24.0;
        edits.background_mode = "rawVideo".into();
        assert!(edits.validate(10.0).is_err());
    }

    #[test]
    fn screen_sizing_defaults_for_legacy_edits_and_presets_and_rejects_unknown_modes() {
        let mut legacy = serde_json::to_value(EditSettings::new(10.0)).unwrap();
        legacy.as_object_mut().unwrap().remove("fitMode");
        let mut edits: EditSettings = serde_json::from_value(legacy).unwrap();
        assert_eq!(edits.fit_mode, "fit");
        edits.fit_mode = "fill".into();
        edits.validate(10.0).unwrap();
        let mut preset = AppearanceSettings::from_edits(&edits);
        let mut legacy_preset = serde_json::to_value(&preset).unwrap();
        legacy_preset.as_object_mut().unwrap().remove("fitMode");
        assert_eq!(
            serde_json::from_value::<AppearanceSettings>(legacy_preset)
                .unwrap()
                .fit_mode,
            "fit"
        );
        let mut target = EditSettings::new(10.0);
        preset.apply_to(&mut target, false);
        assert_eq!(target.fit_mode, "fill");
        preset.fit_mode = "stretch".into();
        assert!(preset.validate().is_err());
        target.fit_mode = "stretch".into();
        assert!(target.validate(10.0).is_err());
    }
    #[test]
    fn appearance_presets_preserve_content_edits_and_reject_content_payloads() {
        let mut legacy = serde_json::to_value(EditSettings::new(10.0)).unwrap();
        legacy["camera"].as_object_mut().unwrap().remove("layout");
        assert_eq!(
            serde_json::from_value::<EditSettings>(legacy)
                .unwrap()
                .camera
                .layout,
            "bubble"
        );
        let mut edits = EditSettings::new(10.0);
        edits.trim_start = 1.0;
        edits.trim_end = 9.0;
        edits.video_clips = vec![VideoClip {
            id: "kept".into(),
            start: 2.0,
            end: 8.0,
        }];
        edits.crop = Some(CropSettings {
            x: 0.1,
            y: 0.1,
            width: 0.8,
            height: 0.8,
        });
        edits.subtitles.push(SubtitleCue {
            id: "speech".into(),
            start: 2.0,
            end: 3.0,
            text: "Existing speech".into(),
            words: vec![],
            source_ranges: vec![],
        });
        edits.redactions.push(Redaction {
            id: "private".into(),
            start: 2.0,
            end: 3.0,
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.2,
            mode: "solid".into(),
            strength: 1.0,
            tracking: None,
        });
        edits.audio_processing.microphone_gain = 2.0;
        let before = serde_json::to_value(&edits).unwrap();
        let mut settings = AppearanceSettings::from_edits(&EditSettings::new(10.0));
        settings.camera.layout = "fullscreen".into();
        settings.background = "#101010".into();
        settings.validate().unwrap();
        settings.apply_to(&mut edits, false);
        assert_eq!(edits.camera.layout, "fullscreen");
        assert!(!edits.camera.enabled);
        let after = serde_json::to_value(&edits).unwrap();
        for key in [
            "videoClips",
            "trimStart",
            "trimEnd",
            "crop",
            "zooms",
            "reframeKeyframes",
            "music",
            "subtitles",
            "redactions",
            "audioProcessing",
            "systemVolume",
            "microphoneVolume",
        ] {
            assert_eq!(before[key], after[key], "{key}");
        }
        let mut injected = serde_json::to_value(&settings).unwrap();
        injected["subtitles"] = serde_json::json!([]);
        assert!(serde_json::from_value::<AppearanceSettings>(injected).is_err());
        settings.camera.layout = "unknown".into();
        assert!(settings.validate().is_err());
    }
}
