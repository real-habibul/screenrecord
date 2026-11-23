use chrono::Local;
use screenrecord_capture::{session::RecordingSession, CaptureResult};
use screenrecord_core::*;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
#[path = "editor_commands.rs"]
mod editor_commands;
use editor_commands::*;
#[path = "camera_commands.rs"]
mod camera_commands;
use camera_commands::*;
#[path = "audio_commands.rs"]
mod audio_commands;
use audio_commands::*;
#[path = "recording_controls.rs"]
mod recording_controls;
use recording_controls::*;
#[path = "recording_commands.rs"]
mod recording_commands;
use recording_commands::*;
#[path = "recording_test_commands.rs"]
mod recording_test_commands;
use recording_test_commands::*;
#[path = "preset_commands.rs"]
mod preset_commands;
use preset_commands::*;
#[path = "local_commands.rs"]
mod local_commands;
use local_commands::*;
#[path = "tracking_commands.rs"]
mod tracking_commands;
use tracking_commands::*;

struct RecordingSlot {
    recorder: RecordingSession,
    pending: PendingProject,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingProject {
    id: String,
    name: String,
    created_at: String,
    options: RecordingOptions,
}
struct AppState {
    _process_job: Option<crate::process_job::ProcessJob>,
    root: PathBuf,
    settings_root: PathBuf,
    preset_write: Mutex<()>,
    local_transcription_root: PathBuf,
    local_install_cancel: Mutex<Option<(String, Arc<AtomicBool>)>>,
    local_install_progress: Mutex<Option<LocalInstallProgress>>,
    mask_tracking_cancel: Mutex<Option<(String, Arc<AtomicBool>)>>,
    mask_tracking_progress: Mutex<Option<MaskTrackingProgress>>,
    ffmpeg: Option<PathBuf>,
    recording: Mutex<Option<RecordingSlot>>,
    recording_status: Mutex<RecordingState>,
    recording_busy: AtomicBool,
    test_cache: PathBuf,
    trials: Mutex<crate::recording_test_store::TrialState>,
    recovery_busy: AtomicBool,
    export_busy: AtomicBool,
    export_cancel: Arc<AtomicBool>,
    export_progress: Mutex<Option<ExportProgress>>,
    editor_busy: AtomicBool,
    transcription_cancel: Mutex<Option<(String, Arc<AtomicBool>)>>,
    transcription_progress: Mutex<Option<TranscriptionProgress>>,
    preview_write: Mutex<()>,
    preview_cancel: Mutex<Arc<AtomicBool>>,
    camera_preview: Mutex<CameraPreviewState>,
    known_devices: Mutex<crate::device_catalog::DeviceCatalog>,
    microphone_meter: Mutex<MicrophoneMeterState>,
    audio_analysis: Mutex<AnalysisState>,
    project_write: Mutex<()>,
    operation: Mutex<()>,
    shortcut: AtomicBool,
    recovery_issues: Mutex<Vec<String>>,
}
struct BusyReset<'a>(&'a AtomicBool);
impl Drop for BusyReset<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
fn ffmpeg(state: &AppState) -> Result<&Path, String> {
    state.ffmpeg.as_deref().ok_or_else(|| {
        "The media engine is missing. Reinstall Screenrecord using the complete installer.".into()
    })
}
fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
fn busy(state: &AppState) -> bool {
    state.recovery_busy.load(Ordering::Acquire)
        || state.recording_busy.load(Ordering::Acquire)
        || state.export_busy.load(Ordering::Acquire)
        || state.editor_busy.load(Ordering::Acquire)
        || state
            .recording
            .try_lock()
            .map(|s| s.is_some())
            .unwrap_or(true)
}
fn find_ffmpeg(app: &tauri::App) -> Option<PathBuf> {
    if let Ok(root) = app.path().resource_dir() {
        for path in [root.join("resources/ffmpeg.exe"), root.join("ffmpeg.exe")] {
            if path.is_file() && path.with_file_name("ffprobe.exe").is_file() {
                return Some(path);
            }
        }
    }
    if cfg!(debug_assertions) {
        which::which("ffmpeg").ok()
    } else {
        None
    }
}

#[tauri::command]
fn get_app_info(state: tauri::State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        projects_directory: state.root.to_string_lossy().into_owned(),
        ffmpeg_available: state.ffmpeg.is_some(),
        ffmpeg_path: state
            .ffmpeg
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
    }
}
#[tauri::command]
fn get_stop_shortcut(state: tauri::State<'_, AppState>) -> Option<String> {
    state
        .shortcut
        .load(Ordering::Acquire)
        .then(|| "Ctrl+Shift+R".into())
}
#[tauri::command]
fn get_recovery_issues(state: tauri::State<'_, AppState>) -> Vec<String> {
    state
        .recovery_issues
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default()
}
#[tauri::command]
async fn close_app(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state.operation.try_lock().map_err(|_| {
            "A media operation is still running. Wait for it to finish before closing.".to_owned()
        })?;
        if busy(&state) {
            return Err(
                "Wait for the current media operation, stop recording, or cancel export, analysis, or subtitles before closing.".into(),
            );
        }
        camera_commands::stop_all(&state)?;
        audio_commands::stop_meter(&state)?;
        drop(state.trials.lock().map_err(|e| e.to_string())?.take_ready());
        app.exit(0);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn list_capture_devices(app: tauri::AppHandle) -> Result<CaptureDevices, String> {
    let generation = app
        .state::<AppState>()
        .known_devices
        .lock()
        .map_err(|e| e.to_string())?
        .reserve()?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let devices = screenrecord_capture::list_devices(ffmpeg(&state)?)?;
        state
            .known_devices
            .lock()
            .map_err(|e| e.to_string())?
            .publish(generation, &devices)?;
        Ok(devices)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn list_projects(app: tauri::AppHandle) -> Result<Vec<ProjectSummary>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut projects = vec![];
        for entry in fs::read_dir(&state.root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if validate_id(&id).is_err() || !entry.path().join("project.json").is_file() {
                continue;
            }
            match load_project_summary(&state.root, &id) {
                Ok(summary) => projects.push(summary),
                Err(error) => log::warn!("Project {id} cannot be loaded: {error}"),
            }
        }
        projects.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(projects)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn open_project(app: tauri::AppHandle, id: String) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || load_project(&app.state::<AppState>().root, &id))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn delete_project(app: tauri::AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state.operation.lock().map_err(|e| e.to_string())?;
        if busy(&state) {
            return Err(
                "Stop recording or wait for the current media operation before deleting a project."
                    .into(),
            );
        }
        state.editor_busy.store(true, Ordering::Release);
        let _busy = BusyReset(&state.editor_busy);
        let _write = state.project_write.lock().map_err(|e| e.to_string())?;
        state
            .preview_cancel
            .lock()
            .map_err(|e| e.to_string())?
            .store(true, Ordering::Release);
        let _preview = state.preview_write.lock().map_err(|e| e.to_string())?;
        if screenrecord_capture::cleanup_in_progress() {
            return Err(
                "Recording cleanup is still releasing files. Wait before deleting a project."
                    .into(),
            );
        }
        crate::project_delete::recycle_project(&state.root, &id)
    })
    .await
    .map_err(|_| {
        "Project deletion worker failed. Refresh projects and check the folder before retrying."
            .to_string()
    })?
}
#[tauri::command]
async fn save_project(app: tauri::AppHandle, project: Project) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.project_write.lock().map_err(|e| e.to_string())?;
        screenrecord_core::save_project_edits(
            &state.root,
            &project.id,
            &project.name,
            project.edits,
        )?;
        load_project(&state.root, &project.id)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn save_project_edits(
    app: tauri::AppHandle,
    id: String,
    name: String,
    edits: EditSettings,
) -> Result<SavedEdits, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.project_write.lock().map_err(|e| e.to_string())?;
        screenrecord_core::save_project_edits(&state.root, &id, &name, edits)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn rename_project(
    app: tauri::AppHandle,
    id: String,
    name: String,
) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.project_write.lock().map_err(|e| e.to_string())?;
        validate_name(&name)?;
        let mut project = load_project(&state.root, &id)?;
        project.name = name.trim().to_owned();
        screenrecord_core::save_project_edits(
            &state.root,
            &id,
            &project.name,
            project.edits.clone(),
        )?;
        Ok(project)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn regenerate_zooms(app: tauri::AppHandle, id: String) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.project_write.lock().map_err(|e| e.to_string())?;
        let mut project = load_project(&state.root, &id)?;
        project.edits.zooms = screenrecord_render::auto_zoom(&project.cursor, project.duration);
        screenrecord_core::save_project_edits(
            &state.root,
            &id,
            &project.name,
            project.edits.clone(),
        )?;
        Ok(project)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn start_recording(
    app: tauri::AppHandle,
    options: RecordingOptions,
) -> Result<RecordingState, String> {
    options.validate()?;
    let result = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || {
            let state = app.state::<AppState>();
            let _operation = state.operation.lock().map_err(|e| e.to_string())?;
            if state.recovery_busy.load(Ordering::Acquire) {
                return Err(
                    "Project recovery is still running. Try recording again shortly.".into(),
                );
            }
            if state.recording_busy.swap(true, Ordering::AcqRel) {
                return Err("A recording operation is already in progress.".into());
            }
            let _busy = BusyReset(&state.recording_busy);
            if state.export_busy.load(Ordering::Acquire)
                || state.editor_busy.load(Ordering::Acquire)
            {
                return Err(
                    "Wait for export or audio processing to finish before recording.".into(),
                );
            }
            if state.recording.lock().map_err(|e| e.to_string())?.is_some() {
                return Err("A recording is already running.".into());
            }
            screenrecord_capture::validate_recording_source(&options)?;
            camera_commands::stop_all(&state)?;
            audio_commands::stop_meter(&state)?;
            let engine = ffmpeg(&state)?;
            let mut ui = recording_controls::StartUiGuard::new(app.clone());
            let controls = recording_controls::prepare(&app, options.camera_id.is_some())?;
            if let Some(main) = app.get_webview_window("main") {
                recording_controls::protect(&main, true)?;
                main.minimize().map_err(|e| e.to_string())?;
            }
            let pending = PendingProject {
                id: uuid::Uuid::new_v4().to_string(),
                name: format!("Recording {}", Local::now().format("%Y-%m-%d %H.%M.%S")),
                created_at: Local::now().to_rfc3339(),
                options: options.clone(),
            };
            let directory = project_directory(&state.root, &pending.id)?;
            create_pending_project(&directory, &pending)?;
            let recorder = match RecordingSession::start(&directory, &options, engine) {
                Ok(recorder) => recorder,
                Err(error) => {
                    log::error!("Recording {} failed to start: {error}", pending.id);
                    if let Err(reason) = crate::recording_failure::write(&state.root, &pending.id, "start", &error, None) {
                        log::warn!("Cannot save recording failure context: {reason}");
                    }
                    match crate::startup_failure::retire(&state.root, &pending.id, &error) {
                        Ok(true) => log::info!("Archived failed screen startup {}; other recording files preserved", pending.id),
                        Ok(false) => log::info!("Recording {} remains available for recovery", pending.id),
                        Err(reason) => log::warn!("Recording {} retains its recovery marker: {reason}", pending.id),
                    }
                    return Err(error);
                }
            };
            let result = RecordingState {
                active: true,
                paused: false,
                camera_available: options.camera_id.is_some(),
                phase: "recording".into(),
                elapsed: recorder.elapsed(),
                project_id: Some(pending.id.clone()),
            };
            *state.recording.lock().map_err(|e| e.to_string())? =
                Some(RecordingSlot { recorder, pending });
            if let Err(error) = controls.show() {
                let cleanup_error = state.recording.lock().map_err(|e| e.to_string())?.take()
                    .and_then(|slot| slot.recorder.stop().err());
                let reason = format!("Recording controls could not be shown. Available media is preserved for recovery: {error}{}", cleanup_error.map(|error| format!(" Cleanup: {error}")).unwrap_or_default());
                if let Err(error) = crate::recording_failure::write(&state.root, result.project_id.as_deref().unwrap(), "controls", &reason, None) {
                    log::warn!("Cannot save recording controls failure: {error}");
                }
                return Err(reason);
            }
            recording_commands::remember_state(&app, result.clone());
            ui.commit();
            recording_commands::watch_health(app.clone(), result.project_id.clone().unwrap());
            log::info!("Recording started");
            Ok(result)
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    result
}
#[tauri::command]
async fn get_recording_state(app: tauri::AppHandle) -> Result<RecordingState, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let slot = state.recording.lock().map_err(|e| e.to_string())?;
        match slot.as_ref() {
            Some(slot) => {
                slot.recorder.health()?;
                Ok(recording_commands::state_for_slot(slot))
            }
            None => Ok(state
                .recording_status
                .lock()
                .map_err(|e| e.to_string())?
                .clone()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
fn create_pending_project(directory: &Path, pending: &PendingProject) -> Result<(), String> {
    let parent = directory.parent().ok_or("Recording library is missing.")?;
    let mut marker = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut marker, pending).map_err(|e| e.to_string())?;
    marker.as_file().sync_all().map_err(|e| e.to_string())?;
    fs::create_dir(directory).map_err(|e| format!("Cannot create recording folder: {e}"))?;
    if let Err(error) = marker.persist_noclobber(directory.join("pending.json")) {
        let _ = fs::remove_dir(directory);
        return Err(format!(
            "Cannot save the recording recovery marker: {error}"
        ));
    }
    Ok(())
}

fn finish_project(
    root: &Path,
    pending: &PendingProject,
    result: CaptureResult,
    status: &str,
) -> Result<Project, String> {
    finish_project_with_intervals(root, pending, result, None, status)
}
fn finish_project_with_intervals(
    root: &Path,
    pending: &PendingProject,
    result: CaptureResult,
    camera_intervals: Option<Vec<CameraInterval>>,
    status: &str,
) -> Result<Project, String> {
    let directory = project_directory(root, &pending.id)?;
    let duration = result.duration;
    let mut edits = EditSettings::new(duration);
    edits.zooms = screenrecord_render::auto_zoom(&result.cursor, duration);
    edits.camera.enabled = result.camera_path.is_some();
    let project = Project {
        version: PROJECT_VERSION,
        id: pending.id.clone(),
        name: pending.name.clone(),
        created_at: pending.created_at.clone(),
        directory: directory.to_string_lossy().into_owned(),
        screen_path: result.screen_path.to_string_lossy().into_owned(),
        microphone_path: result
            .microphone_path
            .map(|p| p.to_string_lossy().into_owned()),
        system_audio_path: result
            .system_audio_path
            .map(|p| p.to_string_lossy().into_owned()),
        camera_path: result.camera_path.map(|p| p.to_string_lossy().into_owned()),
        camera_offset_seconds: result.camera_offset_seconds,
        camera_intervals,
        width: result.width,
        height: result.height,
        fps: result.fps,
        duration,
        cursor: result.cursor,
        edits,
        status: status.into(),
    };
    save_project_atomic(&project)?;
    if let Err(error) = fs::remove_file(directory.join("pending.json")) {
        log::warn!("Project saved; recovery marker remains: {error}");
    }
    Ok(project)
}
fn stop_internal(
    app: &tauri::AppHandle,
    expected_project: Option<&str>,
) -> Result<Project, String> {
    let state = app.state::<AppState>();
    let _operation = state.operation.lock().map_err(|e| e.to_string())?;
    if state.recording_busy.swap(true, Ordering::AcqRel) {
        return Err("A recording operation is already in progress.".into());
    }
    let _busy = BusyReset(&state.recording_busy);
    let slot = {
        let mut current = state.recording.lock().map_err(|e| e.to_string())?;
        recording_commands::take_slot(&mut current, expected_project)?
    };
    let mut finalizing = recording_commands::state_for_slot(&slot);
    finalizing.phase = "finalizing".into();
    finalizing.paused = false;
    recording_commands::remember_state(app, finalizing);
    let pending = slot.pending;
    let result = (|| {
        let project = match slot
            .recorder
            .stop()
            .and_then(|parts| recording_commands::finish_session(app, &pending, parts, "ready"))
        {
            Ok(project) => {
                report_recovery_file_warning(app, Path::new(&project.directory));
                project
            }
            Err(original_error) => {
                let recovered = (|| {
                    let directory = validated_recovery_directory(&state.root, &pending.id)?;
                    let project = recording_commands::recover_recording(app, &pending, &directory)?;
                    report_recovery_file_warning(app, &directory);
                    Ok::<_, String>(project)
                })();
                let recovery_error = recovered.as_ref().err().map(String::as_str);
                if let Err(reason) = crate::recording_failure::write(
                    &state.root,
                    &pending.id,
                    "stop",
                    &original_error,
                    recovery_error,
                ) {
                    log::warn!("Cannot save recording failure context: {reason}");
                }
                match recovered {
                Ok(project)=>{report_recovery_issue(app,format!("{} was recovered after a device or save error: {original_error}",pending.name));project},
                Err(recovery_error)=>return Err(format!("Recording stopped with an error: {original_error} Recovery could not finish: {recovery_error}. Available media is preserved in {}.",state.root.join(&pending.id).display())),
            }
            }
        };
        log::info!("Recording saved");
        Ok(project)
    })();
    if let Ok(mut status) = state.recording_status.lock() {
        *status = RecordingState::default();
    }
    recording_controls::hide(app);
    show_main(app);
    drop(_busy);
    drop(_operation);
    match &result {
        Ok(project) => {
            let _ = app.emit("recording-stopped", project);
        }
        Err(error) => {
            let _ = app.emit(
                "recording-error",
                serde_json::json!({ "projectId": pending.id, "error": error }),
            );
        }
    }
    result
}
#[tauri::command]
async fn stop_recording(
    app: tauri::AppHandle,
    project_id: Option<String>,
) -> Result<Project, String> {
    if let Some(id) = &project_id {
        validate_id(id)?;
    }
    let result = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || stop_internal(&app, project_id.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?;
    result
}
fn stop_from_system(app: &tauri::AppHandle, project_id: Option<String>) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = stop_internal(&app, project_id.as_deref());
    });
}
fn publish_progress(app: &tauri::AppHandle, progress: ExportProgress) {
    if let Ok(mut current) = app.state::<AppState>().export_progress.lock() {
        *current = Some(progress.clone());
    }
    let _ = app.emit("export-progress", progress);
}
#[tauri::command]
async fn export_project(
    app: tauri::AppHandle,
    id: String,
    options: ExportOptions,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || start_export(app, id, options))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn get_whatsapp_export_plan(
    app: tauri::AppHandle,
    id: String,
) -> Result<WhatsappExportPlan, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let project = load_project_metadata(&state.root, &id)?;
        whatsapp_export_plan(&project)
    })
    .await
    .map_err(|error| error.to_string())?
}
fn start_export(app: tauri::AppHandle, id: String, options: ExportOptions) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _operation = state.operation.lock().map_err(|e| e.to_string())?;
    if state.editor_busy.load(Ordering::Acquire) {
        return Err("Wait for the current media operation or cancel it before exporting.".into());
    }
    if state.recovery_busy.load(Ordering::Acquire) {
        return Err("Project recovery is still running. Try exporting again shortly.".into());
    }
    if state.recording_busy.load(Ordering::Acquire)
        || state.recording.lock().map_err(|e| e.to_string())?.is_some()
    {
        return Err("Stop recording before exporting.".into());
    }
    options.validate()?;
    let project = load_project(&state.root, &id)?;
    validate_mask_tracking_review(&project.edits, project.duration)?;
    if options.preset == ExportPreset::Whatsapp {
        whatsapp_export_plan(&project)?;
    }
    let engine = ffmpeg(&state)?.to_path_buf();
    if state.export_busy.swap(true, Ordering::AcqRel) {
        return Err("An export is already running.".into());
    }
    state.export_cancel.store(false, Ordering::Release);
    let cancel = state.export_cancel.clone();
    publish_progress(
        &app,
        ExportProgress {
            state: "running".into(),
            progress: 0.0,
            output_path: options.output_path.clone(),
            error: None,
            output_bytes: None,
        },
    );
    drop(_operation);
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let app = worker_app;
        let state = app.state::<AppState>();
        let _busy = BusyReset(&state.export_busy);
        let reporter = app.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            screenrecord_render::export(&project, &options, &engine, cancel.clone(), move |p| {
                publish_progress(&reporter, p)
            })
        }));
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => Some("Export worker failed. Original media is preserved.".into()),
        };
        if let Some(error) = error {
            let cancelled = cancel.load(Ordering::Acquire);
            log::error!("Export ended: {error}");
            publish_progress(
                &app,
                ExportProgress {
                    state: if cancelled { "cancelled" } else { "failed" }.into(),
                    progress: 0.0,
                    output_path: options.output_path,
                    error: if cancelled { None } else { Some(error) },
                    output_bytes: None,
                },
            );
        }
    });
    Ok(())
}
#[tauri::command]
fn cancel_export(state: tauri::State<'_, AppState>) {
    state.export_cancel.store(true, Ordering::Release);
}
#[tauri::command]
fn get_export_progress(state: tauri::State<'_, AppState>) -> Option<ExportProgress> {
    state.export_progress.lock().ok().and_then(|p| p.clone())
}
#[tauri::command]
fn reveal_file(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    let path = PathBuf::from(path)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let in_library = path.starts_with(state.root.canonicalize().map_err(|e| e.to_string())?);
    let is_export = state
        .export_progress
        .lock()
        .ok()
        .and_then(|p| p.as_ref().map(|p| PathBuf::from(&p.output_path)))
        .and_then(|p| p.canonicalize().ok())
        .is_some_and(|p| p == path);
    if !in_library && !is_export {
        return Err("Only project media and the current export can be revealed.".into());
    }
    let mut command = Command::new("explorer.exe");
    if path.is_file() {
        command.arg(format!("/select,{}", path.display()));
    } else {
        command.arg(&path);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn().map_err(|e| e.to_string())?;
    Ok(())
}
fn report_recovery_issue(app: &tauri::AppHandle, issue: String) {
    log::warn!("{issue}");
    let state = app.state::<AppState>();
    if let Ok(mut issues) = state.recovery_issues.lock() {
        issues.push(issue);
    }
    let _ = app.emit("recovery-issues-updated", ());
}
fn report_recovery_file_warning(app: &tauri::AppHandle, directory: &Path) {
    let path = directory.join("recovery-warning.txt");
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() <= MAX_PENDING_PROJECT_BYTES) {
        if let Ok(warning) = fs::read_to_string(path) {
            report_recovery_issue(app, warning);
        }
    }
}
fn recover_projects(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let _busy = BusyReset(&state.recovery_busy);
    let Ok(_operation) = state.operation.lock() else {
        return;
    };
    if state.ffmpeg.is_none() {
        return;
    }
    let Ok(entries) = fs::read_dir(&state.root) else {
        return;
    };
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if validate_id(&id).is_err() {
            continue;
        }
        if state
            .recording
            .lock()
            .map(|slot| slot.as_ref().is_some_and(|slot| slot.pending.id == id))
            .unwrap_or(true)
        {
            continue;
        }
        let marker = entry.path().join("pending.json");
        if !marker.is_file() || entry.path().join("project.json").exists() {
            continue;
        }
        let result = (|| {
            let directory = validated_recovery_directory(&state.root, &id)?;
            let pending: PendingProject = serde_json::from_reader(
                fs::File::open(directory.join("pending.json")).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            validate_id(&pending.id)?;
            validate_name(&pending.name)?;
            pending.options.validate()?;
            if id != pending.id {
                return Err("Recovery project identity does not match its folder.".to_string());
            }
            match crate::startup_failure::retire(
                &state.root,
                &id,
                "Capture stopped before the first screen frame.",
            ) {
                Ok(true) => {
                    log::info!(
                        "Archived failed screen startup {id}; other recording files preserved"
                    );
                    return Ok(());
                }
                Ok(false) => {}
                Err(reason) => log::debug!("Recording {id} requires normal recovery: {reason}"),
            }
            recording_commands::recover_recording(app, &pending, &directory)?;
            report_recovery_file_warning(app, &directory);
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            report_recovery_issue(app, format!("{}: {error}", entry.path().display()));
        }
    }
    let _ = app.emit("library-updated", ());
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .max_file_size(2_000_000)
                .build(),
        )
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        let project_id = app.state::<AppState>().recording.try_lock().ok().and_then(|slot| slot.as_ref().map(|slot| slot.pending.id.clone()));
                        if let Some(project_id) = project_id { stop_from_system(app, Some(project_id)); }
                        else if let Ok(trials) = app.state::<AppState>().trials.lock() { trials.cancel_active(); }
                    }
                })
                .build(),
        )
        .setup(|app| {
            let root = app
                .path()
                .video_dir()
                .map(|p| p.join("Screenrecord"))
                .unwrap_or(app.path().app_local_data_dir()?.join("projects"));
            fs::create_dir_all(&root)?;
            let settings_root = app.path().app_local_data_dir().map(|directory| directory.join("settings")).unwrap_or_else(|_| root.join(".settings"));
            let local_transcription_root = settings_root.parent().unwrap_or(&root).join("local-transcription");
            let test_cache = app.path().app_local_data_dir()?.join("recording-tests");
            if let Err(error) = crate::recording_test_store::TrialDirectory::clean_previous_launch(&test_cache) {
                log::warn!("Cannot clean previous recording tests: {error}");
            }
            let ffmpeg = find_ffmpeg(app);
            let process_job = match crate::process_job::ProcessJob::attach() {
                Ok(job) => Some(job),
                Err(error) => {
                    log::warn!("Process cleanup job unavailable: {error}");
                    None
                }
            };
            app.manage(AppState {
                _process_job: process_job,
                root,
                settings_root,
                preset_write: Mutex::new(()),
                local_transcription_root,
                local_install_cancel: Mutex::new(None),
                local_install_progress: Mutex::new(None),
                mask_tracking_cancel: Mutex::new(None),
                mask_tracking_progress: Mutex::new(None),
                ffmpeg,
                recording: Mutex::new(None),
                recording_status: Mutex::new(RecordingState::default()),
                recording_busy: AtomicBool::new(false),
                test_cache,
                trials: Mutex::new(crate::recording_test_store::TrialState::default()),
                recovery_busy: AtomicBool::new(true),
                export_busy: AtomicBool::new(false),
                export_cancel: Arc::new(AtomicBool::new(false)),
                export_progress: Mutex::new(None),
                editor_busy: AtomicBool::new(false),
                transcription_cancel: Mutex::new(None),
                transcription_progress: Mutex::new(None),
                preview_write: Mutex::new(()),
                preview_cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
                camera_preview: Mutex::new(CameraPreviewState::default()),
                known_devices: Mutex::new(crate::device_catalog::DeviceCatalog::default()),
                microphone_meter: Mutex::new(MicrophoneMeterState::default()),
                audio_analysis: Mutex::new(AnalysisState::default()),
                project_write: Mutex::new(()),
                operation: Mutex::new(()),
                shortcut: AtomicBool::new(false),
                recovery_issues: Mutex::new(vec![]),
            });
            match app.global_shortcut().register("Ctrl+Shift+R") {
                Ok(()) => app
                    .state::<AppState>()
                    .shortcut
                    .store(true, Ordering::Release),
                Err(error) => log::warn!("Stop shortcut unavailable: {error}"),
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || recover_projects(&handle));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if busy(&window.state::<AppState>()) {
                    api.prevent_close();
                    let _ = window.emit(
                        "close-blocked",
                        "Wait for the current media operation, stop recording, or cancel export, analysis, subtitles, or mask tracking before closing.",
                    );
                }
            } else if matches!(event, tauri::WindowEvent::Destroyed) && window.label() == "main" {
                if let Ok(mut trials) = window.state::<AppState>().trials.lock() { trials.take_ready(); }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            get_local_transcription_status,
            start_local_transcription_install,
            start_mask_tracking,
            cancel_mask_tracking,
            get_mask_tracking_progress,
            cancel_local_transcription_install,
            get_local_transcription_install_progress,
            list_appearance_presets,
            save_appearance_preset,
            delete_appearance_preset,
            get_stop_shortcut,
            start_camera_preview,
            stop_camera_preview,
            get_camera_preview_frame,
            start_microphone_meter,
            stop_microphone_meter,
            get_microphone_meter,
            analyze_audio,
            cancel_audio_analysis,
            get_recovery_issues,
            close_app,
            list_capture_devices,
            list_projects,
            open_project,
            delete_project,
            save_project,
            save_project_edits,
            rename_project,
            regenerate_zooms,
            start_recording,
            get_source_thumbnail,
            run_recording_test,
            cancel_recording_test,
            discard_recording_test,
            get_recording_state,
            pause_recording,
            resume_recording,
            get_recording_camera_frame,
            restore_main_window,
            stop_recording,
            export_project,
            get_whatsapp_export_plan,
            cancel_export,
            get_export_progress,
            import_music,
            export_subtitles,
            import_subtitles,
            render_subtitle_preview,
            start_transcription,
            cancel_transcription,
            get_transcription_progress,
            reveal_file
        ])
        .run(tauri::generate_context!())
        .expect("Screenrecord could not start");
}

#[cfg(test)]
mod tests {
    use super::*;
    mod quota {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../testdata/quota.rs"));
    }

    #[test]
    fn recording_parts_publish_nested_media_and_camera_availability_before_removing_pending() {
        let root = tempfile::tempdir().unwrap();
        let (pending, mut captured) = fixture(root.path(), 3.0);
        let directory = project_directory(root.path(), &pending.id).unwrap();
        let part = directory.join("part-00000");
        fs::create_dir(&part).unwrap();
        let screen = part.join("screen.mp4");
        fs::rename(&captured.screen_path, &screen).unwrap();
        captured.screen_path = screen.clone();
        captured.camera_path = Some(screen);
        let project = finish_project_with_intervals(
            root.path(),
            &pending,
            captured,
            Some(vec![CameraInterval {
                start: 0.5,
                end: 1.5,
            }]),
            "ready",
        )
        .unwrap();
        let loaded = load_project(root.path(), &pending.id).unwrap();
        assert_eq!(
            dunce::canonicalize(&loaded.screen_path).unwrap(),
            dunce::canonicalize(&project.screen_path).unwrap()
        );
        assert!(camera_visible_at(&loaded, 0.5));
        assert!(!camera_visible_at(&loaded, 1.5));
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("project.json")).unwrap()).unwrap();
        assert_eq!(
            Path::new(stored["screenPath"].as_str().unwrap()),
            Path::new("part-00000").join("screen.mp4")
        );
        assert!(!directory.join("pending.json").exists());
    }

    fn fixture(root: &Path, duration: f64) -> (PendingProject, CaptureResult) {
        let pending = PendingProject {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Storage integration test".into(),
            created_at: "2026-11-11T00:00:00+07:00".into(),
            options: RecordingOptions {
                source_id: "test-only".into(),
                fps: 30,
                microphone_id: None,
                system_audio: false,
                system_audio_scope: Default::default(),
                region: None,
                camera_id: None,
            },
        };
        let directory = root.join(&pending.id);
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("pending.json"),
            serde_json::to_vec(&pending).unwrap(),
        )
        .unwrap();
        let screen_path = directory.join("screen.mp4");
        let camera_path = directory.join("camera.mp4");
        fs::write(&screen_path, b"storage-only fixture").unwrap();
        fs::write(&camera_path, b"storage-only fixture").unwrap();
        (
            pending,
            CaptureResult {
                screen_path,
                camera_path: Some(camera_path),
                camera_offset_seconds: 0.75,
                microphone_path: None,
                system_audio_path: None,
                width: 1920,
                height: 1080,
                fps: 30,
                duration,
                cursor: vec![],
            },
        )
    }

    #[test]
    fn finishing_publishes_portable_project_and_camera_timing_before_removing_pending_marker() {
        let root = tempfile::tempdir().unwrap();
        let (pending, captured) = fixture(root.path(), 3.0);
        let project = finish_project(root.path(), &pending, captured, "ready").unwrap();
        let reopened = load_project(root.path(), &project.id).unwrap();
        assert_eq!(reopened.camera_offset_seconds, 0.75);
        assert_eq!(reopened.version, PROJECT_VERSION);
        assert!(!root.path().join(&project.id).join("pending.json").exists());
        let disk: serde_json::Value = serde_json::from_slice(
            &fs::read(root.path().join(&project.id).join("project.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(disk["screenPath"], "screen.mp4");
        assert!(disk.get("cursorFile").is_some());
    }

    #[test]
    fn finalizing_a_capture_preserves_typed_clicks_and_editable_indicators() {
        let root = tempfile::tempdir().unwrap();
        let (pending, mut captured) = fixture(root.path(), 3.0);
        let clicks: serde_json::Value =
            serde_json::from_str(include_str!("../../testdata/cursor-clicks.json")).unwrap();
        captured.cursor = serde_json::from_value(clicks["samples"].clone()).unwrap();
        let finished = finish_project(root.path(), &pending, captured, "ready").unwrap();
        let mut reopened = load_project(root.path(), &finished.id).unwrap();
        assert!(reopened.edits.show_clicks);
        let edges: Vec<_> = cursor_clicks(&reopened.cursor).collect();
        assert_eq!(edges.len(), 7);
        assert_eq!(edges[0].button, ClickButton::Left);
        assert_eq!(edges[2].button, ClickButton::Right);
        assert_eq!(edges[3].button, ClickButton::Generic);
        assert_eq!(reopened.edits.zooms.len(), 1);
        reopened.edits.show_clicks = false;
        screenrecord_core::save_project_edits(
            root.path(),
            &finished.id,
            &finished.name,
            reopened.edits,
        )
        .unwrap();
        let edited = load_project(root.path(), &finished.id).unwrap();
        assert!(!edited.edits.show_clicks);
        assert_eq!(cursor_clicks(&edited.cursor).collect::<Vec<_>>(), edges);
    }

    #[test]
    fn failed_project_validation_preserves_recovery_marker_and_media() {
        let root = tempfile::tempdir().unwrap();
        let (pending, captured) = fixture(root.path(), 0.0);
        assert!(finish_project(root.path(), &pending, captured, "ready").is_err());
        let directory = root.path().join(&pending.id);
        assert!(directory.join("pending.json").is_file());
        assert!(directory.join("screen.mp4").is_file());
        assert!(!directory.join("project.json").exists());
    }

    fn fill_quota(directory: &Path) -> (tempfile::NamedTempFile, std::io::Error) {
        let mut filler = tempfile::NamedTempFile::new_in(directory).unwrap();
        for _ in 0..256 {
            if let Err(error) = filler
                .as_file_mut()
                .write_all(&[0x42; 4096])
                .and_then(|()| filler.as_file().sync_data())
            {
                quota::assert_volume_full();
                assert!(
                    matches!(error.raw_os_error(), Some(31 | 39 | 112)),
                    "{error:?}"
                );
                return (filler, error);
            }
        }
        panic!("Run only with the isolated 1 MiB quota");
    }

    #[test]
    #[ignore = "requires the isolated 1 MiB volume created by scripts/test-disk-full.ps1"]
    fn disk_full_preserves_capture_and_last_save_then_retries() {
        let volume = std::env::var_os("SCREENRECORD_TEST_QUOTA_ROOT").unwrap();
        let root = tempfile::tempdir_in(volume).unwrap();
        let (ready, captured) = fixture(root.path(), 8.0);
        finish_project(root.path(), &ready, captured, "ready").unwrap();
        let saved = load_project(root.path(), &ready.id).unwrap();
        let manifest = Path::new(&saved.directory).join("project.json");
        let original = fs::read(&manifest).unwrap();
        let (pending, captured) = fixture(root.path(), 8.0);
        let directory = root.path().join(&pending.id);
        let marker = fs::read(directory.join("pending.json")).unwrap();
        let media = fs::read(&captured.screen_path).unwrap();
        let capture_result = || CaptureResult {
            screen_path: captured.screen_path.clone(),
            camera_path: captured.camera_path.clone(),
            microphone_path: None,
            system_audio_path: None,
            cursor: (0..1024)
                .map(|i| CursorSample {
                    time: f64::from(i) / 128.0,
                    x: 0.5,
                    y: 0.5,
                    pressed: false,
                    visible: true,
                    buttons: 0,
                })
                .collect(),
            ..captured
        };
        let reserve = root.path().join("reserve");
        fs::write(&reserve, [0; 4096]).unwrap();
        let (filler, quota) = fill_quota(root.path());
        let new_recording = PendingProject {
            id: uuid::Uuid::new_v4().to_string(),
            ..pending.clone()
        };
        let new_dir = root.path().join(&new_recording.id);
        let start_error = create_pending_project(&new_dir, &new_recording).unwrap_err();
        assert!(start_error.contains(&quota.to_string()), "{start_error}");
        assert!(
            !new_dir.exists(),
            "failed start left an empty project in the library"
        );
        let error = screenrecord_core::save_project_edits(
            root.path(),
            &ready.id,
            "Retry edit",
            saved.edits.clone(),
        )
        .unwrap_err();
        assert!(error.contains(&quota.to_string()), "{error}");
        assert_eq!(fs::read(&manifest).unwrap(), original);
        assert_eq!(
            load_project(root.path(), &ready.id).unwrap().name,
            ready.name
        );
        fs::remove_file(reserve).unwrap();
        let error = finish_project(root.path(), &pending, capture_result(), "ready").unwrap_err();
        assert!(error.contains(&quota.to_string()), "{error}");
        assert_eq!(fs::read(directory.join("pending.json")).unwrap(), marker);
        assert_eq!(fs::read(&captured.screen_path).unwrap(), media);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 3);

        drop(filler);
        create_pending_project(&new_dir, &new_recording).unwrap();
        let new_marker = fs::read(new_dir.join("pending.json")).unwrap();
        assert!(create_pending_project(&new_dir, &new_recording).is_err());
        assert_eq!(fs::read(new_dir.join("pending.json")).unwrap(), new_marker);
        screenrecord_core::save_project_edits(root.path(), &ready.id, "Retry edit", saved.edits)
            .unwrap();
        assert_eq!(
            load_project(root.path(), &ready.id).unwrap().name,
            "Retry edit"
        );
        finish_project(root.path(), &pending, capture_result(), "interrupted").unwrap();
        let reopened = load_project(root.path(), &pending.id).unwrap();
        assert_eq!(reopened.status, "interrupted");
        assert_eq!(reopened.cursor.len(), 1024);
        assert_eq!(fs::read(reopened.screen_path).unwrap(), media);
        assert!(!directory.join("pending.json").exists());
        eprintln!("Windows quota error {quota}; last save and pending media survived; both retries reopened.");
    }

    #[test]
    #[ignore = "requires the isolated 1 MiB volume created by scripts/test-disk-full.ps1"]
    fn disk_full_recovery_rebuilds_recorded_video_from_the_original_segment() {
        let root = tempfile::tempdir_in(std::env::var_os("SCREENRECORD_TEST_QUOTA_ROOT").unwrap())
            .unwrap();
        let (pending, unused) = fixture(root.path(), 0.5);
        fs::remove_file(unused.screen_path).unwrap();
        fs::remove_file(unused.camera_path.unwrap()).unwrap();
        let directory = root.path().join(&pending.id);
        let part = directory.join("part-00000");
        let segments = part.join("segments");
        fs::create_dir_all(&segments).unwrap();
        let original = segments.join("screen-00000.mp4");
        let engine = PathBuf::from(std::env::var_os("SCREENRECORD_FFMPEG").unwrap());
        let output = Command::new(&engine)
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:size=64x48:rate=30",
                "-frames:v",
                "15",
                "-c:v",
                "h264_mf",
                "-hw_encoding",
                "0",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&original)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let original_bytes = fs::read(&original).unwrap();
        let journal = part.join("capture-journal.json");
        let part_journal = br#"{"version":1,"sourceId":"monitor:1","width":64,"height":48,"fps":30,"epochQpc":1,"duration":0.5,"microphone":false,"systemAudio":false,"segments":["screen-00000.mp4"],"segmentDurations":[0.5]}"#;
        fs::write(&journal, part_journal).unwrap();
        fs::write(directory.join("session-journal.json"), br#"{"version":1,"dimensions":[64,48,30],"parts":[{"directory":"part-00000","duration":0.5}]}"#).unwrap();
        let marker = fs::read(directory.join("pending.json")).unwrap();
        let (filler, _) = fill_quota(root.path());
        assert!(screenrecord_capture::session::recover_session(&directory, &engine).is_err());
        assert_eq!(fs::read(&journal).unwrap(), part_journal);
        assert_eq!(fs::read(&original).unwrap(), original_bytes);
        assert_eq!(fs::read(directory.join("pending.json")).unwrap(), marker);
        assert!(!directory.join("project.json").exists());
        drop(filler);
        let mut parts =
            screenrecord_capture::session::recover_session(&directory, &engine).unwrap();
        assert_eq!(parts.len(), 1);
        finish_project(root.path(), &pending, parts.remove(0), "interrupted").unwrap();
        let project = load_project(root.path(), &pending.id).unwrap();
        assert_eq!(project.status, "interrupted");
        assert_eq!(project.duration, 0.5);
        let decoded = Command::new(&engine)
            .args(["-v", "error", "-i"])
            .arg(project.screen_path)
            .args([
                "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "gray", "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(decoded.status.success());
        assert_eq!(decoded.stdout.len(), 15 * 64 * 48);
        assert_eq!(fs::read(&original).unwrap(), original_bytes);
        assert!(!directory.join("pending.json").exists());

        let (resumed, unused) = fixture(root.path(), 1.0);
        fs::remove_file(unused.screen_path).unwrap();
        fs::remove_file(unused.camera_path.unwrap()).unwrap();
        let resumed_dir = root.path().join(&resumed.id);
        for name in ["part-00000", "part-00001"] {
            let part = resumed_dir.join(name);
            fs::create_dir_all(part.join("segments")).unwrap();
            fs::write(part.join("segments/screen-00000.mp4"), &original_bytes).unwrap();
            fs::write(part.join("capture-journal.json"), part_journal).unwrap();
        }
        fs::write(resumed_dir.join("part-00000/screen.mp4"), &original_bytes).unwrap();
        fs::write(resumed_dir.join("session-journal.json"), br#"{"version":1,"dimensions":[64,48,30],"parts":[{"directory":"part-00000","duration":0.5},{"directory":"part-00001","duration":0.5}]}"#).unwrap();
        fs::write(
            resumed_dir.join("recovery-warning.txt"),
            b"Previous recovery attempt",
        )
        .unwrap();
        let (filler, _) = fill_quota(root.path());
        assert!(screenrecord_capture::session::recover_session(&resumed_dir, &engine).is_err(),
            "Recovery returned only the already-muxed part while the second intact part was out of space");
        assert!(resumed_dir.join("pending.json").is_file());
        drop(filler);
        let parts = screenrecord_capture::session::recover_session(&resumed_dir, &engine).unwrap();
        assert_eq!(parts.len(), 2);
        for (index, part) in parts.iter().enumerate() {
            let expected = resumed_dir.join(format!("part-{index:05}"));
            assert_eq!(
                dunce::canonicalize(&part.screen_path).unwrap(),
                dunce::canonicalize(expected.join("screen.mp4")).unwrap()
            );
            assert_eq!(part.duration, 0.5);
            assert_eq!(
                fs::read(expected.join("segments/screen-00000.mp4")).unwrap(),
                original_bytes
            );
        }
    }
}
