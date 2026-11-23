use std::{
    collections::VecDeque,
    fs,
    io::{self, Write},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

const OWNER_FILE: &str = ".screenrecord-trial";
const OWNER_BYTES: &[u8] = b"Screenrecord temporary recording v1\n";
pub const CANCELLED: &str = "Recording test cancelled.";

pub fn validate_trial_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id) {
        Ok(())
    } else {
        Err("Invalid recording test identifier.".into())
    }
}

fn check_root(root: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err("The recording test cache must be a local directory.".into());
    }
    Ok(())
}

pub struct TrialDirectory {
    root: PathBuf,
    id: String,
}

impl TrialDirectory {
    pub fn create(root: &Path, id: &str) -> Result<Self, String> {
        validate_trial_id(id)?;
        fs::create_dir_all(root)
            .map_err(|e| format!("Cannot create the recording test cache: {e}"))?;
        check_root(root)?;
        let path = root.join(id);
        fs::create_dir(&path)
            .map_err(|e| format!("Cannot create the recording test folder: {e}"))?;
        let mut marker = fs::File::create_new(path.join(OWNER_FILE)).map_err(|e| {
            let _ = fs::remove_dir(&path);
            format!("Cannot create the recording test marker: {e}")
        })?;
        if let Err(error) = marker
            .write_all(OWNER_BYTES)
            .and_then(|()| marker.sync_all())
        {
            drop(marker);
            let _ = fs::remove_file(path.join(OWNER_FILE));
            let _ = fs::remove_dir(&path);
            return Err(format!("Cannot prepare temporary test media: {error}"));
        }
        Ok(Self {
            root: root.to_owned(),
            id: id.into(),
        })
    }

    pub fn path(&self) -> PathBuf {
        self.root.join(&self.id)
    }

    fn remove(root: &Path, id: &str) -> Result<(), String> {
        validate_trial_id(id)?;
        check_root(root)?;
        let directory = screenrecord_core::validated_recovery_directory(root, id)?;
        let marker = directory.join(OWNER_FILE);
        let meta = fs::symlink_metadata(&marker).map_err(|e| e.to_string())?;
        if !meta.is_file()
            || meta.len() != OWNER_BYTES.len() as u64
            || fs::read(&marker).map_err(|e| e.to_string())? != OWNER_BYTES
        {
            return Err("Folder is not a Screenrecord test recording.".into());
        }
        // Keep the ownership marker until Windows releases every preview and native media handle.
        for child in fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            if child.file_name() == OWNER_FILE {
                continue;
            }
            let kind = child.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                fs::remove_dir_all(child.path())
            } else {
                fs::remove_file(child.path())
            }
            .map_err(|e| e.to_string())?;
        }
        fs::remove_file(marker)
            .and_then(|()| fs::remove_dir(directory))
            .map_err(|e| e.to_string())
    }

    pub fn clean_previous_launch(root: &Path) -> Result<(), String> {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        check_root(root)?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let id = entry.file_name();
            if let Some(id) = id.to_str().filter(|id| validate_trial_id(id).is_ok()) {
                if let Err(e) = Self::remove(root, id) {
                    log::warn!("Could not retire an old recording test: {e}");
                }
            }
        }
        Ok(())
    }
}

impl Drop for TrialDirectory {
    fn drop(&mut self) {
        if screenrecord_capture::cleanup_in_progress() {
            log::warn!("Native media handles are still active. The recording test will be retired at the next launch.");
            return;
        }
        if let Err(e) = Self::remove(&self.root, &self.id) {
            log::warn!("Recording test cleanup will be retried on the next launch: {e}");
        }
    }
}

#[derive(Default)]
pub struct TrialState {
    active: Option<(String, Arc<AtomicBool>)>,
    ready: Option<TrialDirectory>,
    retired: VecDeque<String>,
}

impl TrialState {
    pub fn reserve(&mut self, id: &str) -> Result<Arc<AtomicBool>, String> {
        if self.retired.iter().any(|old| old == id) {
            return Err(CANCELLED.into());
        }
        if self.active.is_some() || self.ready.as_ref().is_some_and(|old| old.id == id) {
            return Err("A recording test is already using this session.".into());
        }
        let token = Arc::new(AtomicBool::new(false));
        self.active = Some((id.to_owned(), token.clone()));
        self.ready.take();
        Ok(token)
    }

    pub fn publish(&mut self, media: TrialDirectory) -> Result<(), String> {
        if !self
            .active
            .as_ref()
            .is_some_and(|(id, cancel)| id == &media.id && !cancel.load(Ordering::Acquire))
        {
            return Err(CANCELLED.into());
        }
        self.ready = Some(media);
        Ok(())
    }

    pub fn finish(&mut self, id: &str) {
        if self
            .active
            .as_ref()
            .is_some_and(|(current, _)| current == id)
        {
            self.active = None;
        }
    }

    pub fn release(&mut self, id: &str) -> Option<TrialDirectory> {
        if !self.retired.iter().any(|old| old == id) {
            if self.retired.len() >= 64 {
                self.retired.pop_front();
            }
            self.retired.push_back(id.to_owned());
        }
        if let Some((_, cancel)) = self.active.as_ref().filter(|(current, _)| current == id) {
            cancel.store(true, Ordering::Release);
        }
        if self.ready.as_ref().is_some_and(|media| media.id == id) {
            return self.ready.take();
        }
        None
    }

    pub fn cancel_active(&self) {
        if let Some((_, cancel)) = &self.active {
            cancel.store(true, Ordering::Release);
        }
    }

    pub fn take_ready(&mut self) -> Option<TrialDirectory> {
        self.ready.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    #[test]
    fn cancelled_before_start_or_after_stop_never_publishes_media() {
        let cache = tempfile::tempdir().unwrap();
        let a = id();
        let b = id();
        let mut state = TrialState::default();
        state.release(&a);
        assert_eq!(state.reserve(&a).unwrap_err(), CANCELLED);
        let token = state.reserve(&b).unwrap();
        state.release(&a);
        assert!(!token.load(Ordering::Acquire));
        state.release(&b);
        let directory = TrialDirectory::create(cache.path(), &b).unwrap();
        let path = directory.path();
        assert_eq!(state.publish(directory).unwrap_err(), CANCELLED);
        assert!(!path.exists());
        state.finish(&b);
        assert!(state.reserve(&id()).is_ok());
    }

    #[test]
    fn replacing_or_discarding_only_removes_the_matching_test() {
        let cache = tempfile::tempdir().unwrap();
        let mut state = TrialState::default();
        let a = id();
        let b = id();
        state.reserve(&a).unwrap();
        state
            .publish(TrialDirectory::create(cache.path(), &a).unwrap())
            .unwrap();
        state.finish(&a);
        state.reserve(&b).unwrap();
        assert!(!cache.path().join(&a).exists());
        state
            .publish(TrialDirectory::create(cache.path(), &b).unwrap())
            .unwrap();
        state.finish(&b);
        state.release(&a);
        assert!(cache.path().join(&b).is_dir());
        state.release(&b);
        assert!(!cache.path().join(&b).exists());
    }

    #[test]
    fn interrupted_run_cleanup_keeps_unowned_entries_and_rejects_path_input() {
        let cache = tempfile::tempdir().unwrap();
        let owned = id();
        let foreign = id();
        let directory = TrialDirectory::create(cache.path(), &owned).unwrap();
        fs::create_dir(cache.path().join(&foreign)).unwrap();
        fs::write(cache.path().join(&foreign).join(OWNER_FILE), b"foreign").unwrap();
        assert!(TrialDirectory::create(cache.path(), "../other").is_err());
        assert!(TrialDirectory::create(cache.path(), &owned).is_err());
        std::mem::forget(directory);
        TrialDirectory::clean_previous_launch(cache.path()).unwrap();
        assert!(!cache.path().join(owned).exists());
        assert!(cache.path().join(foreign).exists());
    }

    #[test]
    fn blocked_windows_file_keeps_the_marker_for_next_launch() {
        use std::os::windows::fs::OpenOptionsExt;
        let cache = tempfile::tempdir().unwrap();
        let id = id();
        let directory = TrialDirectory::create(cache.path(), &id).unwrap();
        let path = directory.path();
        let locked = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(path.join("locked.wav"))
            .unwrap();
        drop(directory);
        assert!(path.join(OWNER_FILE).is_file());
        drop(locked);
        TrialDirectory::clean_previous_launch(cache.path()).unwrap();
        assert!(!path.exists());
    }
}
