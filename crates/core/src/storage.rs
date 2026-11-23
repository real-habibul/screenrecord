use crate::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::sync::Mutex;

const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LEGACY_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const MAX_CURSOR_SAMPLES: usize = 16_000_000;
const CURSOR_HEADER: &[u8; 8] = b"SRCURS02";
const LEGACY_CURSOR_HEADER: &[u8; 8] = b"SRCURS01";
const CURSOR_RECORD_BYTES: u64 = 25;
// Windows may reject replacing a manifest while another thread holds its read handle.
static MANIFEST_IO: Mutex<()> = Mutex::new(());

pub(crate) fn deserialize_cursor<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<CursorSample>, D::Error> {
    struct Samples;
    impl<'de> serde::de::Visitor<'de> for Samples {
        type Value = Vec<CursorSample>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("at most 16 million cursor samples")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut samples = Vec::new();
            while let Some(sample) = sequence.next_element()? {
                if samples.len() == MAX_CURSOR_SAMPLES {
                    return Err(serde::de::Error::custom("Cursor sample limit exceeded."));
                }
                samples.push(sample);
            }
            Ok(samples)
        }
    }
    deserializer.deserialize_seq(Samples)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorFile {
    name: String,
    samples: usize,
    sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest<'a> {
    version: u32,
    id: &'a str,
    name: &'a str,
    created_at: &'a str,
    directory: &'a str,
    screen_path: &'a str,
    microphone_path: &'a Option<String>,
    system_audio_path: &'a Option<String>,
    camera_path: &'a Option<String>,
    camera_offset_seconds: f64,
    camera_intervals: &'a Option<Vec<CameraInterval>>,
    width: u32,
    height: u32,
    fps: u32,
    duration: f64,
    cursor_file: &'a CursorFile,
    edits: &'a EditSettings,
    status: &'a str,
}

fn manifest_bytes(project: &Project, cursor_file: &CursorFile) -> Result<Vec<u8>, String> {
    let directory = dunce::canonicalize(&project.directory).map_err(|e| e.to_string())?;
    let relative = |path: &str| -> Result<String, String> {
        let resolved = dunce::canonicalize(path).map_err(|e| e.to_string())?;
        let relative = resolved
            .strip_prefix(&directory)
            .map_err(|_| "Project media must remain in its project folder.".to_string())?;
        if !resolved.is_file() || !normal_relative(relative) {
            return Err("Project media must be a file within its project folder.".into());
        }
        Ok(relative.to_string_lossy().into_owned())
    };
    let screen_path = relative(&project.screen_path)?;
    let microphone_path = project
        .microphone_path
        .as_deref()
        .map(relative)
        .transpose()?;
    let system_audio_path = project
        .system_audio_path
        .as_deref()
        .map(relative)
        .transpose()?;
    let camera_path = project.camera_path.as_deref().map(relative).transpose()?;
    let mut edits = project.edits.clone();
    for clip in &mut edits.music {
        clip.path = relative(&clip.path)?;
    }
    let bytes = serde_json::to_vec(&Manifest {
        version: PROJECT_VERSION,
        id: &project.id,
        name: &project.name,
        created_at: &project.created_at,
        directory: ".",
        screen_path: &screen_path,
        microphone_path: &microphone_path,
        system_audio_path: &system_audio_path,
        camera_path: &camera_path,
        camera_offset_seconds: project.camera_offset_seconds,
        camera_intervals: &project.camera_intervals,
        width: project.width,
        height: project.height,
        fps: project.fps,
        duration: project.duration,
        cursor_file,
        edits: &edits,
        status: &project.status,
    })
    .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("Project metadata exceeds the 16 MiB limit.".into());
    }
    Ok(bytes)
}

fn write_manifest(directory: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    temporary.write_all(bytes).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    let _guard = MANIFEST_IO
        .lock()
        .map_err(|_| "Project metadata lock failed.".to_string())?;
    temporary
        .persist(directory.join("project.json"))
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn save_project_atomic(project: &Project) -> Result<(), String> {
    project.validate()?;
    if project.cursor.len() > MAX_CURSOR_SAMPLES {
        return Err("Cursor data exceeds the 16 million sample limit.".into());
    }
    let directory = Path::new(&project.directory);
    let mut descriptor = CursorFile {
        name: format!("cursor-{}.bin", uuid::Uuid::new_v4()),
        samples: project.cursor.len(),
        sha256: "0".repeat(64),
    };
    manifest_bytes(project, &descriptor)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        let mut write = |bytes: &[u8]| -> Result<(), String> {
            writer.write_all(bytes).map_err(|e| e.to_string())?;
            hash.update(bytes);
            Ok(())
        };
        write(CURSOR_HEADER)?;
        write(&(descriptor.samples as u64).to_le_bytes())?;
        for sample in &project.cursor {
            let mut record = [0u8; 25];
            record[..8].copy_from_slice(&sample.time.to_le_bytes());
            record[8..16].copy_from_slice(&sample.x.to_le_bytes());
            record[16..24].copy_from_slice(&sample.y.to_le_bytes());
            record[24] =
                u8::from(sample.pressed) | (u8::from(sample.visible) << 1) | (sample.buttons << 2);
            write(&record)?;
        }
        writer.flush().map_err(|e| e.to_string())?;
    }
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    descriptor.sha256 = format!("{:x}", hash.finalize());
    let bytes = manifest_bytes(project, &descriptor)?;
    let cursor_path = directory.join(&descriptor.name);
    temporary
        .persist_noclobber(&cursor_path)
        .map_err(|e| e.to_string())?;
    // Publish the manifest last, so a crash cannot replace a project's cursor with partial data.
    if let Err(error) = write_manifest(directory, &bytes) {
        let _ = fs::remove_file(cursor_path);
        return Err(error);
    }
    Ok(())
}

fn project_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    let directory = dunce::canonicalize(project_directory(root, id)?)
        .map_err(|_| "Project folder is missing.".to_string())?;
    let root = dunce::canonicalize(root).map_err(|e| e.to_string())?;
    if directory == root || !directory.starts_with(root) {
        return Err("Project folder is outside the library.".into());
    }
    Ok(directory)
}

fn read_document(directory: &Path, id: &str) -> Result<(Project, Option<CursorFile>), String> {
    let _guard = MANIFEST_IO
        .lock()
        .map_err(|_| "Project metadata lock failed.".to_string())?;
    let path = directory.join("project.json");
    let file = fs::File::open(&path).map_err(|e| e.to_string())?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    if length > MAX_LEGACY_BYTES {
        return Err("Project metadata exceeds the legacy 1 GiB limit.".into());
    }
    let mut reader = BufReader::new(file);
    if length > MAX_MANIFEST_BYTES {
        #[derive(Deserialize)]
        struct Version {
            version: u32,
        }
        let version: Version = serde_json::from_reader(&mut reader).map_err(|e| e.to_string())?;
        if version.version != 1 {
            return Err("Project metadata exceeds the 16 MiB limit.".into());
        }
        reader.rewind().map_err(|e| e.to_string())?;
    }
    let mut project: Project =
        serde_json::from_reader(&mut reader).map_err(|e| format!("Cannot read project: {e}"))?;
    if project.id != id {
        return Err("Project identity does not match its folder.".into());
    }
    let cursor = match project.version {
        1 => {
            project.version = PROJECT_VERSION;
            None
        }
        2..=PROJECT_VERSION => {
            project.version = PROJECT_VERSION;
            if length > MAX_MANIFEST_BYTES || !project.cursor.is_empty() {
                return Err("Projects require bounded metadata and a cursor sidecar.".into());
            }
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Reference {
                cursor_file: CursorFile,
            }
            reader.rewind().map_err(|e| e.to_string())?;
            let reference: Reference = serde_json::from_reader(&mut reader)
                .map_err(|e| format!("Cannot read cursor reference: {e}"))?;
            Some(reference.cursor_file)
        }
        other => return Err(format!("Unsupported project version {other}.")),
    };
    project.validate()?;
    Ok((project, cursor))
}

fn read_cursor(directory: &Path, descriptor: &CursorFile) -> Result<Vec<CursorSample>, String> {
    let file = open_cursor(directory, descriptor)?;
    let mut reader = BufReader::new(file);
    let mut hash = Sha256::new();
    let mut header = [0u8; 16];
    reader.read_exact(&mut header).map_err(|e| e.to_string())?;
    hash.update(header);
    let legacy = &header[..8] == LEGACY_CURSOR_HEADER;
    if (!legacy && &header[..8] != CURSOR_HEADER)
        || u64::from_le_bytes(header[8..].try_into().unwrap()) != descriptor.samples as u64
    {
        return Err("Invalid cursor sidecar header.".into());
    }
    let mut samples = Vec::with_capacity(descriptor.samples);
    for _ in 0..descriptor.samples {
        let mut record = [0u8; 25];
        reader.read_exact(&mut record).map_err(|e| e.to_string())?;
        hash.update(record);
        let flags = record[24];
        let buttons = flags >> 2;
        let pressed = flags & 1 != 0;
        if (legacy && buttons != 0) || buttons > 7 || (buttons != 0 && !pressed) {
            return Err("Invalid cursor flags.".into());
        }
        samples.push(CursorSample {
            time: f64::from_le_bytes(record[..8].try_into().unwrap()),
            x: f64::from_le_bytes(record[8..16].try_into().unwrap()),
            y: f64::from_le_bytes(record[16..24].try_into().unwrap()),
            pressed,
            buttons,
            visible: flags & 2 != 0,
        });
    }
    if format!("{:x}", hash.finalize()) != descriptor.sha256 {
        return Err("Cursor sidecar checksum does not match.".into());
    }
    Ok(samples)
}

fn open_cursor(directory: &Path, descriptor: &CursorFile) -> Result<fs::File, String> {
    if descriptor.samples > MAX_CURSOR_SAMPLES
        || !descriptor.name.starts_with("cursor-")
        || !descriptor.name.ends_with(".bin")
        || Path::new(&descriptor.name).components().count() != 1
        || descriptor.sha256.len() != 64
    {
        return Err("Invalid cursor sidecar reference.".into());
    }
    let path = dunce::canonicalize(directory.join(&descriptor.name)).map_err(|e| e.to_string())?;
    if !path.starts_with(directory) {
        return Err("Cursor data must remain in its project folder.".into());
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len()
        != 16 + descriptor.samples as u64 * CURSOR_RECORD_BYTES
    {
        return Err("Cursor sidecar is truncated or has an invalid length.".into());
    }
    Ok(file)
}

fn normal_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn resolve_media(project: &mut Project, directory: &Path) -> Result<(), String> {
    let original_directory = PathBuf::from(&project.directory);
    for media in [&mut project.screen_path]
        .into_iter()
        .chain(
            [
                &mut project.microphone_path,
                &mut project.system_audio_path,
                &mut project.camera_path,
            ]
            .into_iter()
            .filter_map(Option::as_mut),
        )
        .chain(project.edits.music.iter_mut().map(|clip| &mut clip.path))
    {
        let stored = Path::new(&media);
        let relative = if stored.is_absolute() {
            stored
                .strip_prefix(&original_directory)
                .or_else(|_| stored.strip_prefix(directory))
                .map_err(|_| "Project media must remain in its project folder.".to_string())?
        } else {
            stored
        };
        if !normal_relative(relative) {
            return Err("Project media has an invalid relative path.".into());
        }
        let resolved = dunce::canonicalize(directory.join(relative))
            .map_err(|_| format!("Project media is missing: {media}"))?;
        if !resolved.starts_with(directory) || !resolved.is_file() {
            return Err("Project media must remain in its project folder.".into());
        }
        *media = resolved.to_string_lossy().into_owned();
    }
    project.directory = directory.to_string_lossy().into_owned();
    Ok(())
}

pub fn load_project(root: &Path, id: &str) -> Result<Project, String> {
    let directory = project_dir(root, id)?;
    let (mut project, cursor) = read_document(&directory, id)?;
    if let Some(cursor) = cursor {
        project.cursor = read_cursor(&directory, &cursor)?;
    }
    resolve_media(&mut project, &directory)?;
    project.validate()?;
    Ok(project)
}

pub fn load_project_metadata(root: &Path, id: &str) -> Result<Project, String> {
    let directory = project_dir(root, id)?;
    let (mut project, _) = read_document(&directory, id)?;
    resolve_media(&mut project, &directory)?;
    project.validate()?;
    Ok(project)
}

pub fn load_project_summary(root: &Path, id: &str) -> Result<ProjectSummary, String> {
    let directory = project_dir(root, id)?;
    let _guard = MANIFEST_IO
        .lock()
        .map_err(|_| "Project metadata lock failed.".to_string())?;
    let file = fs::File::open(directory.join("project.json")).map_err(|e| e.to_string())?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    if length > MAX_LEGACY_BYTES {
        return Err("Project metadata exceeds the legacy 1 GiB limit.".into());
    }
    let mut reader = BufReader::new(file);
    if length > MAX_MANIFEST_BYTES {
        #[derive(Deserialize)]
        struct Version {
            version: u32,
        }
        let version: Version = serde_json::from_reader(&mut reader).map_err(|e| e.to_string())?;
        if version.version != 1 {
            return Err("Project metadata exceeds the 16 MiB limit.".into());
        }
        reader.rewind().map_err(|e| e.to_string())?;
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct SummaryDocument {
        id: String,
        name: String,
        created_at: String,
        duration: f64,
        status: String,
        edits: EditSettings,
        version: u32,
    }
    let document: SummaryDocument = serde_json::from_reader(reader).map_err(|e| e.to_string())?;
    let mut summary = ProjectSummary {
        id: document.id,
        name: document.name,
        created_at: document.created_at,
        duration: document.duration,
        directory: directory.to_string_lossy().into_owned(),
        status: document.status,
    };
    if summary.id != id
        || !summary.duration.is_finite()
        || summary.duration <= 0.0
        || !["ready", "interrupted"].contains(&summary.status.as_str())
    {
        return Err("Invalid project summary.".into());
    }
    validate_name(&summary.name)?;
    if !(1..=PROJECT_VERSION).contains(&document.version) {
        return Err(format!("Unsupported project version {}.", document.version));
    }
    document.edits.validate(summary.duration)?;
    summary.duration = edited_duration(&document.edits, summary.duration);
    Ok(summary)
}

pub fn save_project_edits(
    root: &Path,
    id: &str,
    name: &str,
    edits: EditSettings,
) -> Result<SavedEdits, String> {
    let directory = project_dir(root, id)?;
    let (mut project, cursor) = read_document(&directory, id)?;
    validate_name(name)?;
    edits.validate(project.duration)?;
    project.name = name.trim().to_owned();
    project.edits = edits;
    resolve_media(&mut project, &directory)?;
    if let Some(cursor) = cursor {
        open_cursor(&directory, &cursor)?;
        write_manifest(&directory, &manifest_bytes(&project, &cursor)?)?;
    } else {
        save_project_atomic(&project)?;
    }
    Ok(SavedEdits {
        name: project.name,
        edits: project.edits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn read_only_manifest_keeps_the_previous_edit_and_allows_retry() {
        let root = tempfile::tempdir().unwrap();
        let project = crate::tests::fixture(root.path());
        save_project_atomic(&project).unwrap();
        let directory = Path::new(&project.directory);
        let manifest = directory.join("project.json");
        let before = fs::read(&manifest).unwrap();
        let file_count = fs::read_dir(directory).unwrap().count();
        let original_permissions = fs::metadata(&manifest).unwrap().permissions();
        let mut locked = original_permissions.clone();
        locked.set_readonly(true);
        fs::set_permissions(&manifest, locked).unwrap();
        let failure =
            save_project_edits(root.path(), &project.id, "New name", project.edits.clone());
        fs::set_permissions(&manifest, original_permissions).unwrap();
        assert!(failure.is_err());
        assert_eq!(fs::read(&manifest).unwrap(), before);
        assert_eq!(fs::read_dir(directory).unwrap().count(), file_count);
        save_project_edits(root.path(), &project.id, "New name", project.edits).unwrap();
        assert_eq!(
            load_project(root.path(), &project.id).unwrap().name,
            "New name"
        );
    }

    #[test]
    fn reframe_edits_preserve_cursor_media_and_previous_manifest_on_invalid_save() {
        let root = tempfile::tempdir().unwrap();
        let project = crate::tests::fixture(root.path());
        save_project_atomic(&project).unwrap();
        let directory = Path::new(&project.directory);
        let manifest = directory.join("project.json");
        let before: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        let sidecar = directory.join(before["cursorFile"]["name"].as_str().unwrap());
        let sidecar_bytes = fs::read(&sidecar).unwrap();
        let media = fs::read(&project.screen_path).unwrap();
        let mut edits = project.edits.clone();
        edits.fit_mode = "fill".into();
        edits.background_mode = "videoBlur".into();
        edits.background_blur = 40.0;
        edits.reframe_keyframes = vec![
            ReframeKeyframe {
                id: "first".into(),
                time: 0.0,
                x: 0.25,
                y: 0.4,
                scale: 1.0,
            },
            ReframeKeyframe {
                id: "last".into(),
                time: project.duration,
                x: 0.75,
                y: 0.6,
                scale: 2.0,
            },
        ];
        save_project_edits(root.path(), &project.id, &project.name, edits.clone()).unwrap();
        let valid_bytes = fs::read(&manifest).unwrap();
        let current: serde_json::Value = serde_json::from_slice(&valid_bytes).unwrap();
        assert_eq!(current["cursorFile"], before["cursorFile"]);
        let restored = load_project(root.path(), &project.id).unwrap();
        assert_eq!(restored.edits.reframe_keyframes[1].time, project.duration);
        assert_eq!(restored.edits.reframe_keyframes[1].scale, 2.0);
        assert_eq!(restored.edits.background_mode, "videoBlur");
        assert_eq!(restored.edits.background_blur, 40.0);
        edits.reframe_keyframes[1].time = 0.0;
        assert!(
            save_project_edits(root.path(), &project.id, &project.name, edits.clone()).is_err()
        );
        edits.reframe_keyframes[1].time = project.duration;
        edits.background_blur = f64::NAN;
        assert!(save_project_edits(root.path(), &project.id, &project.name, edits).is_err());
        assert_eq!(fs::read(&manifest).unwrap(), valid_bytes);
        assert_eq!(fs::read(sidecar).unwrap(), sidecar_bytes);
        assert_eq!(fs::read(&project.screen_path).unwrap(), media);
    }

    #[test]
    fn version_four_loads_legacy_fit_without_rewriting_until_save() {
        let root = tempfile::tempdir().unwrap();
        let project = crate::tests::fixture(root.path());
        save_project_atomic(&project).unwrap();
        let path = Path::new(&project.directory).join("project.json");
        let mut stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["version"] = 4.into();
        stored["edits"].as_object_mut().unwrap().remove("fitMode");
        stored["edits"]
            .as_object_mut()
            .unwrap()
            .remove("reframeKeyframes");
        stored["edits"]
            .as_object_mut()
            .unwrap()
            .remove("backgroundMode");
        stored["edits"]
            .as_object_mut()
            .unwrap()
            .remove("backgroundBlur");
        let original = serde_json::to_vec(&stored).unwrap();
        fs::write(&path, &original).unwrap();
        let mut loaded = load_project(root.path(), &project.id).unwrap();
        assert_eq!(loaded.version, PROJECT_VERSION);
        assert_eq!(loaded.edits.fit_mode, "fit");
        load_project_summary(root.path(), &project.id).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        loaded.edits.fit_mode = "fill".into();
        save_project_atomic(&loaded).unwrap();
        let restored = load_project(root.path(), &project.id).unwrap();
        assert_eq!(restored.edits.fit_mode, "fill");
        let current: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(current["version"], PROJECT_VERSION);
    }

    #[test]
    fn version_three_migrates_and_cut_edits_roundtrip_after_relocation() {
        let root = tempfile::tempdir().unwrap();
        let project = crate::tests::fixture(root.path());
        save_project_atomic(&project).unwrap();
        let path = Path::new(&project.directory).join("project.json");
        let mut stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["version"] = 3.into();
        stored["edits"]
            .as_object_mut()
            .unwrap()
            .remove("videoClips");
        let original = serde_json::to_vec(&stored).unwrap();
        fs::write(&path, &original).unwrap();
        let mut loaded = load_project(root.path(), &project.id).unwrap();
        assert_eq!(loaded.version, PROJECT_VERSION);
        assert!(loaded.edits.video_clips.is_empty());
        assert_eq!(fs::read(&path).unwrap(), original);
        loaded.edits.video_clips = vec![
            VideoClip {
                id: "before".into(),
                start: 0.0,
                end: 0.3,
            },
            VideoClip {
                id: "after".into(),
                start: 0.6,
                end: 1.0,
            },
        ];
        save_project_edits(root.path(), &project.id, "Cut", loaded.edits).unwrap();
        let current: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(current["version"], PROJECT_VERSION);
        assert_eq!(current["edits"]["videoClips"].as_array().unwrap().len(), 2);
        let target = tempfile::tempdir().unwrap();
        let copied = target.path().join(&project.id);
        fs::create_dir(&copied).unwrap();
        for entry in fs::read_dir(&project.directory).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), copied.join(entry.file_name())).unwrap();
        }
        let relocated = load_project(target.path(), &project.id).unwrap();
        assert_eq!(relocated.edits.video_clips[1].start, 0.6);
        assert!((edited_duration(&relocated.edits, relocated.duration) - 0.7).abs() < 1e-9);
        assert!(
            (load_project_summary(target.path(), &project.id)
                .unwrap()
                .duration
                - 0.7)
                .abs()
                < 1e-9
        );
        assert!(Path::new(&relocated.screen_path).starts_with(dunce::canonicalize(copied).unwrap()));
    }

    #[test]
    fn version_two_migrates_without_rewriting_until_save() {
        let root = tempfile::tempdir().unwrap();
        let project = crate::tests::fixture(root.path());
        save_project_atomic(&project).unwrap();
        let path = Path::new(&project.directory).join("project.json");
        let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        old["version"] = 2.into();
        for field in ["music", "subtitles", "subtitleStyle", "redactions"] {
            old["edits"].as_object_mut().unwrap().remove(field);
        }
        let original = serde_json::to_vec(&old).unwrap();
        fs::write(&path, &original).unwrap();
        let loaded = load_project(root.path(), &project.id).unwrap();
        assert_eq!(loaded.version, PROJECT_VERSION);
        assert_eq!(fs::read(&path).unwrap(), original);
        save_project_edits(root.path(), &project.id, "Migrated", loaded.edits).unwrap();
        let current: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(current["version"], PROJECT_VERSION);
        assert!(current["edits"]["music"].as_array().unwrap().is_empty());
    }

    #[test]
    fn music_assets_relocate_and_external_paths_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        let music = Path::new(&project.directory).join("music-test.wav");
        fs::write(&music, b"audio path fixture").unwrap();
        project.edits.music.push(MusicClip {
            id: "music".into(),
            name: "Test".into(),
            path: music.to_string_lossy().into_owned(),
            source_duration: 1.0,
            start: 0.0,
            source_start: 0.0,
            duration: 0.5,
            volume: 0.5,
            fade_in: 0.1,
            fade_out: 0.1,
        });
        save_project_atomic(&project).unwrap();
        let target = tempfile::tempdir().unwrap();
        let copied = target.path().join(&project.id);
        fs::create_dir(&copied).unwrap();
        for entry in fs::read_dir(&project.directory).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), copied.join(entry.file_name())).unwrap();
        }
        let loaded = load_project(target.path(), &project.id).unwrap();
        assert_eq!(
            Path::new(&loaded.edits.music[0].path),
            dunce::canonicalize(copied.join("music-test.wav")).unwrap()
        );
        let original = fs::read(copied.join("project.json")).unwrap();
        let mut bad = loaded.edits.clone();
        bad.music[0].path = music.to_string_lossy().into_owned();
        assert!(save_project_edits(target.path(), &project.id, "Escaped", bad).is_err());
        assert_eq!(fs::read(copied.join("project.json")).unwrap(), original);
        let manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(manifest["edits"]["music"][0]["path"], "music-test.wav");
    }

    #[test]
    fn eight_hour_cursor_roundtrip_keeps_manifest_small() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        project.duration = 8.0 * 3600.0;
        project.edits = EditSettings::new(project.duration);
        project.cursor = (0..3_600_000)
            .map(|i| CursorSample {
                time: i as f64 / 125.0,
                x: (i % 1920) as f64 / 1920.0,
                y: (i % 1080) as f64 / 1080.0,
                pressed: i % 12345 == 0,
                buttons: if i % 12345 == 0 {
                    1 << ((i / 12345) % 3)
                } else {
                    0
                },
                visible: i % 61 != 0,
            })
            .collect();
        save_project_atomic(&project).unwrap();
        let manifest = fs::read(Path::new(&project.directory).join("project.json")).unwrap();
        assert!(manifest.len() < 4096);
        let original_reference = read_document(Path::new(&project.directory), &project.id)
            .unwrap()
            .1
            .unwrap();
        for i in 0..5 {
            save_project_edits(
                root.path(),
                &project.id,
                &format!("Take {i}"),
                project.edits.clone(),
            )
            .unwrap();
            assert_eq!(
                load_project_summary(root.path(), &project.id).unwrap().name,
                format!("Take {i}")
            );
        }
        let current_reference = read_document(Path::new(&project.directory), &project.id)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(original_reference.name, current_reference.name);
        assert_eq!(original_reference.sha256, current_reference.sha256);
        assert_eq!(
            fs::read_dir(&project.directory)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "bin"))
                .count(),
            1
        );
        let restored = load_project(root.path(), &project.id).unwrap();
        assert_eq!(restored.cursor.len(), project.cursor.len());
        for (before, after) in project.cursor.iter().zip(&restored.cursor) {
            assert_eq!(before.time.to_bits(), after.time.to_bits());
            assert_eq!(before.x.to_bits(), after.x.to_bits());
            assert_eq!(before.y.to_bits(), after.y.to_bits());
            assert_eq!(before.pressed, after.pressed);
            assert_eq!(before.buttons, after.buttons);
            assert_eq!(before.visible, after.visible);
        }
    }

    #[test]
    fn legacy_project_migrates_without_changing_original_until_save() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        project.version = 1;
        project.cursor.push(CursorSample {
            time: 1.0,
            x: 0.2,
            y: 0.8,
            pressed: true,
            buttons: 0,
            visible: true,
        });
        let path = Path::new(&project.directory).join("project.json");
        let original = serde_json::to_vec(&project).unwrap();
        fs::write(&path, &original).unwrap();
        let restored = load_project(root.path(), &project.id).unwrap();
        assert_eq!(restored.version, PROJECT_VERSION);
        assert_eq!(restored.cursor.len(), 1);
        assert_eq!(fs::read(&path).unwrap(), original);
        save_project_atomic(&restored).unwrap();
        assert_eq!(
            load_project(root.path(), &project.id).unwrap().cursor[0].x,
            0.2
        );
    }

    #[test]
    fn version_five_aggregate_clicks_survive_edit_only_save_with_original_sidecar() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        project.cursor.push(CursorSample {
            time: 0.5,
            x: 0.25,
            y: 0.75,
            pressed: true,
            buttons: 0,
            visible: true,
        });
        save_project_atomic(&project).unwrap();
        let directory = Path::new(&project.directory);
        let manifest_path = directory.join("project.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        let sidecar_path = directory.join(manifest["cursorFile"]["name"].as_str().unwrap());
        let mut sidecar = fs::read(&sidecar_path).unwrap();
        assert_eq!(&sidecar[..8], CURSOR_HEADER);
        sidecar[..8].copy_from_slice(LEGACY_CURSOR_HEADER);
        fs::write(&sidecar_path, &sidecar).unwrap();
        manifest["cursorFile"]["sha256"] = format!("{:x}", Sha256::digest(&sidecar)).into();
        manifest["version"] = 5.into();
        manifest["edits"]
            .as_object_mut()
            .unwrap()
            .remove("showClicks");
        let old_manifest = serde_json::to_vec(&manifest).unwrap();
        fs::write(&manifest_path, &old_manifest).unwrap();

        let mut loaded = load_project(root.path(), &project.id).unwrap();
        assert!(loaded.edits.show_clicks);
        assert_eq!(
            cursor_clicks(&loaded.cursor).next().unwrap().button,
            ClickButton::Generic
        );
        load_project_summary(root.path(), &project.id).unwrap();
        assert_eq!(fs::read(&manifest_path).unwrap(), old_manifest);
        loaded.edits.show_clicks = false;
        save_project_edits(root.path(), &project.id, &project.name, loaded.edits).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(saved["version"], PROJECT_VERSION);
        assert_eq!(saved["cursorFile"], manifest["cursorFile"]);
        assert_eq!(fs::read(sidecar_path).unwrap(), sidecar);
        let reopened = load_project(root.path(), &project.id).unwrap();
        assert!(!reopened.edits.show_clicks);
        assert_eq!(cursor_clicks(&reopened.cursor).count(), 1);
    }

    #[test]
    fn inconsistent_and_unknown_cursor_flags_are_rejected_in_both_formats() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        project.cursor.push(CursorSample {
            time: 0.5,
            x: 0.2,
            y: 0.3,
            pressed: true,
            buttons: 7,
            visible: true,
        });
        save_project_atomic(&project).unwrap();
        let directory = project_dir(root.path(), &project.id).unwrap();
        let mut descriptor = read_document(&directory, &project.id).unwrap().1.unwrap();
        let path = directory.join(&descriptor.name);
        let mut record = fs::read(&path).unwrap();
        assert_eq!(record[40], 31);
        for (header, invalid_flags) in [
            (LEGACY_CURSOR_HEADER, 7),
            (CURSOR_HEADER, 32),
            (CURSOR_HEADER, 4),
        ] {
            record[..8].copy_from_slice(header);
            record[40] = invalid_flags;
            descriptor.sha256 = format!("{:x}", Sha256::digest(&record));
            fs::write(&path, &record).unwrap();
            assert!(read_cursor(&directory, &descriptor)
                .unwrap_err()
                .contains("flags"));
        }
        project.cursor[0].pressed = false;
        assert!(project.validate().unwrap_err().contains("button"));
        project.cursor[0].pressed = true;
        project.cursor[0].buttons = 8;
        assert!(project.validate().unwrap_err().contains("button"));
    }

    #[test]
    fn corrupt_or_missing_sidecar_is_not_silently_accepted() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        project.cursor.push(CursorSample {
            time: 1.0,
            x: 0.2,
            y: 0.8,
            pressed: true,
            buttons: 0,
            visible: true,
        });
        save_project_atomic(&project).unwrap();
        let directory = Path::new(&project.directory);
        let (_, descriptor) = read_document(directory, &project.id).unwrap();
        let path = directory.join(descriptor.unwrap().name);
        let mut bytes = fs::read(&path).unwrap();
        bytes[24] ^= 1;
        fs::write(&path, &bytes).unwrap();
        assert!(load_project(root.path(), &project.id)
            .unwrap_err()
            .contains("checksum"));
        fs::write(&path, &bytes[..20]).unwrap();
        assert!(load_project(root.path(), &project.id)
            .unwrap_err()
            .contains("truncated"));
        fs::remove_file(path).unwrap();
        assert!(load_project(root.path(), &project.id).is_err());
    }

    #[test]
    fn opening_during_atomic_save_reads_one_manifest_generation() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        project.name = "0".into();
        project.cursor.push(CursorSample {
            time: 1.0,
            x: 0.0,
            y: 0.5,
            pressed: false,
            buttons: 0,
            visible: true,
        });
        save_project_atomic(&project).unwrap();
        let id = project.id.clone();
        let writer = std::thread::spawn(move || {
            for i in 0..40 {
                project.name = (i % 2).to_string();
                project.cursor[0].x = (i % 2) as f64;
                save_project_atomic(&project).unwrap();
            }
        });
        for _ in 0..80 {
            let loaded = load_project(root.path(), &id).unwrap();
            assert_eq!(loaded.name.parse::<f64>().unwrap(), loaded.cursor[0].x);
        }
        writer.join().unwrap();
    }

    #[test]
    fn rejects_oversized_v2_manifest() {
        let root = tempfile::tempdir().unwrap();
        let project = crate::tests::fixture(root.path());
        save_project_atomic(&project).unwrap();
        let path = Path::new(&project.directory).join("project.json");
        let mut bytes = fs::read(&path).unwrap();
        bytes.resize(MAX_MANIFEST_BYTES as usize + 1, b' ');
        fs::write(path, bytes).unwrap();
        assert!(load_project(root.path(), &project.id)
            .unwrap_err()
            .contains("16 MiB"));
        assert!(load_project_summary(root.path(), &project.id)
            .unwrap_err()
            .contains("16 MiB"));
    }

    #[test]
    fn relocated_legacy_and_v2_projects_use_their_new_media_directory() {
        for legacy in [false, true] {
            let source = tempfile::tempdir().unwrap();
            let target = tempfile::tempdir().unwrap();
            let mut project = crate::tests::fixture(source.path());
            project.camera_path = Some(project.screen_path.clone());
            project.camera_offset_seconds = 0.75;
            if legacy {
                project.version = 1;
                fs::write(
                    Path::new(&project.directory).join("project.json"),
                    serde_json::to_vec(&project).unwrap(),
                )
                .unwrap();
            } else {
                save_project_atomic(&project).unwrap();
            }
            let copied = target.path().join(&project.id);
            fs::create_dir(&copied).unwrap();
            for entry in fs::read_dir(&project.directory).unwrap() {
                let entry = entry.unwrap();
                fs::copy(entry.path(), copied.join(entry.file_name())).unwrap();
            }
            let loaded = load_project(target.path(), &project.id).unwrap();
            assert!(
                Path::new(&loaded.screen_path).starts_with(dunce::canonicalize(&copied).unwrap())
            );
            assert_eq!(loaded.camera_offset_seconds, 0.75);
            save_project_edits(target.path(), &project.id, "Moved", loaded.edits).unwrap();
            let stored: serde_json::Value =
                serde_json::from_slice(&fs::read(copied.join("project.json")).unwrap()).unwrap();
            assert_eq!(stored["screenPath"], "screen.mp4");
            assert_eq!(
                load_project(target.path(), &project.id).unwrap().name,
                "Moved"
            );
        }
    }

    #[test]
    fn invalid_edits_preserve_manifest_and_summary_does_not_read_cursor() {
        let root = tempfile::tempdir().unwrap();
        let mut project = crate::tests::fixture(root.path());
        save_project_atomic(&project).unwrap();
        let directory = Path::new(&project.directory);
        let before = fs::read(directory.join("project.json")).unwrap();
        project.edits.trim_end = 100.0;
        assert!(save_project_edits(root.path(), &project.id, "Changed", project.edits).is_err());
        assert_eq!(fs::read(directory.join("project.json")).unwrap(), before);
        let (_, cursor) = read_document(directory, &project.id).unwrap();
        fs::write(directory.join(cursor.unwrap().name), b"broken").unwrap();
        assert_eq!(
            load_project_summary(root.path(), &project.id).unwrap().name,
            project.name
        );
        assert!(load_project(root.path(), &project.id).is_err());
    }

    #[test]
    fn relative_media_traversal_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let project = crate::tests::fixture(root.path());
        save_project_atomic(&project).unwrap();
        let path = Path::new(&project.directory).join("project.json");
        let mut stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["screenPath"] = "../outside.mp4".into();
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(load_project(root.path(), &project.id)
            .unwrap_err()
            .contains("relative path"));
    }
}
