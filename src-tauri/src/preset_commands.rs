use super::*;
use std::{collections::HashSet, io::Read};

const MAX_PRESETS: usize = 50;
const MAX_BYTES: u64 = 128 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PresetFile {
    version: u32,
    presets: Vec<AppearancePreset>,
}

fn preset_id(id: &str) -> Result<(), String> {
    validate_id(id).map_err(|_| "Invalid appearance preset identifier.".into())
}
fn validate_presets(presets: &[AppearancePreset]) -> Result<(), String> {
    if presets.len() > MAX_PRESETS {
        return Err("Keep at most 50 custom appearance presets.".into());
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for preset in presets {
        preset_id(&preset.id)?;
        validate_name(&preset.name)?;
        preset.settings.validate()?;
        if preset.name != preset.name.trim()
            || !ids.insert(&preset.id)
            || !names.insert(preset.name.to_lowercase())
        {
            return Err("Appearance presets require unique names and identifiers.".into());
        }
    }
    Ok(())
}

fn load(path: &Path) -> Result<Vec<AppearancePreset>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(format!("Cannot inspect custom presets: {error}")),
    };
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || reparse
        || metadata.len() > MAX_BYTES
    {
        return Err(
            "The custom preset file is unavailable or exceeds 128 KiB. It has not been changed."
                .into(),
        );
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("The custom preset file exceeds 128 KiB. It has not been changed.".into());
    }
    let document: PresetFile = serde_json::from_slice(&bytes).map_err(|_| {
        "Custom presets could not be read. The existing file has not been changed.".to_string()
    })?;
    if document.version != 1 {
        return Err(
            "This custom preset format is not supported. The existing file has not been changed."
                .into(),
        );
    }
    validate_presets(&document.presets)?;
    Ok(document.presets)
}

fn write(path: &Path, presets: Vec<AppearancePreset>) -> Result<(), String> {
    validate_presets(&presets)?;
    let bytes = serde_json::to_vec_pretty(&PresetFile {
        version: 1,
        presets,
    })
    .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Custom presets exceed the 128 KiB limit.".into());
    }
    let parent = path
        .parent()
        .ok_or("Custom preset directory is unavailable.")?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Cannot create the custom preset directory: {e}"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary.write_all(&bytes).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary
        .persist(path)
        .map_err(|e| format!("Cannot save custom presets: {e}"))?;
    Ok(())
}

fn save(
    path: &Path,
    id: Option<&str>,
    name: &str,
    settings: AppearanceSettings,
) -> Result<AppearancePreset, String> {
    let name = name.trim().to_owned();
    validate_name(&name)?;
    settings.validate()?;
    if let Some(id) = id {
        preset_id(id)?;
    }
    let mut presets = load(path)?;
    let preset = AppearancePreset {
        id: id
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name,
        settings,
    };
    match id {
        Some(id) => {
            let existing = presets
                .iter_mut()
                .find(|preset| preset.id == id)
                .ok_or("This preset no longer exists. Reload the preset list.")?;
            *existing = preset.clone();
        }
        None => presets.push(preset.clone()),
    }
    write(path, presets)?;
    Ok(preset)
}

fn delete(path: &Path, id: &str) -> Result<(), String> {
    preset_id(id)?;
    let mut presets = load(path)?;
    let before = presets.len();
    presets.retain(|preset| preset.id != id);
    if presets.len() == before {
        return Err("This preset no longer exists. Reload the preset list.".into());
    }
    write(path, presets)
}

#[tauri::command]
pub(super) async fn list_appearance_presets(
    app: tauri::AppHandle,
) -> Result<Vec<AppearancePreset>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _write = state.preset_write.lock().map_err(|e| e.to_string())?;
        load(&state.settings_root.join("appearance-presets.json"))
    })
    .await
    .map_err(|_| "Custom preset reader failed.".to_string())?
}

#[tauri::command]
pub(super) async fn save_appearance_preset(
    app: tauri::AppHandle,
    id: Option<String>,
    name: String,
    settings: AppearanceSettings,
) -> Result<AppearancePreset, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _write = state.preset_write.lock().map_err(|e| e.to_string())?;
        save(
            &state.settings_root.join("appearance-presets.json"),
            id.as_deref(),
            &name,
            settings,
        )
    })
    .await
    .map_err(|_| "Custom preset writer failed. Existing presets are unchanged.".to_string())?
}

#[tauri::command]
pub(super) async fn delete_appearance_preset(
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _write = state.preset_write.lock().map_err(|e| e.to_string())?;
        delete(&state.settings_root.join("appearance-presets.json"), &id)
    })
    .await
    .map_err(|_| "Custom preset removal failed. Existing presets are unchanged.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> AppearanceSettings {
        AppearanceSettings::from_edits(&EditSettings::new(1.0))
    }
    #[test]
    fn custom_presets_roundtrip_update_and_delete_without_touching_other_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings").join("appearance-presets.json");
        assert!(load(&path).unwrap().is_empty());
        let first = save(&path, None, " My look ", settings()).unwrap();
        assert_eq!(first.name, "My look");
        let mut look = settings();
        look.camera.layout = "split".into();
        let updated = save(&path, Some(&first.id), "Presentation", look).unwrap();
        assert_eq!(updated.id, first.id);
        assert_eq!(load(&path).unwrap()[0].settings.camera.layout, "split");
        let untouched = root.path().join("other.json");
        fs::write(&untouched, b"other settings").unwrap();
        delete(&path, &first.id).unwrap();
        assert!(load(&path).unwrap().is_empty());
        assert_eq!(fs::read(&untouched).unwrap(), b"other settings");
    }
    #[test]
    fn duplicate_names_limits_invalid_settings_and_unknown_updates_preserve_the_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("appearance-presets.json");
        let original = save(&path, None, "Look", settings()).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(save(&path, None, "look", settings()).is_err());
        assert!(save(
            &path,
            Some(&uuid::Uuid::new_v4().to_string()),
            "Other",
            settings()
        )
        .is_err());
        let mut invalid = settings();
        invalid.background = "invalid".into();
        assert!(save(&path, Some(&original.id), "Look", invalid).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        for index in 1..MAX_PRESETS {
            save(&path, None, &format!("Look {index}"), settings()).unwrap();
        }
        let full = fs::read(&path).unwrap();
        assert!(save(&path, None, "Overflow", settings()).is_err());
        assert_eq!(fs::read(&path).unwrap(), full);
    }
    #[test]
    fn corrupt_future_oversized_and_reparse_files_are_not_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("appearance-presets.json");
        for bytes in [
            b"broken json".to_vec(),
            br#"{"version":2,"presets":[]}"#.to_vec(),
            vec![b' '; MAX_BYTES as usize + 1],
        ] {
            fs::write(&path, &bytes).unwrap();
            assert!(save(&path, None, "New", settings()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(load(&path).is_err());
        let target = root.path().join("owned-target.json");
        fs::write(&target, b"target preserved").unwrap();
        #[cfg(windows)]
        {
            let link = root.path().join("linked-presets.json");
            if std::os::windows::fs::symlink_file(&target, &link).is_ok() {
                assert!(save(&link, None, "New", settings()).is_err());
                assert_eq!(fs::read(&target).unwrap(), b"target preserved");
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn failed_atomic_replacement_preserves_read_only_presets_and_removes_temporary_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("appearance-presets.json");
        let preset = save(&path, None, "Original", settings()).unwrap();
        let before = fs::read(&path).unwrap();
        let original = fs::metadata(&path).unwrap().permissions();
        let mut read_only = original.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&path, read_only).unwrap();
        let saving = save(&path, Some(&preset.id), "Changed", settings());
        let deleting = delete(&path, &preset.id);
        fs::set_permissions(&path, original).unwrap();
        assert!(saving.is_err());
        assert!(deleting.is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
