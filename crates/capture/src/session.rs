use crate::{preview::PreviewSnapshot, CaptureResult, Recorder};
use screenrecord_core::{RecordingOptions, MAX_RECORDING_RECOVERY_SECONDS, MAX_RECORDING_SECONDS};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

const JOURNAL_NAME: &str = "session-journal.json";
const MAX_PARTS: usize = 512;
const MAX_JOURNAL_BYTES: u64 = 128 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionJournal {
    version: u32,
    dimensions: Option<(u32, u32, u32)>,
    parts: Vec<JournalPart>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JournalPart {
    directory: String,
    duration: Option<f64>,
}

trait Part: Send {
    fn elapsed(&self) -> f64;
    fn health(&self) -> Result<(), String>;
    fn dimensions(&self) -> (u32, u32, u32);
    fn snapshot(&self) -> PreviewSnapshot;
    fn finish(self: Box<Self>) -> Result<CaptureResult, String>;
}

impl Part for Recorder {
    fn elapsed(&self) -> f64 {
        self.elapsed()
    }
    fn health(&self) -> Result<(), String> {
        self.health()
    }
    fn dimensions(&self) -> (u32, u32, u32) {
        (self.journal.width, self.journal.height, self.journal.fps)
    }
    fn snapshot(&self) -> PreviewSnapshot {
        self.camera_snapshot()
    }
    fn finish(self: Box<Self>) -> Result<CaptureResult, String> {
        (*self).stop()
    }
}

type Opener = dyn Fn(&Path, &RecordingOptions, &Path, Option<(u32, u32, u32)>) -> Result<Box<dyn Part>, String>
    + Send
    + Sync;

pub struct RecordingSession {
    directory: PathBuf,
    ffmpeg: PathBuf,
    options: RecordingOptions,
    journal: SessionJournal,
    recorder: Option<Box<dyn Part>>,
    results: Vec<CaptureResult>,
    failed_elapsed: f64,
    failure: Option<String>,
    warnings: Vec<String>,
    opener: Arc<Opener>,
}

impl RecordingSession {
    pub fn start(
        directory: &Path,
        options: &RecordingOptions,
        ffmpeg: &Path,
    ) -> Result<Self, String> {
        crate::cleanup::ensure_idle()?;
        Self::start_with(directory, options, ffmpeg, Arc::new(open_native))
    }

    pub fn start_cancellable(
        directory: &Path,
        options: &RecordingOptions,
        ffmpeg: &Path,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        crate::startup::check_cancel(Some(&cancel))?;
        crate::cleanup::ensure_idle()?;
        let native_cancel = cancel.clone();
        Self::start_cancellable_with(
            directory,
            options,
            ffmpeg,
            cancel,
            Arc::new(move |directory, options, ffmpeg, expected| {
                open_native_with_cancel(
                    directory,
                    options,
                    ffmpeg,
                    expected,
                    Some(native_cancel.clone()),
                )
            }),
        )
    }

    fn start_cancellable_with(
        directory: &Path,
        options: &RecordingOptions,
        ffmpeg: &Path,
        cancel: Arc<AtomicBool>,
        opener: Arc<Opener>,
    ) -> Result<Self, String> {
        crate::startup::check_cancel(Some(&cancel))?;
        let part_cancel = cancel.clone();
        let wrapped: Arc<Opener> = Arc::new(move |directory, options, ffmpeg, expected| {
            crate::startup::check_cancel(Some(&part_cancel))?;
            let result = opener(directory, options, ffmpeg, expected);
            if crate::startup::check_cancel(Some(&part_cancel)).is_err() {
                drop(result);
                Err(crate::startup::CANCELLED.into())
            } else {
                result
            }
        });
        let result = Self::start_with(directory, options, ffmpeg, wrapped);
        if crate::startup::check_cancel(Some(&cancel)).is_err() {
            drop(result);
            Err(crate::startup::CANCELLED.into())
        } else {
            result
        }
    }

    fn start_with(
        directory: &Path,
        options: &RecordingOptions,
        ffmpeg: &Path,
        opener: Arc<Opener>,
    ) -> Result<Self, String> {
        options.validate()?;
        fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let directory = crate::canonical_media_directory(directory)?;
        for name in [
            JOURNAL_NAME,
            "session-journal.tmp",
            "capture-journal.json",
            "part-00000",
        ] {
            if directory.join(name).exists() {
                return Err("This project already contains recording data.".into());
            }
        }
        let mut session = Self {
            directory,
            ffmpeg: ffmpeg.to_owned(),
            options: options.clone(),
            journal: SessionJournal {
                version: 1,
                dimensions: None,
                parts: Vec::new(),
            },
            recorder: None,
            results: Vec::new(),
            failed_elapsed: 0.0,
            failure: None,
            warnings: Vec::new(),
            opener,
        };
        session.resume()?;
        Ok(session)
    }

    pub fn pause(&mut self) -> Result<(), String> {
        let Some(recorder) = self.recorder.take() else {
            return self.failure.clone().map_or(Ok(()), Err);
        };
        self.failed_elapsed = recorder.elapsed();
        let result = recorder.finish().and_then(|result| {
            if Some((result.width, result.height, result.fps)) != self.journal.dimensions {
                return Err(
                    "Recording dimensions changed. Stop the session to recover the saved parts."
                        .into(),
                );
            }
            if !result.duration.is_finite() || result.duration <= 0.0 {
                return Err("The recording part has no usable duration.".into());
            }
            self.journal.parts.last_mut().unwrap().duration = Some(result.duration);
            self.results.push(result);
            self.failed_elapsed = 0.0;
            self.write_journal()?;
            let part = self.journal.parts.last().unwrap();
            if let Some(warning) = part_warning(&self.directory, &part.directory)? {
                self.warnings.push(warning);
            }
            publish_warnings(&self.directory, &self.warnings)
        });
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if self.recorder.is_some() {
            return Ok(());
        }
        if let Some(error) = &self.failure {
            return Err(format!(
                "This session needs recovery before recording can continue: {error}"
            ));
        }
        if self.journal.parts.len() >= MAX_PARTS {
            return Err("This recording reached the limit of 512 parts. Stop and save it before starting another recording.".into());
        }
        if self.elapsed() >= MAX_RECORDING_SECONDS {
            return Err("This recording reached eight hours. Stop and save it before starting another recording.".into());
        }
        let name = format!("part-{:05}", self.journal.parts.len());
        let path = self.directory.join(&name);
        if path.exists() {
            return Err(
                "The next recording part already exists. Stop and recover this session.".into(),
            );
        }
        self.journal.parts.push(JournalPart {
            directory: name,
            duration: None,
        });
        if let Err(error) = self.write_journal() {
            self.journal.parts.pop();
            return Err(error);
        }
        let result = (self.opener)(&path, &self.options, &self.ffmpeg, self.journal.dimensions)
            .and_then(|recorder| {
                let dimensions = recorder.dimensions();
                if self.journal.dimensions.is_some_and(|expected| expected != dimensions) {
                    drop(recorder);
                    return Err("The capture source changed size while resuming. Stop and recover the saved parts.".into());
                }
                self.journal.dimensions = Some(dimensions);
                self.recorder = Some(recorder);
                self.write_journal()
            });
        if let Err(error) = &result {
            self.recorder.take();
            self.failure = Some(error.clone());
        }
        result
    }

    pub fn elapsed(&self) -> f64 {
        self.results.iter().map(|part| part.duration).sum::<f64>()
            + self
                .recorder
                .as_ref()
                .map_or(self.failed_elapsed, |part| part.elapsed())
    }

    pub fn is_paused(&self) -> bool {
        self.recorder.is_none()
    }

    pub fn health(&self) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if self.elapsed() >= MAX_RECORDING_SECONDS {
            return Err(
                "The recording reached eight hours. Stop recording to save the available media."
                    .into(),
            );
        }
        self.recorder.as_ref().map_or(Ok(()), |part| part.health())
    }

    pub fn preview_snapshot(&self) -> PreviewSnapshot {
        self.recorder
            .as_ref()
            .map_or_else(inactive_snapshot, |part| part.snapshot())
    }

    pub fn generation(&self) -> u32 {
        self.journal.parts.len() as u32
    }

    pub fn stop(mut self) -> Result<Vec<CaptureResult>, String> {
        self.pause()?;
        if self.results.is_empty() {
            return Err("No video frames were recorded.".into());
        }
        Ok(std::mem::take(&mut self.results))
    }

    fn write_journal(&self) -> Result<(), String> {
        let temporary = self.directory.join("session-journal.tmp");
        reject_reparse(&temporary)?;
        reject_reparse(&self.directory.join(JOURNAL_NAME))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        serde_json::to_writer(&mut file, &self.journal).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        fs::rename(temporary, self.directory.join(JOURNAL_NAME)).map_err(|error| error.to_string())
    }
}

fn open_native(
    directory: &Path,
    options: &RecordingOptions,
    ffmpeg: &Path,
    expected: Option<(u32, u32, u32)>,
) -> Result<Box<dyn Part>, String> {
    open_native_with_cancel(directory, options, ffmpeg, expected, None)
}

fn open_native_with_cancel(
    directory: &Path,
    options: &RecordingOptions,
    ffmpeg: &Path,
    expected: Option<(u32, u32, u32)>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<Box<dyn Part>, String> {
    crate::startup::check_cancel(cancel.as_deref())?;
    crate::cleanup::ensure_idle()?;
    if let Some((width, height, fps)) = expected {
        let (_, rect) = crate::validated_target(&options.source_id)?;
        let dimensions = crate::region::dimensions(
            (
                (rect.right as i64 - rect.left as i64) as u32,
                (rect.bottom as i64 - rect.top as i64) as u32,
            ),
            options.region,
        )?;
        let actual = (dimensions.0, dimensions.1, options.fps);
        if actual != (width, height, fps) {
            return Err("The selected source changed size while paused. Restore its original size, then stop this session to preserve its recorded parts.".into());
        }
    }
    match cancel {
        Some(cancel) => Ok(Box::new(Recorder::start_cancellable(
            directory, options, ffmpeg, cancel,
        )?)),
        None => Ok(Box::new(Recorder::start(directory, options, ffmpeg)?)),
    }
}

pub(crate) fn inactive_snapshot() -> PreviewSnapshot {
    PreviewSnapshot {
        sequence: 0,
        jpeg: None,
        active: false,
        error: None,
    }
}

fn reject_reparse(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("Recording session paths cannot contain links or junctions.".into());
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn read_journal(directory: &Path) -> Result<SessionJournal, String> {
    let path = directory.join(JOURNAL_NAME);
    reject_reparse(&path)?;
    let file = File::open(&path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_JOURNAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err("The recording session journal is too large.".into());
    }
    let journal: SessionJournal = serde_json::from_slice(&bytes)
        .map_err(|_| "The recording session journal is invalid.".to_owned())?;
    if journal.version != 1 || journal.parts.is_empty() || journal.parts.len() > MAX_PARTS {
        return Err("The recording session journal has unsupported parts.".into());
    }
    if journal.dimensions.is_some_and(|(w, h, f)| {
        !(2..=16384).contains(&w)
            || !(2..=16384).contains(&h)
            || w % 2 != 0
            || h % 2 != 0
            || ![30, 60].contains(&f)
    }) {
        return Err("The recording session dimensions are invalid.".into());
    }
    if journal
        .parts
        .iter()
        .filter_map(|part| part.duration)
        .sum::<f64>()
        > MAX_RECORDING_RECOVERY_SECONDS
    {
        return Err("The recording session exceeds the supported recovery duration.".into());
    }
    for (index, part) in journal.parts.iter().enumerate() {
        if part.directory != format!("part-{index:05}")
            || part.duration.is_some_and(|duration| {
                !duration.is_finite()
                    || duration <= 0.0
                    || duration > MAX_RECORDING_RECOVERY_SECONDS
            })
        {
            return Err("The recording session contains an invalid part.".into());
        }
        reject_reparse(&directory.join(&part.directory))?;
        crate::validate_owned_path(directory, &directory.join(&part.directory))?;
    }
    Ok(journal)
}

pub(crate) fn initial_failed_part(directory: &Path) -> Result<PathBuf, String> {
    let journal = read_journal(directory)?;
    let value: serde_json::Value = serde_json::from_reader(
        File::open(directory.join(JOURNAL_NAME))
            .map_err(|error| error.to_string())?
            .take(MAX_JOURNAL_BYTES + 1),
    )
    .map_err(|_| "The initial session journal is malformed.")?;
    if value.get("dimensions") != Some(&serde_json::Value::Null)
        || value
            .get("parts")
            .and_then(|parts| parts.get(0))
            .and_then(|part| part.get("duration"))
            != Some(&serde_json::Value::Null)
    {
        return Err("The initial session journal is incomplete.".into());
    }
    if journal.dimensions.is_some()
        || journal.parts.len() != 1
        || journal.parts[0].duration.is_some()
    {
        return Err("The session has recorded or uncertain screen history.".into());
    }
    Ok(directory.join(&journal.parts[0].directory))
}

pub fn recover_session(directory: &Path, ffmpeg: &Path) -> Result<Vec<CaptureResult>, String> {
    recover_with(directory, |path| crate::recover_part(path, ffmpeg))
}

fn recover_with(
    directory: &Path,
    mut recover: impl FnMut(&Path) -> Result<CaptureResult, crate::media::RecoveryError>,
) -> Result<Vec<CaptureResult>, String> {
    let directory = crate::canonical_media_directory(directory)?;
    let journal = read_journal(&directory)?;
    for part in &journal.parts {
        crate::cleanup::ensure_recovery_ready(&directory.join(&part.directory))?;
    }
    let mut results = Vec::new();
    let mut warnings = Vec::new();
    let mut dimensions = journal.dimensions;
    let mut duration = 0.0;
    for part in &journal.parts {
        match recover(&directory.join(&part.directory)) {
            Ok(result) => {
                if dimensions
                    .is_some_and(|expected| expected != (result.width, result.height, result.fps))
                {
                    warnings.push(format!("{} has different dimensions and was omitted from the recovered session. Its original recording files have been preserved.",part.directory));
                    continue;
                }
                if !result.duration.is_finite()
                    || result.duration <= 0.0
                    || duration + result.duration > MAX_RECORDING_RECOVERY_SECONDS
                {
                    return Err("The recovered recording exceeds the supported duration. Original recording parts are preserved.".into());
                }
                dimensions = Some((result.width, result.height, result.fps));
                duration += result.duration;
                if let Some(warning) = part_warning(&directory, &part.directory)? {
                    warnings.push(warning);
                }
                results.push(result);
            }
            Err(crate::media::RecoveryError::Unusable(error)) => warnings.push(format!(
                "{} could not be recovered: {error}. Its original files have been preserved.",
                part.directory
            )),
            Err(crate::media::RecoveryError::Retry(error)) => {
                return Err(format!("Recovery could not finish {}: {error}. Original recording parts are preserved.", part.directory));
            }
        }
    }
    if results.is_empty() {
        return Err(format!(
            "No recording parts could be recovered. {}",
            warnings.join(" ")
        ));
    }
    publish_warnings(&directory, &warnings)?;
    Ok(results)
}

fn part_warning(directory: &Path, part: &str) -> Result<Option<String>, String> {
    let warning_path = directory.join(part).join("recovery-warning.txt");
    reject_reparse(&warning_path)?;
    if let Ok(file) = File::open(warning_path) {
        let mut text = String::new();
        let _ = file.take(16 * 1024).read_to_string(&mut text);
        if !text.trim().is_empty() {
            return Ok(Some(format!("{part}: {text}")));
        }
    }
    Ok(None)
}

fn publish_warnings(directory: &Path, warnings: &[String]) -> Result<(), String> {
    if !warnings.is_empty() {
        let warning_path = directory.join("recovery-warning.txt");
        reject_reparse(&warning_path)?;
        fs::write(warning_path, warnings.join("\n")).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakePart {
        directory: PathBuf,
        active: Arc<AtomicUsize>,
        milliseconds: Arc<AtomicUsize>,
        fail_stop: bool,
    }
    impl Drop for FakePart {
        fn drop(&mut self) {
            self.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl Part for FakePart {
        fn elapsed(&self) -> f64 {
            self.milliseconds.load(Ordering::SeqCst) as f64 / 1000.0
        }
        fn dimensions(&self) -> (u32, u32, u32) {
            (320, 180, 30)
        }
        fn health(&self) -> Result<(), String> {
            Ok(())
        }
        fn snapshot(&self) -> PreviewSnapshot {
            PreviewSnapshot {
                sequence: 3,
                jpeg: Some(vec![1, 2, 3]),
                active: true,
                error: None,
            }
        }
        fn finish(self: Box<Self>) -> Result<CaptureResult, String> {
            if self.fail_stop {
                Err("injected finalize failure".into())
            } else {
                Ok(result(&self.directory, self.elapsed()))
            }
        }
    }
    fn result(path: &Path, duration: f64) -> CaptureResult {
        CaptureResult {
            screen_path: path.join("screen.mp4"),
            microphone_path: None,
            system_audio_path: None,
            camera_path: None,
            camera_offset_seconds: 0.0,
            width: 320,
            height: 180,
            fps: 30,
            duration,
            cursor: vec![screenrecord_core::CursorSample {
                time: 0.25,
                x: 0.5,
                y: 0.5,
                pressed: true,
                buttons: 1,
                visible: true,
            }],
        }
    }
    fn options() -> RecordingOptions {
        RecordingOptions {
            source_id: "monitor:1".into(),
            fps: 30,
            microphone_id: None,
            system_audio: false,
            system_audio_scope: Default::default(),
            region: None,
            camera_id: None,
        }
    }
    fn opener(
        active: Arc<AtomicUsize>,
        milliseconds: Arc<AtomicUsize>,
        fail_stop: bool,
        fail_second: bool,
    ) -> Arc<Opener> {
        let opened = AtomicUsize::new(0);
        Arc::new(move |directory, _, _, expected| {
            let journal = read_journal(directory.parent().unwrap()).unwrap();
            assert_eq!(
                journal.parts.last().unwrap().directory,
                directory.file_name().unwrap().to_str().unwrap()
            );
            let index = opened.fetch_add(1, Ordering::SeqCst);
            if index > 0 {
                assert_eq!(expected, Some((320, 180, 30)));
            }
            if fail_second && index > 0 {
                return Err("selected source disconnected".into());
            }
            assert_eq!(
                active.fetch_add(1, Ordering::SeqCst),
                0,
                "A resumed part must not overlap acquisition."
            );
            fs::create_dir(directory).unwrap();
            fs::write(directory.join("preserved.raw"), b"recoverable part").unwrap();
            Ok(Box::new(FakePart {
                directory: directory.to_owned(),
                active: active.clone(),
                milliseconds: milliseconds.clone(),
                fail_stop,
            }))
        })
    }

    #[test]
    fn cancelled_trial_start_creates_no_files_or_native_resources() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("never-created");
        let cancel = Arc::new(AtomicBool::new(true));
        let result = RecordingSession::start_cancellable(
            &directory,
            &options(),
            Path::new("ffmpeg"),
            cancel.clone(),
        );
        assert_eq!(result.err().unwrap(), crate::startup::CANCELLED);
        assert!(!directory.exists());
        assert_eq!(
            Recorder::start_cancellable(&directory, &options(), Path::new("ffmpeg"), cancel)
                .err()
                .unwrap(),
            crate::startup::CANCELLED
        );
        assert!(!directory.exists());
    }

    #[test]
    fn cancellation_during_part_start_drops_late_resource_and_preserves_exact_error() {
        let temporary = tempfile::tempdir().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let late_cancel = cancelled.clone();
        let late_opener = opener(
            active.clone(),
            Arc::new(AtomicUsize::new(100)),
            false,
            false,
        );
        let result = RecordingSession::start_cancellable_with(
            temporary.path(),
            &options(),
            Path::new("ffmpeg"),
            cancelled,
            Arc::new(move |directory, options, ffmpeg, expected| {
                let resource = late_opener(directory, options, ffmpeg, expected)?;
                late_cancel.store(true, Ordering::Release);
                Ok(resource)
            }),
        );
        assert_eq!(result.err().unwrap(), crate::startup::CANCELLED);
        assert_eq!(active.load(Ordering::Acquire), 0);
        assert_eq!(
            fs::read(temporary.path().join("part-00000/preserved.raw")).unwrap(),
            b"recoverable part"
        );
        assert_eq!(read_journal(temporary.path()).unwrap().dimensions, None);
    }

    #[test]
    fn ordinary_start_failure_does_not_mark_the_trial_cancelled() {
        let temporary = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let result = RecordingSession::start_cancellable_with(
            temporary.path(),
            &options(),
            Path::new("ffmpeg"),
            cancel.clone(),
            Arc::new(|_, _, _, _| Err("injected device unavailable".into())),
        );
        assert_eq!(result.err().unwrap(), "injected device unavailable");
        assert!(!cancel.load(Ordering::Acquire));
    }

    #[test]
    fn pause_releases_all_acquisition_freezes_elapsed_and_preserves_part_local_times() {
        let temporary = tempfile::tempdir().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let clock = Arc::new(AtomicUsize::new(1250));
        let mut session = RecordingSession::start_with(
            temporary.path(),
            &options(),
            Path::new("ffmpeg"),
            opener(active.clone(), clock.clone(), false, false),
        )
        .unwrap();
        assert_eq!(session.generation(), 1);
        assert!(session.preview_snapshot().active);
        session.pause().unwrap();
        assert_eq!(active.load(Ordering::SeqCst), 0);
        clock.store(5000, Ordering::SeqCst);
        assert_eq!(session.elapsed(), 1.25);
        assert!(session.is_paused());
        assert!(session.preview_snapshot().jpeg.is_none());
        session.pause().unwrap();
        clock.store(2500, Ordering::SeqCst);
        session.resume().unwrap();
        assert_eq!(session.generation(), 2);
        assert_eq!(session.elapsed(), 3.75);
        let parts = session.stop().unwrap();
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(
            parts.iter().map(|part| part.duration).collect::<Vec<_>>(),
            vec![1.25, 2.5]
        );
        assert_eq!(parts[1].cursor[0].time, 0.25);
        assert!(parts[0].screen_path.ends_with("part-00000/screen.mp4"));
        assert!(parts[1].screen_path.ends_with("part-00001/screen.mp4"));
    }

    #[test]
    fn failed_finalize_blocks_resume_and_keeps_original_part_for_recovery() {
        let temporary = tempfile::tempdir().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let clock = Arc::new(AtomicUsize::new(1200));
        let mut session = RecordingSession::start_with(
            temporary.path(),
            &options(),
            Path::new("ffmpeg"),
            opener(active.clone(), clock, true, false),
        )
        .unwrap();
        assert!(session.pause().unwrap_err().contains("finalize failure"));
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert!(session.resume().unwrap_err().contains("needs recovery"));
        assert!(session.stop().is_err());
        assert!(temporary.path().join("part-00000/preserved.raw").is_file());
        let recovered = recover_with(temporary.path(), |path| Ok(result(path, 1.2))).unwrap();
        assert_eq!(recovered.len(), 1);
    }

    #[test]
    fn failed_resume_is_journaled_and_recovery_warns_without_losing_saved_parts() {
        let temporary = tempfile::tempdir().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let clock = Arc::new(AtomicUsize::new(2000));
        let mut session = RecordingSession::start_with(
            temporary.path(),
            &options(),
            Path::new("ffmpeg"),
            opener(active.clone(), clock, false, true),
        )
        .unwrap();
        session.pause().unwrap();
        assert!(session.resume().unwrap_err().contains("disconnected"));
        assert!(session.is_paused());
        assert_eq!(session.elapsed(), 2.0);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(read_journal(temporary.path()).unwrap().parts.len(), 2);
        drop(session);
        let parts = recover_with(temporary.path(), |path| {
            if path.is_dir() {
                Ok(result(path, 2.0))
            } else {
                Err(crate::media::RecoveryError::Unusable(
                    "no first frame".into(),
                ))
            }
        })
        .unwrap();
        assert_eq!(parts.len(), 1);
        assert!(
            fs::read_to_string(temporary.path().join("recovery-warning.txt"))
                .unwrap()
                .contains("part-00001")
        );
    }

    #[test]
    fn drop_releases_active_part_and_recovery_rejects_traversal_before_dispatch() {
        let temporary = tempfile::tempdir().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let clock = Arc::new(AtomicUsize::new(1000));
        let session = RecordingSession::start_with(
            temporary.path(),
            &options(),
            Path::new("ffmpeg"),
            opener(active.clone(), clock, false, false),
        )
        .unwrap();
        drop(session);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        for name in [
            "../outside",
            "part-00001",
            "C:\\outside",
            "part-00000:stream",
        ] {
            fs::write(
                temporary.path().join(JOURNAL_NAME),
                serde_json::to_vec(&SessionJournal {
                    version: 1,
                    dimensions: Some((320, 180, 30)),
                    parts: vec![JournalPart {
                        directory: name.into(),
                        duration: None,
                    }],
                })
                .unwrap(),
            )
            .unwrap();
            assert!(
                recover_with(temporary.path(), |_| panic!("unsafe journal dispatched")).is_err()
            );
        }
    }

    #[test]
    fn part_limit_and_duration_limit_prevent_additional_acquisition() {
        let temporary = tempfile::tempdir().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let clock = Arc::new(AtomicUsize::new(1000));
        let mut session = RecordingSession::start_with(
            temporary.path(),
            &options(),
            Path::new("ffmpeg"),
            opener(active.clone(), clock, false, false),
        )
        .unwrap();
        session.pause().unwrap();
        session.results[0].duration = MAX_RECORDING_SECONDS;
        assert!(session.resume().unwrap_err().contains("eight hours"));
        session.results[0].duration = 1.0;
        while session.journal.parts.len() < MAX_PARTS {
            session.journal.parts.push(JournalPart {
                directory: format!("part-{:05}", session.journal.parts.len()),
                duration: None,
            });
        }
        assert!(session.resume().unwrap_err().contains("512"));
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn recovery_rejects_junction_parts_before_dispatch() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("untouched"), b"outside").unwrap();
        let junction = directory.path().join("part-00000");
        let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut child=crate::process::command(&powershell)
            .env("SCREENRECORD_TEST_JUNCTION",&junction)
            .env("SCREENRECORD_TEST_TARGET",outside.path())
            .args(["-NoProfile","-NonInteractive","-Command","$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:SCREENRECORD_TEST_JUNCTION -Target $env:SCREENRECORD_TEST_TARGET | Out-Null"])
            .spawn().unwrap();
        assert!(
            crate::process::wait_bounded(&mut child, std::time::Duration::from_secs(10))
                .unwrap()
                .success()
        );
        fs::write(directory.path().join(JOURNAL_NAME),br#"{"version":1,"dimensions":[320,180,30],"parts":[{"directory":"part-00000","duration":1.0}]}"#).unwrap();
        assert!(
            recover_with(directory.path(), |_| panic!("junction dispatched"))
                .err()
                .unwrap()
                .contains("junction")
        );
        fs::remove_dir(&junction).unwrap();
        assert_eq!(
            fs::read(outside.path().join("untouched")).unwrap(),
            b"outside"
        );
    }

    #[test]
    fn recovery_accepts_stop_poll_overshoot_but_enforces_the_shared_total_limit() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(JOURNAL_NAME),br#"{"version":1,"dimensions":[320,180,30],"parts":[{"directory":"part-00000","duration":null}]}"#).unwrap();
        let recovered = recover_with(directory.path(), |path| {
            Ok(result(path, MAX_RECORDING_SECONDS + 0.5))
        })
        .unwrap();
        assert_eq!(recovered[0].duration, MAX_RECORDING_SECONDS + 0.5);
        assert!(recover_with(directory.path(), |path| Ok(result(
            path,
            MAX_RECORDING_RECOVERY_SECONDS + 0.01
        )))
        .is_err());
        fs::write(
            directory.path().join(JOURNAL_NAME),
            serde_json::to_vec(&SessionJournal {
                version: 1,
                dimensions: Some((320, 180, 30)),
                parts: vec![
                    JournalPart {
                        directory: "part-00000".into(),
                        duration: Some(MAX_RECORDING_SECONDS),
                    },
                    JournalPart {
                        directory: "part-00001".into(),
                        duration: Some(61.0),
                    },
                ],
            })
            .unwrap(),
        )
        .unwrap();
        assert!(
            recover_with(directory.path(), |_| panic!("oversized journal dispatched")).is_err()
        );
    }

    #[test]
    fn changed_geometry_recovery_warns_and_preserves_compatible_parts() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(JOURNAL_NAME),br#"{"version":1,"dimensions":[320,180,30],"parts":[{"directory":"part-00000","duration":1.0},{"directory":"part-00001","duration":null}]}"#).unwrap();
        let results = recover_with(directory.path(), |path| {
            let mut part = result(path, 1.0);
            if path.ends_with("part-00001") {
                part.width = 640;
            }
            Ok(part)
        })
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            fs::read_to_string(directory.path().join("recovery-warning.txt"))
                .unwrap()
                .contains("different dimensions")
        );
    }
}
