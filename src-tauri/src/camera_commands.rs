use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use screenrecord_capture::preview::CameraPreview;

#[derive(Default)]
pub(super) struct CameraPreviewState {
    request: Option<(String, Arc<AtomicBool>)>,
    resource: Option<(String, CameraPreview)>,
}

impl CameraPreviewState {
    fn reserve(&mut self, id: String) -> Arc<AtomicBool> {
        if let Some((_, token)) = &self.request {
            token.store(true, Ordering::Release);
        }
        let token = Arc::new(AtomicBool::new(false));
        self.request = Some((id, token.clone()));
        token
    }
    fn revoke(&mut self, id: Option<&str>) {
        if let Some((current, token)) = &self.request {
            if id.is_none_or(|id| id == current) {
                token.store(true, Ordering::Release);
            }
        }
    }
}

pub(super) fn stop_all(state: &AppState) -> Result<(), String> {
    let resource = {
        let mut preview = state.camera_preview.lock().map_err(|e| e.to_string())?;
        preview.revoke(None);
        preview.request = None;
        preview.resource.take()
    };
    if let Some((_, mut resource)) = resource {
        resource.stop()?;
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CameraPreviewFrame {
    sequence: u64,
    mime_type: &'static str,
    data: Option<String>,
    active: bool,
    error: Option<String>,
}

#[tauri::command]
pub(super) async fn start_camera_preview(
    app: tauri::AppHandle,
    camera_id: String,
    session_id: String,
) -> Result<(), String> {
    validate_id(&session_id)?;
    if camera_id.is_empty() || camera_id.len() > 4096 || camera_id.contains('\0') {
        return Err("Select a valid camera before opening preview.".into());
    }
    let token = {
        let state = app.state::<AppState>();
        if busy(&state) {
            return Err("Stop the current media operation before previewing a camera.".into());
        }
        if !state
            .known_devices
            .lock()
            .map_err(|e| e.to_string())?
            .camera_available(&camera_id)
        {
            return Err("Refresh devices and select an available camera before previewing.".into());
        }
        let token = state
            .camera_preview
            .lock()
            .map_err(|e| e.to_string())?
            .reserve(session_id.clone());
        token
    };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state.operation.lock().map_err(|e| e.to_string())?;
        if token.load(Ordering::Acquire) {
            return Err("Camera preview was cancelled.".into());
        }
        let previous = state
            .camera_preview
            .lock()
            .map_err(|e| e.to_string())?
            .resource
            .take();
        if let Some((_, mut previous)) = previous {
            previous.stop()?;
        }
        if busy(&state) {
            return Err("Stop the current media operation before previewing a camera.".into());
        }
        if token.load(Ordering::Acquire) {
            return Err("Camera preview was cancelled.".into());
        }
        let mut resource = CameraPreview::start(&camera_id, ffmpeg(&state)?)?;
        let mut slot = state.camera_preview.lock().map_err(|e| e.to_string())?;
        if token.load(Ordering::Acquire) {
            drop(slot);
            resource.stop()?;
            return Err("Camera preview was cancelled.".into());
        }
        slot.resource = Some((session_id, resource));
        Ok(())
    })
    .await
    .map_err(|_| "Camera preview worker failed.".to_string())?
}

#[tauri::command]
pub(super) async fn stop_camera_preview(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        state
            .camera_preview
            .lock()
            .map_err(|e| e.to_string())?
            .revoke(Some(&session_id));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state.operation.lock().map_err(|e| e.to_string())?;
        let resource = {
            let mut preview = state.camera_preview.lock().map_err(|e| e.to_string())?;
            if preview
                .resource
                .as_ref()
                .is_some_and(|(id, _)| id == &session_id)
            {
                preview.resource.take()
            } else {
                None
            }
        };
        if let Some((_, mut resource)) = resource {
            resource.stop()?;
        }
        Ok(())
    })
    .await
    .map_err(|_| "Camera preview cleanup failed.".to_string())?
}

#[tauri::command]
pub(super) fn get_camera_preview_frame(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<CameraPreviewFrame, String> {
    let preview = state.camera_preview.lock().map_err(|e| e.to_string())?;
    let current = preview
        .request
        .as_ref()
        .is_some_and(|(id, token)| id == &session_id && !token.load(Ordering::Acquire));
    let snapshot = preview
        .resource
        .as_ref()
        .filter(|(id, _)| id == &session_id && current)
        .map(|(_, resource)| resource.snapshot());
    match snapshot {
        Some(snapshot) => Ok(CameraPreviewFrame {
            sequence: snapshot.sequence,
            mime_type: "image/jpeg",
            data: snapshot.jpeg.map(|jpeg| STANDARD.encode(jpeg)),
            active: snapshot.active,
            error: snapshot.error,
        }),
        None => Ok(CameraPreviewFrame {
            sequence: 0,
            mime_type: "image/jpeg",
            data: None,
            active: false,
            error: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_stop_cannot_cancel_a_new_camera_session_and_pending_starts_are_revoked() {
        let mut preview = CameraPreviewState::default();
        let first = preview.reserve("first".into());
        let second = preview.reserve("second".into());
        assert!(first.load(Ordering::Acquire));
        preview.revoke(Some("first"));
        assert!(!second.load(Ordering::Acquire));
        preview.revoke(Some("second"));
        assert!(second.load(Ordering::Acquire));
        let pending = preview.reserve("pending".into());
        preview.revoke(None);
        assert!(pending.load(Ordering::Acquire));
    }
}
