use screenrecord_core::{ExportOptions, Project};
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        return Err(
            "Usage: render_project <project.json> <output.mp4> <ffmpeg-path> [1080p|1440p|2160p]"
                .into(),
        );
    }
    let project = read_project(Path::new(&args[1]))?;
    let options = ExportOptions {
        preset: screenrecord_core::ExportPreset::Standard,
        output_path: args[2].clone(),
        resolution: args.get(4).cloned().unwrap_or("1080p".into()),
        fps: project.fps,
    };
    screenrecord_render::export(
        &project,
        &options,
        Path::new(&args[3]),
        Arc::new(AtomicBool::new(false)),
        |progress| {
            eprintln!(
                "{} {:.1}% {}",
                progress.state,
                progress.progress * 100.0,
                progress.error.as_deref().unwrap_or("")
            );
        },
    )
}

fn read_project(path: &Path) -> Result<Project, String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    if path.file_name().and_then(|name| name.to_str()) != Some("project.json") {
        return Err("Choose the saved project's project.json file.".into());
    }
    let directory = path.parent().ok_or("Project directory is missing.")?;
    let root = directory.parent().ok_or("Project library is missing.")?;
    let id = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid project directory.")?;
    screenrecord_core::load_project(root, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use screenrecord_core::{CursorSample, EditSettings};

    fn fixture(root: &Path) -> Project {
        let id = "42751a57-9044-4493-84c3-43dd70402962";
        let directory = root.join(id);
        std::fs::create_dir(&directory).unwrap();
        let media = directory.join("screen.mp4");
        std::fs::write(&media, b"path-validation-fixture").unwrap();
        Project {
            version: screenrecord_core::PROJECT_VERSION,
            id: id.into(),
            name: "Example loader".into(),
            created_at: "2026-11-11T00:00:00Z".into(),
            directory: directory.to_string_lossy().into_owned(),
            screen_path: media.to_string_lossy().into_owned(),
            microphone_path: None,
            system_audio_path: None,
            camera_path: None,
            camera_offset_seconds: 0.0,
            camera_intervals: None,
            width: 320,
            height: 180,
            fps: 30,
            duration: 2.0,
            cursor: vec![CursorSample {
                buttons: 0,
                time: 0.5,
                x: 0.25,
                y: 0.75,
                pressed: true,
                visible: true,
            }],
            edits: EditSettings::new(2.0),
            status: "ready".into(),
        }
    }

    #[test]
    fn example_loads_detached_cursor_and_legacy_inline_cursor() {
        let root = tempfile::tempdir().unwrap();
        let mut project = fixture(root.path());
        screenrecord_core::save_project_atomic(&project).unwrap();
        let path = Path::new(&project.directory).join("project.json");
        let loaded = read_project(&path).unwrap();
        assert_eq!(loaded.cursor.len(), 1);
        assert_eq!(loaded.cursor[0].x, 0.25);
        project.version = 1;
        std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
        let loaded = read_project(&path).unwrap();
        assert_eq!(loaded.version, screenrecord_core::PROJECT_VERSION);
        assert_eq!(loaded.cursor.len(), 1);
        assert!(loaded.cursor[0].pressed);
    }
}
