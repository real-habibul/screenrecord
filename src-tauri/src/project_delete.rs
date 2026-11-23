use screenrecord_core::validated_deletion_directory;
use std::path::Path;
use windows::{
    core::PCWSTR,
    Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        },
        UI::Shell::{
            FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName,
            FOFX_ADDUNDORECORD, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FOF_NOCONFIRMATION,
            FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
        },
    },
};

pub fn recycle_project(root: &Path, id: &str) -> Result<(), String> {
    recycle_validated(root, id, recycle_folder)
}

fn recycle_validated(
    root: &Path,
    id: &str,
    recycle: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let directory = validated_deletion_directory(root, id)?;
    recycle(&directory)?;
    if directory.try_exists().map_err(|e| e.to_string())? {
        return Err("Windows did not finish moving the project to the Recycle Bin. Close other apps using its files, then retry.".into());
    }
    Ok(())
}

fn recycle_folder(directory: &Path) -> Result<(), String> {
    let directory = directory.to_owned();
    std::thread::Builder::new().name("project-recycle".into()).spawn(move || {
        struct Apartment;
        impl Drop for Apartment { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
        let operation = || -> windows::core::Result<()> {
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()?;
                let _apartment = Apartment;
                let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
                operation.SetOperationFlags(FOFX_RECYCLEONDELETE | FOFX_ADDUNDORECORD | FOFX_EARLYFAILURE | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOF_WANTNUKEWARNING)?;
                use std::os::windows::ffi::OsStrExt;
                let path: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
                let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None)?;
                let (sink, result) = crate::recycle_sink::new_sink();
                operation.DeleteItem(&item, &sink)?;
                operation.PerformOperations()?;
                result.check()?;
                if operation.GetAnyOperationsAborted()?.as_bool() {
                    return Err(windows::core::Error::from(windows::Win32::Foundation::E_ABORT));
                }
            }
            Ok(())
        };
        operation().map_err(|error| format!("Windows could not move the project to the Recycle Bin ({}). Check that the drive supports recycling and close apps using these files, then retry.", error.code()))
    }).map_err(|_| "Cannot start the Recycle Bin operation.".to_string())?
        .join().map_err(|_| "The Recycle Bin operation stopped unexpectedly. Refresh projects and check the folder before retrying.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use screenrecord_core::{save_project_atomic, EditSettings, Project, PROJECT_VERSION};
    use std::{
        fs,
        sync::atomic::{AtomicBool, Ordering},
    };

    fn fixture(root: &Path) -> Project {
        let id = uuid::Uuid::new_v4().to_string();
        let directory = root.join(&id);
        fs::create_dir(&directory).unwrap();
        let screen = directory.join("screen.mp4");
        fs::write(&screen, b"owned test fixture").unwrap();
        let project = Project {
            version: PROJECT_VERSION,
            id,
            name: "Delete test".into(),
            created_at: "2026-11-11T00:00:00Z".into(),
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
            duration: 1.0,
            cursor: vec![],
            edits: EditSettings::new(1.0),
            status: "ready".into(),
        };
        save_project_atomic(&project).unwrap();
        project
    }

    #[test]
    fn validated_project_only_is_dispatched_and_external_export_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        let archive = tempfile::tempdir().unwrap();
        let project = fixture(root.path());
        let external = root.path().join("external-export.mp4");
        fs::write(&external, b"keep export").unwrap();
        recycle_validated(root.path(), &project.id, |directory| {
            assert_eq!(
                directory.parent(),
                Some(dunce::canonicalize(root.path()).unwrap().as_path())
            );
            fs::rename(directory, archive.path().join(&project.id)).map_err(|e| e.to_string())
        })
        .unwrap();
        assert!(!Path::new(&project.directory).exists());
        assert_eq!(fs::read(external).unwrap(), b"keep export");
        assert!(archive
            .path()
            .join(&project.id)
            .join("project.json")
            .is_file());
    }

    #[test]
    fn malformed_identity_and_failed_recycling_preserve_owned_media() {
        let root = tempfile::tempdir().unwrap();
        let project = fixture(root.path());
        let invoked = AtomicBool::new(false);
        for id in [
            "..",
            "../outside",
            "",
            "C:\\",
            &uuid::Uuid::new_v4().to_string(),
        ] {
            assert!(recycle_validated(root.path(), id, |_| {
                invoked.store(true, Ordering::Relaxed);
                Ok(())
            })
            .is_err());
        }
        assert!(!invoked.load(Ordering::Relaxed));
        assert!(recycle_validated(root.path(), &project.id, |_| Err(
            "Recycle Bin unavailable".into()
        ))
        .is_err());
        assert!(recycle_validated(root.path(), &project.id, |_| Ok(())).is_err());
        assert_eq!(
            fs::read(&project.screen_path).unwrap(),
            b"owned test fixture"
        );
        let manifest = Path::new(&project.directory).join("project.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value["id"] = uuid::Uuid::new_v4().to_string().into();
        fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(recycle_validated(root.path(), &project.id, |_| {
            invoked.store(true, Ordering::Relaxed);
            Ok(())
        })
        .is_err());
        assert!(!invoked.load(Ordering::Relaxed));
    }
}
