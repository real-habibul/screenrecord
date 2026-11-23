use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

mod audio;
mod captions;
mod clicks;
mod editor;
mod exporting;
mod framing;
mod presets;
mod recording;
mod srt;
mod storage;
mod timeline;
mod tracking;
pub use audio::*;
pub use captions::*;
pub use clicks::*;
pub use editor::*;
pub use exporting::*;
pub use framing::*;
pub use presets::*;
pub use recording::*;
pub use srt::*;
pub use storage::{
    load_project, load_project_metadata, load_project_summary, save_project_atomic,
    save_project_edits,
};
pub use timeline::*;
pub use tracking::*;

pub const PROJECT_VERSION: u32 = 7;
pub const MAX_PENDING_PROJECT_BYTES: u64 = 64 * 1024;
pub const TIME_EPSILON: f64 = 1e-9;
pub const MAX_RECORDING_SECONDS: f64 = 8.0 * 3600.0;
pub const MAX_RECORDING_RECOVERY_SECONDS: f64 = MAX_RECORDING_SECONDS + 60.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSource {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureApp {
    pub id: String,
    pub name: String,
    pub windows: Vec<CaptureSource>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDevices {
    pub sources: Vec<CaptureSource>,
    #[serde(default)]
    pub apps: Vec<CaptureApp>,
    pub microphones: Vec<Device>,
    pub cameras: Vec<Device>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingOptions {
    pub source_id: String,
    pub fps: u32,
    pub microphone_id: Option<String>,
    pub system_audio: bool,
    #[serde(default)]
    pub system_audio_scope: SystemAudioScope,
    pub camera_id: Option<String>,
    #[serde(default)]
    pub region: Option<CaptureRegion>,
}
impl RecordingOptions {
    pub fn validate(&self) -> Result<(), String> {
        if ![30, 60].contains(&self.fps) {
            return Err("Recording frame rate must be 30 or 60 fps.".into());
        }
        if self.source_id.is_empty() || self.source_id.len() > 512 {
            return Err("Select a valid capture source.".into());
        }
        if self.system_audio_scope == SystemAudioScope::Application
            && (!self.system_audio || !self.source_id.starts_with("app:"))
        {
            return Err(
                "Select a window in Apps and enable audio to record application audio.".into(),
            );
        }
        if let Some(region) = self.region {
            region.validate_bounds(16_384, 16_384)?;
        }
        for id in [&self.microphone_id, &self.camera_id].into_iter().flatten() {
            if id.is_empty() || id.len() > 4096 || id.contains('\0') {
                return Err("Invalid recording device.".into());
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorSample {
    pub time: f64,
    pub x: f64,
    pub y: f64,
    pub pressed: bool,
    #[serde(default, skip_serializing_if = "no_buttons")]
    pub buttons: u8,
    pub visible: bool,
}
fn no_buttons(buttons: &u8) -> bool {
    *buttons == 0
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomSegment {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub scale: f64,
    pub x: f64,
    pub y: f64,
    pub follow_cursor: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraSettings {
    #[serde(default = "bubble_layout")]
    pub layout: String,
    pub enabled: bool,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub shape: String,
    pub mirror: bool,
}
impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            layout: bubble_layout(),
            enabled: true,
            x: 0.73,
            y: 0.7,
            width: 0.22,
            shape: "circle".into(),
            mirror: true,
        }
    }
}
fn bubble_layout() -> String {
    "bubble".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropSettings {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditSettings {
    #[serde(default)]
    pub video_clips: Vec<VideoClip>,
    pub trim_start: f64,
    pub trim_end: f64,
    pub aspect_ratio: String,
    #[serde(default = "fit_mode_default")]
    pub fit_mode: String,
    #[serde(default)]
    pub reframe_keyframes: Vec<ReframeKeyframe>,
    pub background: String,
    #[serde(default = "background_mode_default")]
    pub background_mode: String,
    #[serde(default = "background_blur_default")]
    pub background_blur: f64,
    pub padding: u32,
    pub corner_radius: u32,
    pub cursor_size: f64,
    pub cursor_smoothing: f64,
    pub show_cursor: bool,
    #[serde(default = "show_clicks_default")]
    pub show_clicks: bool,
    pub zooms: Vec<ZoomSegment>,
    pub camera: CameraSettings,
    pub crop: Option<CropSettings>,
    pub system_volume: f64,
    pub microphone_volume: f64,
    #[serde(default)]
    pub audio_processing: AudioProcessing,
    #[serde(default)]
    pub music: Vec<MusicClip>,
    #[serde(default)]
    pub subtitles: Vec<SubtitleCue>,
    #[serde(default)]
    pub subtitle_style: SubtitleStyle,
    #[serde(default)]
    pub redactions: Vec<Redaction>,
}
impl EditSettings {
    pub fn new(duration: f64) -> Self {
        Self {
            trim_start: 0.0,
            video_clips: vec![],
            trim_end: duration,
            aspect_ratio: "source".into(),
            fit_mode: fit_mode_default(),
            reframe_keyframes: vec![],
            background: "#20242b".into(),
            background_mode: background_mode_default(),
            background_blur: background_blur_default(),
            padding: 32,
            corner_radius: 12,
            cursor_size: 1.5,
            cursor_smoothing: 0.5,
            show_cursor: true,
            show_clicks: true,
            zooms: vec![],
            camera: CameraSettings::default(),
            crop: None,
            system_volume: 1.0,
            microphone_volume: 1.0,
            audio_processing: AudioProcessing::default(),
            music: vec![],
            subtitles: vec![],
            subtitle_style: SubtitleStyle::default(),
            redactions: vec![],
        }
    }
    pub fn validate(&self, duration: f64) -> Result<(), String> {
        if !duration.is_finite() || duration <= 0.0 {
            return Err("Project has no playable duration.".into());
        }
        if !self.trim_start.is_finite()
            || !self.trim_end.is_finite()
            || self.trim_start < 0.0
            || self.trim_end > duration + 0.05
            || self.trim_end - self.trim_start + TIME_EPSILON < 0.1
        {
            return Err("Trim must contain at least 0.1 seconds within the recording.".into());
        }
        if !["source", "16:9", "9:16", "1:1"].contains(&self.aspect_ratio.as_str()) {
            return Err("Unsupported aspect ratio.".into());
        }
        if !["fit", "fill"].contains(&self.fit_mode.as_str()) {
            return Err("Screen sizing must be Fit or Fill.".into());
        }
        self.validate_reframe(duration)?;
        if self.background.len() != 7
            || !self.background.starts_with('#')
            || !self.background[1..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Background must be a six-digit hex color.".into());
        }
        if !["solid", "videoBlur"].contains(&self.background_mode.as_str()) {
            return Err("Background mode must be Solid or Video blur.".into());
        }
        bounded(self.background_blur, 4.0, 64.0, "Background blur")?;
        if self.padding > 240 || self.corner_radius > 160 {
            return Err("Canvas spacing is out of range.".into());
        }
        bounded(self.cursor_size, 0.5, 4.0, "Cursor size")?;
        bounded(self.cursor_smoothing, 0.0, 1.0, "Cursor smoothing")?;
        bounded(self.system_volume, 0.0, 2.0, "System volume")?;
        bounded(self.microphone_volume, 0.0, 2.0, "Microphone volume")?;
        self.audio_processing.validate(duration)?;
        bounded(self.camera.x, 0.0, 1.0, "Camera X")?;
        bounded(self.camera.y, 0.0, 1.0, "Camera Y")?;
        bounded(self.camera.width, 0.08, 0.6, "Camera width")?;
        if !["bubble", "fullscreen", "split"].contains(&self.camera.layout.as_str()) {
            return Err("Unsupported camera layout.".into());
        }
        if !["circle", "rectangle"].contains(&self.camera.shape.as_str()) {
            return Err("Unsupported camera shape.".into());
        }
        if let Some(crop) = &self.crop {
            bounded(crop.x, 0.0, 1.0, "Crop X")?;
            bounded(crop.y, 0.0, 1.0, "Crop Y")?;
            bounded(crop.width, 0.05, 1.0, "Crop width")?;
            bounded(crop.height, 0.05, 1.0, "Crop height")?;
            if crop.x + crop.width > 1.00001 || crop.y + crop.height > 1.00001 {
                return Err("Crop exceeds the recording bounds.".into());
            }
        }
        if self.zooms.len() > 2000 {
            return Err("A project can contain at most 2000 zoom segments.".into());
        }
        let mut previous_end = 0.0;
        let mut sorted: Vec<_> = self.zooms.iter().collect();
        sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
        for zoom in sorted {
            if zoom.id.is_empty() || zoom.id.len() > 128 {
                return Err("Invalid zoom identifier.".into());
            }
            bounded(zoom.start, 0.0, duration, "Zoom start")?;
            bounded(zoom.end, 0.0, duration + 0.05, "Zoom end")?;
            if zoom.end - zoom.start + TIME_EPSILON < 0.1 || zoom.start < previous_end - 0.001 {
                return Err(
                    "Zoom segments must not overlap and must last at least 0.1 seconds.".into(),
                );
            }
            bounded(zoom.scale, 1.0, 3.0, "Zoom scale")?;
            bounded(zoom.x, 0.0, 1.0, "Zoom X")?;
            bounded(zoom.y, 0.0, 1.0, "Zoom Y")?;
            previous_end = zoom.end;
        }
        self.validate_video_clips(duration)?;
        self.validate_editor_tracks(duration)
    }
}
fn fit_mode_default() -> String {
    "fit".into()
}

fn show_clicks_default() -> bool {
    true
}

fn background_mode_default() -> String {
    "solid".into()
}

fn background_blur_default() -> f64 {
    24.0
}

fn bounded(value: f64, min: f64, max: f64, name: &str) -> Result<(), String> {
    if !value.is_finite() || value < min || value > max {
        Err(format!("{name} must be between {min} and {max}."))
    } else {
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub directory: String,
    pub screen_path: String,
    pub microphone_path: Option<String>,
    pub system_audio_path: Option<String>,
    pub camera_path: Option<String>,
    #[serde(default)]
    pub camera_offset_seconds: f64,
    #[serde(default)]
    pub camera_intervals: Option<Vec<CameraInterval>>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration: f64,
    #[serde(default, deserialize_with = "storage::deserialize_cursor")]
    pub cursor: Vec<CursorSample>,
    pub edits: EditSettings,
    pub status: String,
}
impl Project {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != PROJECT_VERSION {
            return Err(format!("Unsupported project version {}.", self.version));
        }
        validate_id(&self.id)?;
        validate_name(&self.name)?;
        if self.width < 2 || self.height < 2 || self.width > 16384 || self.height > 16384 {
            return Err("Invalid video dimensions.".into());
        }
        if ![30, 60].contains(&self.fps) {
            return Err("Unsupported project frame rate.".into());
        }
        if !["ready", "interrupted"].contains(&self.status.as_str()) {
            return Err("Invalid project status.".into());
        }
        self.edits.validate(self.duration)?;
        bounded(self.camera_offset_seconds, 0.0, 3600.0, "Camera offset")?;
        self.validate_camera_intervals()?;
        if self.cursor.len() > storage::MAX_CURSOR_SAMPLES {
            return Err("Cursor data exceeds the 16 million sample limit.".into());
        }
        let mut previous = 0.0;
        for sample in &self.cursor {
            if sample.buttons > 7 || (sample.buttons != 0 && !sample.pressed) {
                return Err("Invalid cursor button state.".into());
            }
            bounded(sample.time, 0.0, self.duration + 2.0, "Cursor time")?;
            if sample.time < previous {
                return Err("Cursor samples must be chronological.".into());
            }
            bounded(sample.x, 0.0, 1.0, "Cursor X")?;
            bounded(sample.y, 0.0, 1.0, "Cursor Y")?;
            previous = sample.time;
        }
        Ok(())
    }
    pub fn summary(&self) -> ProjectSummary {
        ProjectSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            created_at: self.created_at.clone(),
            duration: edited_duration(&self.edits, self.duration),
            directory: self.directory.clone(),
            status: self.status.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub duration: f64,
    pub directory: String,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedEdits {
    pub name: String,
    pub edits: EditSettings,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingState {
    pub active: bool,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub camera_available: bool,
    #[serde(default = "idle_phase")]
    pub phase: String,
    pub elapsed: f64,
    pub project_id: Option<String>,
}
fn idle_phase() -> String {
    "idle".into()
}
impl Default for RecordingState {
    fn default() -> Self {
        Self {
            active: false,
            paused: false,
            camera_available: false,
            phase: idle_phase(),
            elapsed: 0.0,
            project_id: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub output_path: String,
    pub resolution: String,
    pub fps: u32,
    #[serde(default)]
    pub preset: ExportPreset,
}
impl ExportOptions {
    pub fn validate(&self) -> Result<(), String> {
        let quality_supported = match self.preset {
            ExportPreset::Standard => {
                ["1080p", "1440p", "2160p"].contains(&self.resolution.as_str())
                    && [30, 60].contains(&self.fps)
            }
            ExportPreset::Whatsapp => self.resolution == "auto" && self.fps == 30,
        };
        if !quality_supported {
            return Err("Unsupported export quality.".into());
        }
        let path = Path::new(&self.output_path);
        if !path.is_absolute()
            || path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| !s.eq_ignore_ascii_case("mp4"))
                .unwrap_or(true)
        {
            return Err("Choose an absolute MP4 output path.".into());
        }
        if path.exists() {
            return Err("This output file already exists. Choose a new filename.".into());
        }
        if !path.parent().is_some_and(Path::is_dir) {
            return Err("The output folder does not exist.".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub state: String,
    pub progress: f64,
    pub output_path: String,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub projects_directory: String,
    pub ffmpeg_available: bool,
    pub ffmpeg_path: Option<String>,
}

pub fn validate_id(id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid project identifier.".to_string())?;
    if id.len() != 36 {
        return Err("Invalid project identifier.".into());
    }
    Ok(())
}
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err("Project name must contain 1 to 120 visible characters.".into());
    }
    Ok(())
}
pub fn project_directory(root: &Path, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    Ok(root.join(id))
}

pub fn validated_recovery_directory(root: &Path, id: &str) -> Result<PathBuf, String> {
    recovery_directory_with_limit(root, id, 100_000)
}

pub fn validated_deletion_directory(root: &Path, id: &str) -> Result<PathBuf, String> {
    let directory = validated_recovery_directory(root, id)?;
    let library = dunce::canonicalize(root).map_err(|e| e.to_string())?;
    if directory.parent() != Some(library.as_path()) {
        return Err("Only a project directly inside the recording library can be deleted.".into());
    }
    load_project_summary(root, id)?;
    Ok(directory)
}

fn recovery_directory_with_limit(
    root: &Path,
    id: &str,
    max_entries: usize,
) -> Result<PathBuf, String> {
    let requested = project_directory(root, id)?;
    let metadata = fs::symlink_metadata(&requested).map_err(|error| {
        format!(
            "Cannot inspect recovery folder {}: {error}",
            requested.display()
        )
    })?;
    reject_recovery_link(&requested, &metadata)?;
    if !metadata.is_dir() {
        return Err("The recovery project must be a directory.".into());
    }
    let library = dunce::canonicalize(root)
        .map_err(|error| format!("Cannot resolve the recording library: {error}"))?;
    let directory = dunce::canonicalize(&requested)
        .map_err(|error| format!("Cannot resolve the recovery folder: {error}"))?;
    if directory == library || !directory.starts_with(&library) {
        return Err("Recovery folder is outside the recording library.".into());
    }
    let marker = directory.join("pending.json");
    let mut directories = vec![(directory.clone(), 0usize)];
    let mut count = 0usize;
    while let Some((parent, depth)) = directories.pop() {
        let entries = fs::read_dir(&parent).map_err(|error| {
            format!(
                "Cannot inspect recovery directory {}: {error}",
                parent.display()
            )
        })?;
        for entry in entries {
            let entry = entry
                .map_err(|error| format!("Cannot inspect a recovery directory entry: {error}"))?;
            count += 1;
            if count > max_entries {
                return Err(format!(
                    "Recovery directory exceeds the {max_entries}-entry inspection limit."
                ));
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                format!("Cannot inspect recovery entry {}: {error}", path.display())
            })?;
            reject_recovery_link(&path, &metadata)?;
            if path == marker && (!metadata.is_file() || metadata.len() > MAX_PENDING_PROJECT_BYTES)
            {
                return Err("Recovery marker must be a regular file no larger than 64 KiB.".into());
            }
            if metadata.is_dir() {
                if depth >= 128 {
                    return Err(
                        "Recovery directory nesting exceeds the 128-level inspection limit.".into(),
                    );
                }
                let resolved = dunce::canonicalize(&path).map_err(|error| {
                    format!(
                        "Cannot resolve recovery directory {}: {error}",
                        path.display()
                    )
                })?;
                if !resolved.starts_with(&directory) {
                    return Err("Recovery entry is outside its project folder.".into());
                }
                directories.push((resolved, depth + 1));
            } else if !metadata.is_file() {
                return Err(format!(
                    "Recovery entry is not a regular file or directory: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(directory)
}

fn reject_recovery_link(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    let is_link = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let is_link = {
        use std::os::windows::fs::MetadataExt;
        is_link || metadata.file_attributes() & 0x400 != 0
    };
    if is_link {
        return Err(format!(
            "Recovery does not follow symbolic links, junctions, or reparse points: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_capture_devices_have_an_empty_apps_list() {
        let devices: CaptureDevices = serde_json::from_value(serde_json::json!({
            "sources": [{ "id": "window:32", "title": "Existing window", "kind": "window", "width": 800, "height": 600 }],
            "microphones": [], "cameras": []
        })).unwrap();
        assert!(devices.apps.is_empty());
        assert_eq!(devices.sources[0].id, "window:32");
    }

    #[test]
    fn apps_wire_contract_preserves_process_bound_windows_and_unicode_names() {
        let value = serde_json::json!({
            "sources": [], "microphones": [], "cameras": [],
            "apps": [{ "id": "process:42", "name": "Editor 日本語.exe", "windows": [
                { "id": "app:42:32", "title": "First", "kind": "window", "width": 800, "height": 600 },
                { "id": "app:42:64", "title": "Second", "kind": "window", "width": 1280, "height": 720 }
            ] }]
        });
        let devices: CaptureDevices = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(devices.apps[0].windows[1].id, "app:42:64");
        assert_eq!(serde_json::to_value(devices).unwrap(), value);
    }
    #[test]
    fn rejects_nonfinite_and_invalid_trim() {
        let mut e = EditSettings::new(10.0);
        assert!(e.validate(10.0).is_ok());
        e.cursor_size = f64::NAN;
        assert!(e.validate(10.0).is_err());
        e.cursor_size = 1.0;
        e.trim_start = 11.0;
        assert!(e.validate(10.0).is_err());
    }
    #[test]
    fn hundred_millisecond_ranges_survive_binary_rounding() {
        for end in [12.0, 3600.0, 28800.0] {
            let mut edits = EditSettings::new(end);
            edits.trim_start = end - 0.1;
            edits.zooms = vec![ZoomSegment {
                id: "short".into(),
                start: end - 0.1,
                end,
                scale: 1.6,
                x: 0.5,
                y: 0.5,
                follow_cursor: false,
            }];
            assert!(edits.validate(end).is_ok());
            edits.trim_start = end - 0.099;
            assert!(edits.validate(end).is_err());
        }
    }
    #[test]
    fn prevents_traversal_and_crop_escape() {
        assert!(project_directory(Path::new("library"), "../secret").is_err());
        let mut e = EditSettings::new(10.0);
        e.crop = Some(CropSettings {
            x: 0.8,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        });
        assert!(e.validate(10.0).is_err());
    }
    #[test]
    fn refuses_overlapping_zoom_and_injected_color() {
        let mut e = EditSettings::new(10.0);
        e.background = "#000000;movie=secret".into();
        assert!(e.validate(10.0).is_err());
        e.background = "#000000".into();
        e.zooms = vec![
            ZoomSegment {
                id: "a".into(),
                start: 1.0,
                end: 4.0,
                scale: 2.0,
                x: 0.5,
                y: 0.5,
                follow_cursor: true,
            },
            ZoomSegment {
                id: "b".into(),
                start: 3.0,
                end: 6.0,
                scale: 2.0,
                x: 0.5,
                y: 0.5,
                follow_cursor: true,
            },
        ];
        assert!(e.validate(10.0).is_err());
    }
    #[test]
    fn output_never_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("export.mp4");
        fs::write(&p, b"keep").unwrap();
        assert!(ExportOptions {
            output_path: p.to_string_lossy().into_owned(),
            resolution: "1080p".into(),
            fps: 30,
            preset: ExportPreset::Standard,
        }
        .validate()
        .is_err());
        assert_eq!(fs::read(p).unwrap(), b"keep");
    }
    pub(super) fn fixture(root: &Path) -> Project {
        let id = uuid::Uuid::new_v4().to_string();
        let directory = root.join(&id);
        fs::create_dir(&directory).unwrap();
        let screen = directory.join("screen.mp4");
        fs::write(&screen, b"media-path-fixture").unwrap();
        Project {
            version: PROJECT_VERSION,
            id,
            name: "Test recording".into(),
            created_at: "2026-11-11T15:00:00+07:00".into(),
            directory: directory.to_string_lossy().into_owned(),
            screen_path: screen.to_string_lossy().into_owned(),
            microphone_path: None,
            system_audio_path: None,
            camera_path: None,
            camera_offset_seconds: 0.0,
            camera_intervals: None,
            width: 1920,
            height: 1080,
            fps: 30,
            duration: 3.0,
            cursor: vec![],
            edits: EditSettings::new(3.0),
            status: "ready".into(),
        }
    }
    #[test]
    fn atomic_save_round_trip_and_invalid_write_preserves_project() {
        let root = tempfile::tempdir().unwrap();
        let mut project = fixture(root.path());
        save_project_atomic(&project).unwrap();
        project.name = "Renamed".into();
        save_project_atomic(&project).unwrap();
        assert_eq!(
            load_project(root.path(), &project.id).unwrap().name,
            "Renamed"
        );
        project.edits.trim_end = 50.0;
        assert!(save_project_atomic(&project).is_err());
        assert_eq!(
            load_project(root.path(), &project.id)
                .unwrap()
                .edits
                .trim_end,
            3.0
        );
    }
    #[test]
    fn saved_media_cannot_escape_its_project() {
        let root = tempfile::tempdir().unwrap();
        let mut project = fixture(root.path());
        let outside = root.path().join("outside.mp4");
        fs::write(&outside, b"outside").unwrap();
        project.screen_path = outside.to_string_lossy().into_owned();
        assert!(save_project_atomic(&project)
            .unwrap_err()
            .contains("project folder"));
    }
    #[test]
    fn rejects_future_versions_and_nonchronological_cursor() {
        let root = tempfile::tempdir().unwrap();
        let mut project = fixture(root.path());
        project.version = PROJECT_VERSION + 1;
        assert!(project.validate().is_err());
        project.version = PROJECT_VERSION;
        project.cursor = vec![
            CursorSample {
                time: 2.0,
                x: 0.5,
                y: 0.5,
                pressed: false,
                buttons: 0,
                visible: true,
            },
            CursorSample {
                time: 1.0,
                x: 0.5,
                y: 0.5,
                pressed: false,
                buttons: 0,
                visible: true,
            },
        ];
        assert!(project.validate().is_err());
    }

    #[test]
    fn recovery_accepts_regular_project_tree_and_bounds_marker() {
        let root = tempfile::tempdir().unwrap();
        let project = fixture(root.path());
        let directory = Path::new(&project.directory);
        fs::create_dir(directory.join("segments")).unwrap();
        fs::write(directory.join("segments/screen-00000.mp4"), b"segment").unwrap();
        let marker = directory.join("pending.json");
        fs::write(&marker, b"{}").unwrap();
        assert_eq!(
            validated_recovery_directory(root.path(), &project.id).unwrap(),
            dunce::canonicalize(directory).unwrap()
        );
        fs::OpenOptions::new()
            .write(true)
            .open(marker)
            .unwrap()
            .set_len(MAX_PENDING_PROJECT_BYTES + 1)
            .unwrap();
        assert!(validated_recovery_directory(root.path(), &project.id)
            .unwrap_err()
            .contains("64 KiB"));
    }

    #[test]
    fn recovery_rejects_invalid_identity_missing_folder_and_traversal_budget() {
        let root = tempfile::tempdir().unwrap();
        assert!(validated_recovery_directory(root.path(), "../outside").is_err());
        assert!(
            validated_recovery_directory(root.path(), &uuid::Uuid::new_v4().to_string()).is_err()
        );
        let project = fixture(root.path());
        fs::write(Path::new(&project.directory).join("second-file"), b"second").unwrap();
        assert!(recovery_directory_with_limit(root.path(), &project.id, 1)
            .unwrap_err()
            .contains("inspection limit"));
    }

    #[cfg(any(windows, unix))]
    fn create_directory_link(target: &Path, link: &Path) -> bool {
        #[cfg(windows)]
        let result = std::os::windows::fs::symlink_dir(target, link);
        #[cfg(unix)]
        let result = std::os::unix::fs::symlink(target, link);
        match result {
            Ok(()) => true,
            Err(error)
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    || error.raw_os_error() == Some(1314) =>
            {
                eprintln!("SKIPPED: creating directory symlinks is not permitted: {error}");
                false
            }
            Err(error) => panic!("Cannot create test directory link: {error}"),
        }
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn recovery_rejects_project_link_outside_library() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        if !create_directory_link(outside.path(), &root.path().join(&id)) {
            return;
        }
        assert!(validated_recovery_directory(root.path(), &id)
            .unwrap_err()
            .contains("does not follow"));
        assert!(validated_deletion_directory(root.path(), &id)
            .unwrap_err()
            .contains("does not follow"));
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn recovery_rejects_nested_link_even_when_target_is_inside_project() {
        let root = tempfile::tempdir().unwrap();
        let project = fixture(root.path());
        let directory = Path::new(&project.directory);
        let target = directory.join("segments");
        fs::create_dir(&target).unwrap();
        if !create_directory_link(&target, &directory.join("aliased-segments")) {
            return;
        }
        assert!(validated_recovery_directory(root.path(), &project.id)
            .unwrap_err()
            .contains("does not follow"));
        assert!(validated_deletion_directory(root.path(), &project.id)
            .unwrap_err()
            .contains("does not follow"));
    }

    #[cfg(windows)]
    #[test]
    fn recovery_rejects_windows_junction_outside_library() {
        use std::os::windows::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let junction = root.path().join(&id);
        let powershell =
            PathBuf::from(std::env::var_os("SystemRoot").expect("Windows system directory"))
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let output = std::process::Command::new(powershell)
            .creation_flags(0x08000000)
            .env("SCREENRECORD_TEST_JUNCTION", &junction)
            .env("SCREENRECORD_TEST_TARGET", outside.path())
            .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:SCREENRECORD_TEST_JUNCTION -Target $env:SCREENRECORD_TEST_TARGET | Out-Null"])
            .output().expect("Create test junction");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(validated_recovery_directory(root.path(), &id)
            .unwrap_err()
            .contains("does not follow"));
        assert!(outside.path().is_dir());
        assert!(validated_deletion_directory(root.path(), &id)
            .unwrap_err()
            .contains("does not follow"));
    }
}
