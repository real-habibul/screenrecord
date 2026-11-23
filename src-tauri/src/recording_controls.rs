use super::*;
use tauri::{
    LogicalSize, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

const LABEL: &str = "recording-controls";

pub(super) struct StartUiGuard {
    app: tauri::AppHandle,
    committed: bool,
}
impl StartUiGuard {
    pub(super) fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            committed: false,
        }
    }
    pub(super) fn commit(&mut self) {
        self.committed = true;
    }
}
impl Drop for StartUiGuard {
    fn drop(&mut self) {
        if !self.committed {
            hide(&self.app);
            show_main(&self.app);
        }
    }
}

pub(super) fn protect(window: &WebviewWindow, enabled: bool) -> Result<(), String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GetWindowDisplayAffinity, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
        },
    };
    let handle = window.hwnd().map_err(|e| e.to_string())?;
    let handle = HWND(handle.0 as *mut _);
    let requested = if enabled {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };
    unsafe {
        SetWindowDisplayAffinity(handle, requested)
            .map_err(|e| format!("Cannot change recording-window capture exclusion: {e}"))?;
        let mut actual = 0;
        match GetWindowDisplayAffinity(handle, &mut actual) {
            Ok(()) if actual != requested.0 => {
                return Err(
                    "Windows did not apply the requested recording-window capture exclusion."
                        .into(),
                )
            }
            Err(error) => {
                log::debug!("Display affinity readback is unavailable for this window: {error}")
            }
            _ => {}
        }
    }
    Ok(())
}

fn fit_position(
    position: (i32, i32),
    size: (u32, u32),
    origin: (i32, i32),
    area: (u32, u32),
) -> (i32, i32) {
    let clamp = |value: i32, start: i32, available: u32, needed: u32| {
        i64::from(value).clamp(
            i64::from(start),
            (i64::from(start) + i64::from(available) - i64::from(needed)).max(i64::from(start)),
        ) as i32
    };
    (
        clamp(position.0, origin.0, area.0, size.0),
        clamp(position.1, origin.1, area.1, size.1),
    )
}

pub(super) fn prepare(app: &tauri::AppHandle, camera: bool) -> Result<WebviewWindow, String> {
    let existing = app.get_webview_window(LABEL);
    let fresh = existing.is_none();
    let window = match existing {
        Some(window) => window,
        None => WebviewWindowBuilder::new(
            app,
            LABEL,
            WebviewUrl::App("index.html?overlay=recording".into()),
        )
        .title("Recording controls")
        .inner_size(340.0, if camera { 278.0 } else { 116.0 })
        .decorations(false)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .content_protected(true)
        .shadow(false)
        .build()
        .map_err(|e| format!("Cannot open floating recording controls: {e}"))?,
    };
    protect(&window, true)?;
    window.set_always_on_top(true).map_err(|e| e.to_string())?;
    let monitor = if fresh {
        None
    } else {
        window.current_monitor().map_err(|e| e.to_string())?
    }
    .or_else(|| {
        app.get_webview_window("main")
            .and_then(|main| main.current_monitor().ok().flatten())
    })
    .or(app.primary_monitor().map_err(|e| e.to_string())?);
    if let Some(monitor) = monitor {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let width = (340.0 * scale).round().min(f64::from(area.size.width)) as u32;
        let height = ((if camera { 278.0 } else { 116.0 }) * scale)
            .round()
            .min(f64::from(area.size.height)) as u32;
        window
            .set_size(PhysicalSize::new(width.max(1), height.max(1)))
            .map_err(|e| e.to_string())?;
        let current = if fresh {
            (
                area.position
                    .x
                    .saturating_add(area.size.width.saturating_sub(width) as i32)
                    .saturating_sub(16),
                area.position.y.saturating_add(24),
            )
        } else {
            let current = window.outer_position().map_err(|e| e.to_string())?;
            (current.x, current.y)
        };
        let position = fit_position(
            current,
            (width, height),
            (area.position.x, area.position.y),
            (area.size.width, area.size.height),
        );
        window
            .set_position(PhysicalPosition::new(position.0, position.1))
            .map_err(|e| e.to_string())?;
    } else {
        window
            .set_size(LogicalSize::new(340.0, if camera { 278.0 } else { 116.0 }))
            .map_err(|e| e.to_string())?;
    }
    Ok(window)
}

pub(super) fn hide(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = protect(&main, false);
    }
}

#[tauri::command]
pub(super) async fn restore_main_window(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        show_main(&app);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn floating_controls_fit_negative_monitors_small_work_areas_and_removed_displays() {
        assert_eq!(
            fit_position((-3000, -500), (340, 278), (-1920, 0), (1920, 1040)),
            (-1920, 0)
        );
        assert_eq!(
            fit_position((4000, 4000), (340, 278), (0, 0), (1920, 1040)),
            (1580, 762)
        );
        assert_eq!(
            fit_position((300, 300), (340, 278), (0, 0), (200, 100)),
            (0, 0)
        );
        assert_eq!(
            fit_position((50, 50), (340, 278), (0, 0), (1920, 1040)),
            (50, 50)
        );
    }
}
