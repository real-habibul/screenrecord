use screenrecord_core::validated_recovery_directory;
use std::{
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn write(
    root: &Path,
    id: &str,
    phase: &str,
    error: &str,
    recovery_error: Option<&str>,
) -> Result<(), String> {
    let directory = validated_recovery_directory(root, id)?;
    let bounded = |value: &str| value.chars().take(4096).collect::<String>();
    let value = serde_json::json!({
        "schemaVersion": 1,
        "appVersion": env!("CARGO_PKG_VERSION"),
        "recordingId": id,
        "timestampUnixMs": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
        "phase": phase.chars().take(64).collect::<String>(),
        "error": bounded(error),
        "recoveryError": recovery_error.map(bounded),
    });
    let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, &value).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(directory.join("recording-failure.json"))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn failure_context_keeps_version_identity_unicode_bounds_and_media() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let directory = root.path().join(&id);
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("pending.json"), b"keep marker").unwrap();
        fs::write(directory.join("screen.mp4"), b"keep media").unwrap();
        write(
            root.path(),
            &id,
            "stop",
            &"é".repeat(5000),
            Some("recovery unavailable"),
        )
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("recording-failure.json")).unwrap())
                .unwrap();
        assert_eq!(value["appVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["recordingId"], id);
        assert_eq!(value["error"].as_str().unwrap().chars().count(), 4096);
        assert_eq!(value["recoveryError"], "recovery unavailable");
        assert_eq!(
            fs::read(directory.join("screen.mp4")).unwrap(),
            b"keep media"
        );
        assert_eq!(
            fs::read(directory.join("pending.json")).unwrap(),
            b"keep marker"
        );
        assert!(write(root.path(), "../outside", "stop", "error", None).is_err());
    }

    #[test]
    fn failed_diagnostic_replacement_preserves_media_and_existing_evidence() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let directory = root.path().join(&id);
        fs::create_dir(&directory).unwrap();
        fs::create_dir(directory.join("recording-failure.json")).unwrap();
        fs::write(
            directory.join("recording-failure.json/original"),
            b"keep evidence",
        )
        .unwrap();
        fs::write(directory.join("screen.mp4"), b"keep media").unwrap();
        assert!(write(root.path(), &id, "start", "error", None).is_err());
        assert_eq!(
            fs::read(directory.join("recording-failure.json/original")).unwrap(),
            b"keep evidence"
        );
        assert_eq!(
            fs::read(directory.join("screen.mp4")).unwrap(),
            b"keep media"
        );
        assert_eq!(fs::read_dir(directory).unwrap().count(), 2);
    }
}
