use crate::{edited_duration, Project};
use serde::{Deserialize, Serialize};

pub const WHATSAPP_MAX_BYTES: u64 = 16_000_000;
pub const WHATSAPP_MIN_VIDEO_BITRATE: u32 = 320_000;
const AUDIO_BITRATE: u32 = 96_000;
const MAX_VIDEO_BITRATE: u32 = 2_400_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportPreset {
    #[default]
    Standard,
    Whatsapp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsappExportPlan {
    pub width: u32,
    pub height: u32,
    pub video_bitrate: u32,
    pub audio_bitrate: u32,
}

pub fn whatsapp_export_plan(project: &Project) -> Result<WhatsappExportPlan, String> {
    let ratio = match project.edits.aspect_ratio.as_str() {
        "source" => {
            let crop = project.edits.crop.as_ref();
            let width = f64::from(project.width) * crop.map_or(1.0, |crop| crop.width);
            let height = f64::from(project.height) * crop.map_or(1.0, |crop| crop.height);
            width / height
        }
        "16:9" => 16.0 / 9.0,
        "9:16" => 9.0 / 16.0,
        "1:1" => 1.0,
        _ => return Err("Unsupported canvas aspect ratio.".into()),
    };
    plan(edited_duration(&project.edits, project.duration), ratio)
}

fn plan(duration: f64, ratio: f64) -> Result<WhatsappExportPlan, String> {
    if !duration.is_finite() || duration <= 0.0 || !ratio.is_finite() || ratio <= 0.0 {
        return Err("Choose a playable video and a valid canvas before exporting.".into());
    }
    let duration = (duration.mul_add(30.0, -1e-7).ceil().max(1.0)) / 30.0;
    // Leave room for bitrate variation, MP4 headers and AAC even when the project has no audio.
    let budget_bits = (WHATSAPP_MAX_BYTES as f64 * 0.9 - 65_536.0) * 8.0;
    let video_bitrate =
        ((budget_bits / duration - f64::from(AUDIO_BITRATE)) / 1000.0).floor() * 1000.0;
    if video_bitrate < f64::from(WHATSAPP_MIN_VIDEO_BITRATE) {
        return Err("This video is too long for 16 MB at the minimum bitrate. Shorten the video or choose Standard.".into());
    }
    let video_bitrate = video_bitrate.min(f64::from(MAX_VIDEO_BITRATE)) as u32;
    let (short_limit, long_limit) = match video_bitrate {
        1_000_000.. => (720.0_f64, 1280.0_f64),
        550_000.. => (480.0, 854.0),
        _ => (360.0, 640.0),
    };
    let horizontal = ratio.max(1.0);
    let vertical = ratio.recip().max(1.0);
    let scale = short_limit.min(long_limit / horizontal.max(vertical));
    let even = |value: f64| (value.round() as u32 / 2) * 2;
    let (width, height) = (even(scale * horizontal), even(scale * vertical));
    if width < 64 || height < 64 {
        return Err(
            "This canvas is too narrow for WhatsApp export. Choose 16:9, 9:16, or 1:1 in Canvas."
                .into(),
        );
    }
    Ok(WhatsappExportPlan {
        width,
        height,
        video_bitrate,
        audio_bitrate: AUDIO_BITRATE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CropSettings, ExportOptions, VideoClip};

    #[test]
    fn export_preset_is_backward_compatible_and_rejects_mixed_settings() {
        let directory = tempfile::tempdir().unwrap();
        let value = serde_json::json!({ "outputPath": directory.path().join("clip.mp4"), "resolution": "1080p", "fps": 60 });
        let mut options: ExportOptions = serde_json::from_value(value).unwrap();
        assert_eq!(options.preset, ExportPreset::Standard);
        assert!(options.validate().is_ok());
        options.preset = ExportPreset::Whatsapp;
        assert!(options.validate().is_err());
        options.resolution = "auto".into();
        options.fps = 30;
        assert!(options.validate().is_ok());
        assert_eq!(
            serde_json::to_value(&options).unwrap()["preset"],
            "whatsapp"
        );
        options.preset = ExportPreset::Standard;
        assert!(options.validate().is_err());
    }

    #[test]
    fn share_plan_preserves_aspect_and_lowers_resolution_for_long_clips() {
        for (duration, ratio, width, height) in [
            (15.0, 16.0 / 9.0, 1280, 720),
            (60.0, 9.0 / 16.0, 720, 1280),
            (120.0, 16.0 / 9.0, 852, 480),
            (240.0, 9.0 / 16.0, 360, 640),
            (30.0, 1.0, 720, 720),
            (30.0, 4.0, 1280, 320),
        ] {
            let result = plan(duration, ratio).unwrap();
            assert_eq!((result.width, result.height), (width, height));
            assert!(result.video_bitrate >= WHATSAPP_MIN_VIDEO_BITRATE);
            assert!(result.video_bitrate <= MAX_VIDEO_BITRATE);
            assert!(
                (f64::from(result.video_bitrate + result.audio_bitrate) * duration / 8.0)
                    < WHATSAPP_MAX_BYTES as f64
            );
        }
        assert!(plan(300.0, 1.0).unwrap_err().contains("Shorten"));
        assert!(plan(30.0, 40.0).unwrap_err().contains("too narrow"));
        assert!(plan(f64::NAN, 1.0).is_err());
    }

    #[test]
    fn share_plan_uses_retained_duration_and_cropped_source_aspect() {
        let directory = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(directory.path());
        project.duration = 400.0;
        project.edits.trim_end = 400.0;
        assert!(whatsapp_export_plan(&project).is_err());
        project.edits.crop = Some(CropSettings {
            x: 0.25,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        });
        project.edits.video_clips = vec![
            VideoClip {
                id: "head".into(),
                start: 0.0,
                end: 30.0,
            },
            VideoClip {
                id: "tail".into(),
                start: 370.0,
                end: 400.0,
            },
        ];
        assert_eq!(
            whatsapp_export_plan(&project).unwrap(),
            plan(60.0, 8.0 / 9.0).unwrap()
        );
    }
}
