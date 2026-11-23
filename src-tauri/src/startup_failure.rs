use screenrecord_core::validated_recovery_directory;
use std::{io::Write, path::Path};

pub fn retire(root: &Path, id: &str, reason: &str) -> Result<bool, String> {
    retire_with(root, id, reason, move_marker)
}

fn retire_with(
    root: &Path,
    id: &str,
    reason: &str,
    move_file: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<bool, String> {
    let directory = validated_recovery_directory(root, id)?;
    if directory
        .join("project.json")
        .try_exists()
        .map_err(|e| e.to_string())?
        || !screenrecord_capture::never_started_screen(&directory)?
    {
        return Ok(false);
    }
    let marker = directory.join("pending.json");
    if !marker.is_file() {
        return Err("The failed-start marker is unavailable.".into());
    }
    let archive = directory.join("failed-start.json");
    if archive.try_exists().map_err(|e| e.to_string())? {
        return Err("A failed-start marker already exists. Recording files were preserved.".into());
    }
    let diagnostic = directory.join("startup-failure.json");
    if !diagnostic.try_exists().map_err(|e| e.to_string())? {
        let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(
            &mut file,
            &serde_json::json!({
                "version": 1,
                "appVersion": env!("CARGO_PKG_VERSION"),
                "recordingId": id,
                "reason": reason.chars().take(4096).collect::<String>(),
                "screenStarted": false,
                "recordingFilesPreserved": true,
            }),
        )
        .map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist_noclobber(diagnostic)
            .map_err(|e| e.to_string())?;
    }
    // A no-replace move keeps the original marker intact if archival fails.
    move_file(&marker, &archive)?;
    Ok(true)
}

fn move_marker(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{core::PCWSTR, Win32::Storage::FileSystem::MoveFileW};
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe { MoveFileW(PCWSTR(source.as_ptr()), PCWSTR(destination.as_ptr())) }.map_err(|e| {
        format!("Cannot archive the failed-start marker. Recording files were preserved: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(root: &Path) -> (String, std::path::PathBuf) {
        let id = uuid::Uuid::new_v4().to_string();
        let directory = root.join(&id);
        fs::create_dir_all(directory.join("part-00000/segments")).unwrap();
        fs::write(
            directory.join("pending.json"),
            b"{\"preserve\":\"exact bytes\"}",
        )
        .unwrap();
        fs::write(directory.join("session-journal.json"), br#"{"version":1,"dimensions":null,"parts":[{"directory":"part-00000","duration":null}]}"#).unwrap();
        fs::write(directory.join("part-00000/capture-journal.json"), br#"{"version":1,"sourceId":"monitor:123","width":1920,"height":1080,"fps":30,"epochQpc":0,"cameraEpochQpc":null,"cameraOffsetSeconds":null,"duration":0.0,"microphone":false,"systemAudio":true,"segments":[],"segmentDurations":[]}"#).unwrap();
        (id, directory)
    }

    #[test]
    fn failed_screen_archives_marker_and_preserves_camera_audio_and_journals() {
        let root = tempfile::tempdir().unwrap();
        let (id, directory) = fixture(root.path());
        fs::write(
            directory.join("part-00000/camera.raw.mp4"),
            b"camera fixture",
        )
        .unwrap();
        fs::write(directory.join("part-00000/system.wav"), b"audio fixture").unwrap();
        let marker = fs::read(directory.join("pending.json")).unwrap();
        assert!(retire(root.path(), &id, "No screen frame").unwrap());
        assert!(!directory.join("pending.json").exists());
        assert_eq!(
            fs::read(directory.join("failed-start.json")).unwrap(),
            marker
        );
        assert_eq!(
            fs::read(directory.join("part-00000/camera.raw.mp4")).unwrap(),
            b"camera fixture"
        );
        assert_eq!(
            fs::read(directory.join("part-00000/system.wav")).unwrap(),
            b"audio fixture"
        );
        assert!(directory.join("session-journal.json").is_file());
        assert!(!directory.join("project.json").exists());
    }

    #[test]
    fn any_screen_segment_or_completed_project_keeps_recovery_marker() {
        for file in [
            "part-00000/segments/segment-00000.mp4",
            "screen.mp4",
            "project.json",
        ] {
            let root = tempfile::tempdir().unwrap();
            let (id, directory) = fixture(root.path());
            fs::write(directory.join(file), b"").unwrap();
            assert!(!retire(root.path(), &id, "test").unwrap());
            assert!(directory.join("pending.json").is_file());
            assert!(!directory.join("failed-start.json").exists());
        }
    }

    #[test]
    fn unknown_or_missing_journal_keeps_recovery_marker() {
        for contents in [None, Some("{bad"), Some("{\"version\":99}")] {
            let root = tempfile::tempdir().unwrap();
            let (id, directory) = fixture(root.path());
            let journal = directory.join("part-00000/capture-journal.json");
            if let Some(contents) = contents {
                fs::write(journal, contents).unwrap();
            } else {
                fs::remove_file(journal).unwrap();
            }
            assert!(retire(root.path(), &id, "test").is_err());
            assert!(directory.join("pending.json").is_file());
        }
    }

    #[test]
    fn archive_conflict_and_failed_move_preserve_original_marker() {
        let root = tempfile::tempdir().unwrap();
        let (id, directory) = fixture(root.path());
        let before = fs::read(directory.join("pending.json")).unwrap();
        assert!(retire_with(root.path(), &id, "test", |_, _| Err(
            "injected access denied".into()
        ))
        .is_err());
        assert_eq!(fs::read(directory.join("pending.json")).unwrap(), before);
        fs::write(directory.join("failed-start.json"), b"existing archive").unwrap();
        assert!(retire(root.path(), &id, "test").is_err());
        assert_eq!(
            fs::read(directory.join("failed-start.json")).unwrap(),
            b"existing archive"
        );
        assert_eq!(fs::read(directory.join("pending.json")).unwrap(), before);
    }

    #[test]
    fn native_move_refuses_existing_destination() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::write(&source, b"source").unwrap();
        fs::write(&destination, b"destination").unwrap();
        assert!(move_marker(&source, &destination).is_err());
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(destination).unwrap(), b"destination");
    }
}
