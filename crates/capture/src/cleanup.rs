use std::{
    collections::HashMap,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, LazyLock, Mutex},
    thread,
    time::Duration,
};

pub const DEADLINE: Duration = Duration::from_secs(15);

static REGISTRY: LazyLock<Arc<Registry>> = LazyLock::new(|| Arc::new(Registry::default()));

#[derive(Default)]
struct Registry {
    active: Mutex<HashMap<PathBuf, usize>>,
    quarantined: Mutex<Vec<Box<dyn Send>>>,
}

struct Lease {
    registry: Arc<Registry>,
    directory: PathBuf,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut active = self
            .registry
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(count) = active.get_mut(&self.directory) {
            *count -= 1;
            if *count == 0 {
                active.remove(&self.directory);
            }
        }
    }
}

impl Registry {
    fn has_pending(&self, directory: Option<&Path>) -> bool {
        let active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        directory.map_or_else(|| !active.is_empty(), |path| active.contains_key(path))
    }

    fn run<T, F>(
        self: &Arc<Self>,
        directory: &Path,
        deadline: Duration,
        work: F,
    ) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        self.run_initialized(directory, deadline, || Ok(()), work)
    }

    fn run_initialized<T, F, I, A>(
        self: &Arc<Self>,
        directory: &Path,
        deadline: Duration,
        initialize: I,
        work: F,
    ) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
        I: FnOnce() -> Result<A, String> + Send + 'static,
    {
        *self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(directory.to_owned())
            .or_default() += 1;
        let lease = Lease {
            registry: self.clone(),
            directory: directory.to_owned(),
        };
        let pending = Arc::new(Mutex::new(Some((lease, work))));
        let worker_pending = pending.clone();
        let worker_registry = self.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let spawned = thread::Builder::new().name("capture-cleanup".into()).spawn(move || {
            let _apartment = match initialize() {
                Ok(apartment) => apartment,
                Err(error) => {
                    // Retain resources whose finalizers require an initialized apartment.
                    worker_registry.quarantined.lock().unwrap_or_else(|error| error.into_inner())
                        .push(Box::new(worker_pending));
                    let _ = sender.send(Err(format!("Cannot initialize native cleanup: {error}. Restart the application before recording or recovery.")));
                    return;
                }
            };
            let (lease, work) = worker_pending.lock().unwrap_or_else(|error| error.into_inner())
                .take().expect("Cleanup work is assigned once.");
            let result = match catch_unwind(AssertUnwindSafe(work)) {
                Ok(result) => { drop(lease); result }
                Err(_) => {
                    // A panic leaves resource ownership uncertain; keep recovery blocked until restart.
                    let registry = lease.registry.clone();
                    registry.quarantined.lock().unwrap_or_else(|error| error.into_inner()).push(Box::new(lease));
                    Err("Native capture cleanup panicked. Restart the application before recording or recovery.".into())
                }
            };
            let _ = sender.send(result);
        });
        if let Err(error) = spawned {
            self.quarantined
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(Box::new(pending));
            return Err(format!("Cannot start capture cleanup: {error}. Restart the application before recording or recovery."));
        }
        receiver.recv_timeout(deadline).map_err(|_| {
            "Native capture is still stopping. Recording and recovery of this project remain blocked until cleanup finishes. Restart the application if this persists.".to_owned()
        })?
    }
}

pub fn in_progress() -> bool {
    REGISTRY.has_pending(None)
}

pub fn ensure_idle() -> Result<(), String> {
    if in_progress() {
        return Err("The previous recording is still stopping. Wait for cleanup to finish or restart the application before recording again.".into());
    }
    Ok(())
}

pub fn ensure_recovery_ready(directory: &Path) -> Result<(), String> {
    if REGISTRY.has_pending(Some(directory)) {
        return Err("This recording is still stopping. Recovery cannot access its media until cleanup finishes; restart the application if this persists.".into());
    }
    Ok(())
}

pub fn run<T, F>(directory: &Path, deadline: Duration, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    REGISTRY.run(directory, deadline, work)
}

pub(crate) struct NativeRuntime(std::marker::PhantomData<std::rc::Rc<()>>);

impl NativeRuntime {
    pub(crate) fn initialize() -> Result<Self, String> {
        unsafe {
            windows::Win32::System::WinRT::RoInitialize(
                windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            )
        }
        .map_err(|error| error.to_string())?;
        Ok(Self(std::marker::PhantomData))
    }
}

impl Drop for NativeRuntime {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::WinRT::RoUninitialize() };
    }
}

pub fn run_native<T, F>(directory: &Path, deadline: Duration, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    REGISTRY.run_initialized(directory, deadline, NativeRuntime::initialize, work)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_mta() {
        use windows::Win32::System::Com::{
            CoGetApartmentType, APTTYPE, APTTYPEQUALIFIER, APTTYPE_MTA,
        };
        let mut apartment = APTTYPE::default();
        let mut qualifier = APTTYPEQUALIFIER::default();
        unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) }.unwrap();
        assert_eq!(apartment, APTTYPE_MTA);
    }

    #[test]
    fn native_cleanup_keeps_the_apartment_initialized_through_owned_destruction() {
        struct Finalizer(mpsc::Sender<()>);
        impl Drop for Finalizer {
            fn drop(&mut self) {
                assert_mta();
                self.0.send(()).unwrap();
            }
        }
        let registry = Arc::new(Registry::default());
        let (sent, dropped) = mpsc::channel();
        let finalizer = Finalizer(sent);
        let result: Result<(), String> = registry.run_initialized(
            Path::new("native-project"),
            Duration::from_secs(2),
            NativeRuntime::initialize,
            move || {
                assert_mta();
                let _owned = finalizer;
                Err("Injected native finalization error".into())
            },
        );
        assert_eq!(result.unwrap_err(), "Injected native finalization error");
        dropped.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(!registry.has_pending(None));
    }

    #[test]
    fn failed_apartment_initialization_preserves_resources_and_the_cleanup_lock() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Finalizer(Arc<AtomicBool>);
        impl Drop for Finalizer {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let registry = Arc::new(Registry::default());
        let destroyed = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(AtomicBool::new(false));
        let work_entered = entered.clone();
        let finalizer = Finalizer(destroyed.clone());
        let result: Result<(), String> = registry.run_initialized(
            Path::new("native-project"),
            Duration::from_secs(2),
            || Err::<(), _>("Injected RoInitialize failure".into()),
            move || {
                work_entered.store(true, Ordering::Release);
                drop(finalizer);
                Ok(())
            },
        );
        assert!(result
            .unwrap_err()
            .contains("Injected RoInitialize failure"));
        assert!(!entered.load(Ordering::Acquire));
        assert!(!destroyed.load(Ordering::Acquire));
        assert!(registry.has_pending(None));
        // Simulate process disposal only after checking quarantined ownership.
        registry.quarantined.lock().unwrap().clear();
        assert!(destroyed.load(Ordering::Acquire));
        assert!(!registry.has_pending(None));
    }

    #[test]
    fn overlapping_cleanup_keeps_the_project_locked_until_every_writer_exits() {
        let registry = Arc::new(Registry::default());
        let path = Path::new("project");
        let (release, blocked) = mpsc::channel();
        assert!(registry
            .run(path, Duration::from_millis(20), move || {
                blocked.recv().map_err(|error| error.to_string())?;
                Ok(())
            })
            .is_err());
        assert!(registry.has_pending(Some(path)));
        assert!(registry
            .run(path, Duration::from_secs(1), || Ok(()))
            .is_ok());
        assert!(registry.has_pending(Some(path)));
        release.send(()).unwrap();
        for _ in 0..100 {
            if !registry.has_pending(None) {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("Cleanup guard remained after all writers exited.");
    }

    #[test]
    fn a_cleanup_panic_does_not_publish_uncertain_media_as_safe() {
        let registry = Arc::new(Registry::default());
        let result: Result<(), String> =
            registry.run(Path::new("project"), Duration::from_secs(1), || {
                panic!("Injected native finalizer panic");
            });
        assert!(result.unwrap_err().contains("panicked"));
        assert!(registry.has_pending(None));
    }
}
