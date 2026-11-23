use super::*;
use screenrecord_transcribe::local::LocalStatus;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LocalInstallProgress {
    job_id: String,
    state: String,
    progress: f64,
    error: Option<String>,
    status: Option<LocalStatus>,
}

fn publish(app: &tauri::AppHandle, progress: LocalInstallProgress) {
    if let Ok(mut current) = app.state::<AppState>().local_install_progress.lock() {
        *current = Some(progress.clone());
    }
    let _ = app.emit("local-transcription-install-progress", progress);
}

#[tauri::command]
pub(super) async fn get_local_transcription_status(
    app: tauri::AppHandle,
) -> Result<LocalStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        screenrecord_transcribe::local::status(&app.state::<AppState>().local_transcription_root)
    })
    .await
    .map_err(|_| "Local subtitle setup check failed.".to_string())?
}

#[tauri::command]
pub(super) async fn start_local_transcription_install(
    app: tauri::AppHandle,
    job_id: String,
) -> Result<(), String> {
    validate_id(&job_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let directory = {
        let state = app.state::<AppState>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "A media operation is starting. Try again shortly.")?;
        if busy(&state) {
            return Err(
                "Wait for the current media operation before installing local subtitles.".into(),
            );
        }
        *state
            .local_install_cancel
            .lock()
            .map_err(|e| e.to_string())? = Some((job_id.clone(), cancel.clone()));
        state.editor_busy.store(true, Ordering::Release);
        state.local_transcription_root.clone()
    };
    publish(
        &app,
        LocalInstallProgress {
            job_id: job_id.clone(),
            state: "running".into(),
            progress: 0.0,
            error: None,
            status: None,
        },
    );
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let busy = BusyReset(&state.editor_busy);
        let reporter = app.clone();
        let reporter_job = job_id.clone();
        let result = screenrecord_transcribe::local::install(
            directory.clone(),
            cancel.clone(),
            move |progress| {
                publish(
                    &reporter,
                    LocalInstallProgress {
                        job_id: reporter_job.clone(),
                        state: "running".into(),
                        progress,
                        error: None,
                        status: None,
                    },
                );
            },
        )
        .await;
        let (status, error, state_name) = match result {
            Ok(status) if !cancel.load(Ordering::Acquire) => (Some(status), None, "completed"),
            result => {
                let status = tauri::async_runtime::spawn_blocking(move || {
                    screenrecord_transcribe::local::status(&directory)
                })
                .await
                .ok()
                .and_then(Result::ok);
                if cancel.load(Ordering::Acquire) {
                    (status, None, "cancelled")
                } else {
                    (status, result.err(), "failed")
                }
            }
        };
        drop(busy);
        publish(
            &app,
            LocalInstallProgress {
                job_id,
                state: state_name.into(),
                progress: if state_name == "completed" { 1.0 } else { 0.0 },
                error,
                status,
            },
        );
    });
    Ok(())
}

#[tauri::command]
pub(super) fn cancel_local_transcription_install(
    state: tauri::State<'_, AppState>,
    job_id: String,
) -> Result<(), String> {
    validate_id(&job_id)?;
    if let Some((current, cancel)) = state
        .local_install_cancel
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
    {
        if current == &job_id {
            cancel.store(true, Ordering::Release);
        }
    }
    Ok(())
}

#[tauri::command]
pub(super) fn get_local_transcription_install_progress(
    state: tauri::State<'_, AppState>,
) -> Option<LocalInstallProgress> {
    state
        .local_install_progress
        .lock()
        .ok()
        .and_then(|progress| progress.clone())
}
