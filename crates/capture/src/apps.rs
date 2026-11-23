use screenrecord_core::{CaptureApp, CaptureSource};
use std::collections::BTreeMap;
use windows::{
    core::{Owned, PWSTR},
    Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    },
};
use windows_capture::{settings::GraphicsCaptureItemType, window::Window};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AppWindow {
    pub(crate) pid: u32,
    pub(crate) handle: usize,
}

impl AppWindow {
    pub(crate) fn parse(id: &str) -> Result<Self, String> {
        let invalid = || "Invalid app window identifier.".to_owned();
        let mut fields = id.split(':');
        if fields.next() != Some("app") {
            return Err(invalid());
        }
        let pid = fields.next().ok_or_else(invalid)?;
        let handle = fields.next().ok_or_else(invalid)?;
        if fields.next().is_some() || !canonical_decimal(pid) || !canonical_decimal(handle) {
            return Err(invalid());
        }
        Ok(Self {
            pid: pid.parse().map_err(|_| invalid())?,
            handle: handle.parse().map_err(|_| invalid())?,
        })
    }

    pub(crate) fn validate(self) -> Result<(), String> {
        let window = Window::from_raw_hwnd(self.handle as *mut _);
        let first_pid = window.process_id().ok();
        let eligible = first_pid.is_some_and(|pid| pid != std::process::id())
            && window.is_valid()
            && window.title().is_ok_and(|title| !title.trim().is_empty())
            && super::Target::Window(self.handle).rect().is_ok_and(|rect| {
                rect.right as i64 - rect.left as i64 >= 2
                    && rect.bottom as i64 - rect.top as i64 >= 2
            });
        let last_pid = window.process_id().ok();
        self.validate_owner(first_pid, last_pid, eligible)
    }

    fn validate_owner(
        self,
        first_pid: Option<u32>,
        last_pid: Option<u32>,
        eligible: bool,
    ) -> Result<(), String> {
        if self.pid == 0 || self.handle == 0 || !eligible {
            return Err(
                "The selected app window is no longer available. Refresh Apps and select a window."
                    .into(),
            );
        }
        if first_pid != Some(self.pid) || last_pid != Some(self.pid) {
            return Err("The selected window no longer belongs to this app. Refresh Apps and select a window.".into());
        }
        Ok(())
    }

    fn resolve<T>(
        self,
        mut validate: impl FnMut(Self) -> Result<(), String>,
        create: impl FnOnce(Self) -> Result<T, String>,
    ) -> Result<T, String> {
        validate(self)?;
        let item = create(self)?;
        validate(self)?;
        Ok(item)
    }
}

impl TryFrom<AppWindow> for GraphicsCaptureItemType {
    type Error = String;

    fn try_from(app: AppWindow) -> Result<Self, Self::Error> {
        app.resolve(AppWindow::validate, |app| {
            Window::from_raw_hwnd(app.handle as *mut _)
                .try_into()
                .map_err(|error: windows::core::Error| error.to_string())
        })
    }
}

fn canonical_decimal(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('0') && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub(crate) fn group(
    windows: Vec<(AppWindow, CaptureSource)>,
    mut executable: impl FnMut(u32) -> Option<String>,
) -> Vec<CaptureApp> {
    let mut groups: BTreeMap<u32, Vec<CaptureSource>> = BTreeMap::new();
    for (app, mut source) in windows {
        if app.pid == 0
            || app.handle == 0
            || source.kind != "window"
            || source.title.trim().is_empty()
            || source.width < 2
            || source.height < 2
        {
            continue;
        }
        source.id = format!("app:{}:{}", app.pid, app.handle);
        let windows = groups.entry(app.pid).or_default();
        if !windows.iter().any(|window| window.id == source.id) {
            windows.push(source);
        }
    }
    let mut apps = groups
        .into_iter()
        .map(|(pid, mut windows)| {
            windows.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
            let name = executable(pid)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| windows[0].title.clone());
            CaptureApp {
                id: format!("process:{pid}"),
                name,
                windows,
            }
        })
        .collect::<Vec<_>>();
    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    apps
}

pub(crate) fn executable_name(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let process = unsafe { Owned::new(process) };
    let mut buffer = vec![0u16; 32768];
    let mut size = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            *process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    }
    .ok()?;
    executable_basename(&String::from_utf16(buffer.get(..size as usize)?).ok()?)
}

fn executable_basename(path: &str) -> Option<String> {
    let name = path.rsplit(['\\', '/']).next()?;
    (!name.trim().is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(pid: u32, handle: usize, title: &str) -> (AppWindow, CaptureSource) {
        (
            AppWindow { pid, handle },
            CaptureSource {
                id: format!("window:{handle}"),
                title: title.into(),
                kind: "window".into(),
                width: 1280,
                height: 720,
            },
        )
    }

    #[test]
    fn executable_metadata_queries_only_the_owned_test_process() {
        let expected = std::env::current_exe().unwrap();
        assert_eq!(
            executable_name(std::process::id()).as_deref(),
            expected.file_name().and_then(|name| name.to_str())
        );
    }

    #[test]
    fn grouping_keeps_process_identity_multiple_windows_and_legacy_sources() {
        let windows = vec![
            source(10, 31, "Second"),
            source(10, 30, "First"),
            source(20, 40, "Other process"),
            source(10, 30, "First"),
        ];
        let original = windows[0].1.clone();
        let mut queried = Vec::new();
        let apps = group(windows, |pid| {
            queried.push(pid);
            Some("editor.exe".into())
        });
        assert_eq!(queried, [10, 20]);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].id, "process:10");
        assert_eq!(apps[0].windows.len(), 2);
        assert_eq!(apps[0].windows[0].id, "app:10:30");
        assert_eq!(apps[1].windows[0].id, "app:20:40");
        assert_eq!(original.id, "window:31");
        assert_eq!(apps[0].windows[0].kind, "window");
        assert_eq!(
            (apps[0].windows[0].width, apps[0].windows[0].height),
            (1280, 720)
        );
    }

    #[test]
    fn denied_process_metadata_uses_a_real_title_and_excludes_unusable_windows() {
        let mut tiny = source(13, 33, "Tiny");
        tiny.1.width = 1;
        let apps = group(
            vec![
                source(11, 31, "Dokumen 日本語"),
                source(0, 32, "Unknown"),
                source(12, 0, "Gone"),
                source(14, 34, " "),
                tiny,
            ],
            |_| None,
        );
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Dokumen 日本語");
        assert_eq!(apps[0].windows[0].id, "app:11:31");
        assert_eq!(
            executable_basename(r"C:\Program Files\日本語\editor.exe").as_deref(),
            Some("editor.exe")
        );
        assert_eq!(executable_basename(r"C:\folder\"), None);
    }

    #[test]
    fn app_identifiers_require_canonical_nonzero_pid_and_handle() {
        assert_eq!(
            AppWindow::parse("app:42:900").unwrap(),
            AppWindow {
                pid: 42,
                handle: 900
            }
        );
        for id in [
            "app",
            "process:42",
            "app:0:9",
            "app:2:0",
            "app:01:9",
            "app:2:09",
            "app:+2:9",
            "app:2:-9",
            "app:2:9:1",
            "app:4294967296:9",
            "app:2:18446744073709551616",
            "app:2: 9",
            "app:2:../9",
            "app:２:9",
        ] {
            assert!(AppWindow::parse(id).is_err(), "{id}");
        }
    }

    #[test]
    fn stale_wrong_owner_and_ineligible_snapshots_never_match() {
        let target = AppWindow::parse("app:42:900").unwrap();
        assert!(target.validate_owner(Some(42), Some(42), true).is_ok());
        for (first, last, eligible) in [
            (None, None, false),
            (Some(7), Some(7), true),
            (Some(42), Some(7), true),
            (Some(42), None, true),
            (Some(42), Some(42), false),
        ] {
            assert!(target.validate_owner(first, last, eligible).is_err());
        }
    }

    #[test]
    fn delayed_or_reused_window_is_rejected_around_native_item_creation() {
        use std::cell::Cell;
        let app = AppWindow::parse("app:42:900").unwrap();
        let created = Cell::new(false);
        let failed: Result<(), String> = app.resolve(
            |_| Err("closed during device setup".into()),
            |_| {
                created.set(true);
                Ok(())
            },
        );
        assert!(failed.is_err());
        assert!(!created.get());
        let mut checks = 0;
        let changed: Result<(), String> = app.resolve(
            |expected| {
                checks += 1;
                let owner = if checks == 1 { 42 } else { 77 };
                expected.validate_owner(Some(owner), Some(owner), true)
            },
            |_| {
                created.set(true);
                Ok(())
            },
        );
        assert!(created.get());
        assert_eq!(checks, 2);
        assert!(changed.unwrap_err().contains("no longer belongs"));
    }
}
