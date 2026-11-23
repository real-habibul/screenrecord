use crate::apps::AppWindow;
use std::{
    mem::{size_of, ManuallyDrop},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use windows::{
    core::{implement, IUnknown, Interface, Owned, Ref, HRESULT},
    Win32::{
        Foundation::{HANDLE, HWND, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{
            ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
            IActivateAudioInterfaceCompletionHandler,
            IActivateAudioInterfaceCompletionHandler_Impl, IAudioClient,
            AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
            AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
            PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        },
        System::{
            Com::{
                StructuredStorage::{
                    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                },
                BLOB,
            },
            Threading::{
                CreateEventW, GetProcessId, OpenProcess, WaitForSingleObject,
                PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            },
            Variant::VT_BLOB,
        },
        UI::WindowsAndMessaging::GetWindowThreadProcessId,
    },
};

pub(super) struct Target {
    app: AppWindow,
    process: Owned<HANDLE>,
}

impl Target {
    pub(super) fn open(app: AppWindow) -> Result<Self, String> {
        app.validate()?;
        let process = unsafe {
            Owned::new(
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    false,
                    app.pid,
                )
                .map_err(|error| {
                    format!("Cannot access the selected application for audio: {error}")
                })?,
            )
        };
        let target = Self { app, process };
        target.validate()?;
        Ok(target)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        let mut owner = 0;
        unsafe {
            GetWindowThreadProcessId(HWND(self.app.handle as *mut _), Some(&mut owner));
        }
        let process_id = unsafe { GetProcessId(*self.process) };
        let alive = unsafe { WaitForSingleObject(*self.process, 0) } == WAIT_TIMEOUT;
        validate_identity(self.app.pid, process_id, owner, alive)
    }
}

fn validate_identity(expected: u32, process: u32, owner: u32, alive: bool) -> Result<(), String> {
    if expected == 0 || !alive || process != expected || owner != expected {
        return Err("The selected application or window closed or changed. Select it again before recording application audio.".into());
    }
    Ok(())
}

#[derive(Default)]
struct CompletionState {
    done: Mutex<bool>,
    changed: Condvar,
}

impl CompletionState {
    fn complete(&self) {
        *self.done.lock().unwrap_or_else(|error| error.into_inner()) = true;
        self.changed.notify_all();
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion(Arc<CompletionState>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        self.0.complete();
        Ok(())
    }
}

fn activation_params(pid: u32) -> AUDIOCLIENT_ACTIVATION_PARAMS {
    AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    }
}

pub(super) fn activate(
    pid: u32,
    stop: &AtomicBool,
    ready: &mpsc::SyncSender<Result<(), String>>,
) -> Result<wasapi::AudioClient, String> {
    if stop.load(Ordering::Acquire) {
        return Err("Application audio startup was cancelled.".into());
    }
    let mut params = Box::new(activation_params(pid));
    // The blob borrows params; PROPVARIANT must not free it with CoTaskMemFree.
    let prop = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: std::ptr::from_mut(params.as_mut()).cast(),
                    },
                },
            }),
        },
    });
    let state = Arc::new(CompletionState::default());
    let callback: IActivateAudioInterfaceCompletionHandler = Completion(state.clone()).into();
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*prop),
            &callback,
        )
    }
    .map_err(|error| {
        format!(
            "Application audio activation failed. Whole-computer audio was not selected: {error}"
        )
    })?;

    let waited = await_completion(&state, stop, Duration::from_secs(5), |error| {
        let _ = ready.try_send(Err(error));
    });
    // A timed-out activation stays on this MTA worker until Windows releases its callback.
    // The caller's cleanup registry retains the worker and blocks a second recorder.
    waited?;
    let mut result = HRESULT(0);
    let mut interface: Option<IUnknown> = None;
    unsafe { operation.GetActivateResult(&mut result, &mut interface) }
        .map_err(|error| format!("Cannot read application audio activation result: {error}"))?;
    activated_client(result, interface)
}

fn activated_client(
    result: HRESULT,
    interface: Option<IUnknown>,
) -> Result<wasapi::AudioClient, String> {
    result.ok().map_err(|error| {
        format!("Application audio is unavailable. Whole-computer audio was not selected: {error}")
    })?;
    let client = interface
        .ok_or("Application audio activation returned no audio client.")?
        .cast::<IAudioClient>()
        .map_err(|error| error.to_string())?;
    Ok(wasapi::AudioClient::from_iaudioclient(
        client,
        wasapi::Direction::Render,
    ))
}

fn await_completion(
    state: &CompletionState,
    stop: &AtomicBool,
    deadline: Duration,
    mut abandoned: impl FnMut(String),
) -> Result<(), String> {
    let started = Instant::now();
    let mut failure = None;
    let mut done = state.done.lock().unwrap_or_else(|error| error.into_inner());
    loop {
        if failure.is_none() && (stop.load(Ordering::Acquire) || started.elapsed() >= deadline) {
            let error = if stop.load(Ordering::Acquire) {
                "Application audio startup was cancelled."
            } else {
                "Application audio did not activate within five seconds. Whole-computer audio was not selected."
            }.to_owned();
            abandoned(error.clone());
            failure = Some(error);
        }
        if *done {
            return failure.map_or(Ok(()), Err);
        }
        let (next, _) = state
            .changed
            .wait_timeout(done, Duration::from_millis(20))
            .unwrap_or_else(|error| error.into_inner());
        done = next;
    }
}

pub(super) fn create_event(client: &wasapi::AudioClient) -> Result<Owned<HANDLE>, String> {
    let event = unsafe {
        Owned::new(CreateEventW(None, false, false, None).map_err(|error| error.to_string())?)
    };
    unsafe { client.as_iaudioclient().SetEventHandle(*event) }
        .map_err(|error| format!("Cannot initialize application audio events: {error}"))?;
    Ok(event)
}

pub(super) fn wait_event(event: &Owned<HANDLE>) -> Result<(), String> {
    match unsafe { WaitForSingleObject(**event, 20) } {
        WAIT_OBJECT_0 | WAIT_TIMEOUT => Ok(()),
        _ => Err(format!(
            "Application audio event wait failed: {}",
            windows::core::Error::from_thread()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn activation_errors_and_missing_interface_do_not_return_a_client() {
        let denied = activated_client(HRESULT(0x80070005u32 as i32), None)
            .err()
            .unwrap();
        assert!(denied.contains("Whole-computer audio was not selected"));
        assert!(activated_client(HRESULT(0), None)
            .err()
            .unwrap()
            .contains("no audio client"));
    }

    #[test]
    fn completion_handler_supports_agile_callback_marshalling() {
        let _com = super::super::Com::initialize().unwrap();
        let callback: IActivateAudioInterfaceCompletionHandler =
            Completion(Arc::new(CompletionState::default())).into();
        assert!(callback
            .cast::<windows::Win32::System::Com::IAgileObject>()
            .is_ok());
        assert!(callback
            .cast::<windows::Win32::System::Com::Marshal::IMarshal>()
            .is_ok());
    }

    #[test]
    fn activation_includes_only_the_selected_process_tree() {
        let params = activation_params(42);
        assert_eq!(
            params.ActivationType,
            AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK
        );
        let process = unsafe { params.Anonymous.ProcessLoopbackParams };
        assert_eq!(process.TargetProcessId, 42);
        assert_eq!(
            process.ProcessLoopbackMode,
            PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
        );
    }

    #[test]
    fn closed_or_reused_process_and_window_are_rejected() {
        assert!(validate_identity(42, 42, 42, true).is_ok());
        for (process, owner, alive) in [
            (42, 42, false),
            (43, 42, true),
            (42, 43, true),
            (42, 0, true),
            (0, 42, true),
        ] {
            assert!(validate_identity(42, process, owner, alive).is_err());
        }
        assert!(validate_identity(0, 0, 0, true).is_err());
    }

    #[test]
    fn completed_activation_and_precancelled_completion() {
        let state = CompletionState::default();
        state.complete();
        let stop = AtomicBool::new(false);
        assert!(
            await_completion(&state, &stop, Duration::from_secs(1), |_| panic!(
                "unexpected abandon"
            ))
            .is_ok()
        );
        stop.store(true, Ordering::Release);
        let mut count = 0;
        assert!(
            await_completion(&state, &stop, Duration::from_secs(1), |_| count += 1)
                .unwrap_err()
                .contains("cancelled")
        );
        assert_eq!(count, 1);
    }

    #[test]
    fn timeout_reports_promptly_but_retains_activation_until_late_completion() {
        struct OwnedProbe(Arc<AtomicUsize>);
        impl Drop for OwnedProbe {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let probe = OwnedProbe(drops.clone());
        let state = Arc::new(CompletionState::default());
        let pending = state.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let _owned = probe;
            await_completion(
                &pending,
                &AtomicBool::new(false),
                Duration::from_millis(20),
                |error| {
                    tx.send(error).unwrap();
                },
            )
        });
        assert!(rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .contains("did not activate"));
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(!worker.is_finished());
        state.complete();
        assert!(worker.join().unwrap().is_err());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancellation_is_reported_once_and_late_success_cannot_restart_capture() {
        let state = Arc::new(CompletionState::default());
        let pending = state.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let (tx, rx) = mpsc::sync_channel(2);
        let worker = std::thread::spawn(move || {
            await_completion(&pending, &cancelled, Duration::from_secs(10), |error| {
                tx.send(error).unwrap();
            })
        });
        stop.store(true, Ordering::Release);
        assert!(rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .contains("cancelled"));
        assert!(!worker.is_finished());
        state.complete();
        assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
        assert!(rx.try_recv().is_err());
    }
}
