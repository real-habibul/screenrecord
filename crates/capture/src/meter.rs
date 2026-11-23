use crate::{audio::Com, cleanup};
use serde::Serialize;
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

const RATE: u32 = 48_000;
const CHANNELS: usize = 2;
const WINDOW_FRAMES: usize = 4_800;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterSnapshot {
    pub sequence: u64,
    pub peak: f64,
    pub rms: f64,
    pub active: bool,
    pub ready: bool,
    pub error: Option<String>,
}

struct State {
    snapshot: MeterSnapshot,
}

pub struct MicrophoneMeter {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
}

impl MicrophoneMeter {
    /// Open only after an explicit microphone test; the caller releases it before recording.
    pub fn start(device_id: &str) -> Result<Self, String> {
        cleanup::ensure_idle()?;
        if device_id.is_empty() || device_id.len() > 4096 || device_id.chars().any(char::is_control)
        {
            return Err("Select a valid microphone before starting the test.".into());
        }
        let device_id = device_id.to_owned();
        Self::spawn(
            move |stop, state| capture(&device_id, stop, state),
            Duration::from_secs(10),
            Duration::from_secs(30),
        )
    }

    fn spawn(
        work: impl FnOnce(Arc<AtomicBool>, Arc<Mutex<State>>) -> Result<(), String> + Send + 'static,
        startup: Duration,
        lifetime: Duration,
    ) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(State {
            snapshot: MeterSnapshot {
                sequence: 0,
                peak: 0.0,
                rms: 0.0,
                active: true,
                ready: false,
                error: None,
            },
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = state.clone();
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("microphone-meter".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    work(worker_stop.clone(), worker_state.clone())
                }))
                .unwrap_or_else(|_| Err("Microphone test worker failed.".into()));
                let mut state = worker_state.lock().unwrap_or_else(|e| e.into_inner());
                state.snapshot.active = false;
                if let Err(error) = result {
                    state.snapshot.error.get_or_insert(error);
                } else if !worker_stop.load(Ordering::Acquire) {
                    state
                        .snapshot
                        .error
                        .get_or_insert_with(|| "Microphone test ended unexpectedly.".into());
                }
                worker_stop.store(true, Ordering::Release);
            })
            .map_err(|_| "Cannot start the microphone test worker.".to_owned())?;
        let watched_state = state.clone();
        let watched_stop = stop.clone();
        let watchdog = thread::Builder::new()
            .name("microphone-meter-watchdog".into())
            .spawn(move || {
                let started = Instant::now();
                while !watched_stop.load(Ordering::Acquire) {
                    let mut state = watched_state.lock().unwrap_or_else(|e| e.into_inner());
                    if !state.snapshot.ready && started.elapsed() >= startup {
                        state.snapshot.error =
                            Some("Microphone did not become ready within 10 seconds.".into());
                        state.snapshot.active = false;
                        watched_stop.store(true, Ordering::Release);
                    } else if started.elapsed() >= lifetime {
                        state.snapshot.active = false;
                        watched_stop.store(true, Ordering::Release);
                    }
                    drop(state);
                    thread::sleep(Duration::from_millis(10));
                }
            });
        match watchdog {
            Ok(watchdog) => Ok(Self {
                state,
                stop,
                worker: Some(worker),
                watchdog: Some(watchdog),
            }),
            Err(_) => {
                stop.store(true, Ordering::Release);
                let _ = cleanup::run(
                    Path::new("microphone-meter"),
                    Duration::from_secs(2),
                    move || {
                        worker
                            .join()
                            .map_err(|_| "Microphone test worker failed.".to_owned())
                    },
                );
                Err("Cannot start the microphone test watchdog.".into())
            }
        }
    }

    pub fn snapshot(&self) -> MeterSnapshot {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone()
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.stop_with_deadline(Duration::from_secs(2))
    }

    fn stop_with_deadline(&mut self, deadline: Duration) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.snapshot.active = false;
            state.snapshot.peak = 0.0;
            state.snapshot.rms = 0.0;
        }
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        let watchdog = self.watchdog.take();
        cleanup::run(Path::new("microphone-meter"), deadline, move || {
            worker
                .join()
                .map_err(|_| "Microphone test worker failed.".to_owned())?;
            if let Some(watchdog) = watchdog {
                watchdog
                    .join()
                    .map_err(|_| "Microphone test watchdog failed.".to_owned())?;
            }
            Ok(())
        })
    }
}

impl Drop for MicrophoneMeter {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn capture(device_id: &str, stop: Arc<AtomicBool>, state: Arc<Mutex<State>>) -> Result<(), String> {
    let _com = Com::initialize()?;
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let enumerator = DeviceEnumerator::new().map_err(|e| e.to_string())?;
    let device = enumerator
        .get_device(device_id)
        .map_err(|_| "The selected microphone is unavailable.".to_owned())?;
    if device.get_direction() != Direction::Capture {
        return Err("The selected device is not a microphone endpoint.".into());
    }
    let mut client = device
        .get_iaudioclient()
        .map_err(|e| format!("Cannot open the selected microphone: {e}"))?;
    let format = WaveFormat::new(32, 32, SampleType::Float, RATE as usize, CHANNELS, None);
    let (period, _) = client.get_device_period().map_err(|e| e.to_string())?;
    client
        .initialize_client(
            &format,
            Direction::Capture,
            StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: period.max(2_000_000),
            },
            None,
        )
        .map_err(|e| format!("Cannot initialize the microphone test: {e}"))?;
    let frames = client.get_buffer_size().map_err(|e| e.to_string())? as usize;
    if frames > RATE as usize * 2 {
        return Err("Microphone test requested an unsupported buffer size.".into());
    }
    let mut bytes = vec![0u8; frames.max(4096) * CHANNELS * 4];
    let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    client
        .start_stream()
        .map_err(|e| format!("Cannot start the microphone test: {e}"))?;
    let result = (|| {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .ready = true;
        let mut levels = Levels::default();
        let mut last_packet = Instant::now();
        while !stop.load(Ordering::Acquire) {
            while !stop.load(Ordering::Acquire)
                && capture
                    .get_next_packet_size()
                    .map_err(|e| e.to_string())?
                    .unwrap_or(0)
                    > 0
            {
                let (count, _) = capture
                    .read_from_device(&mut bytes)
                    .map_err(|e| format!("Microphone test failed: {e}"))?;
                if count > 0 {
                    last_packet = Instant::now();
                }
                levels.push(&bytes[..count as usize * CHANNELS * 4], |peak, rms| {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
                    state.snapshot.sequence += 1;
                    state.snapshot.peak = peak;
                    state.snapshot.rms = rms;
                })?;
            }
            if last_packet.elapsed() > Duration::from_secs(5) {
                return Err(
                    "Microphone stopped delivering audio. Check the device connection.".into(),
                );
            }
            thread::sleep(Duration::from_millis(4));
        }
        Ok(())
    })();
    let stopped = client
        .stop_stream()
        .map_err(|e| format!("Cannot release the microphone test: {e}"));
    result.and(stopped)
}

#[derive(Default)]
struct Levels {
    frames: usize,
    peak: f64,
    squares: f64,
}
impl Levels {
    fn push(&mut self, bytes: &[u8], mut publish: impl FnMut(f64, f64)) -> Result<(), String> {
        if !bytes.len().is_multiple_of(CHANNELS * 4) {
            return Err("Microphone returned an incomplete audio frame.".into());
        }
        for frame in bytes.as_chunks::<8>().0 {
            for sample in frame.as_chunks::<4>().0 {
                let sample = f64::from(f32::from_le_bytes(*sample));
                if !sample.is_finite() {
                    return Err("Microphone returned invalid audio samples.".into());
                }
                self.peak = self.peak.max(sample.abs());
                self.squares += sample * sample;
            }
            self.frames += 1;
            if self.frames == WINDOW_FRAMES {
                publish(
                    self.peak,
                    (self.squares / (self.frames * CHANNELS) as f64).sqrt(),
                );
                *self = Self::default();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn meter_windows_use_all_stereo_samples_across_packet_boundaries() {
        let mut levels = Levels::default();
        let mut snapshots = Vec::new();
        let packet: Vec<_> = (0..WINDOW_FRAMES + 777)
            .flat_map(|_| [0.5f32, -0.25].into_iter().flat_map(f32::to_le_bytes))
            .collect();
        for bytes in packet.chunks(8 * 137) {
            levels
                .push(bytes, |peak, rms| snapshots.push((peak, rms)))
                .unwrap();
        }
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].0, 0.5);
        assert!((snapshots[0].1 - 0.15625f64.sqrt()).abs() < 1e-12);
        let silence = vec![0u8; (WINDOW_FRAMES - 777) * 8];
        levels
            .push(&silence, |peak, rms| snapshots.push((peak, rms)))
            .unwrap();
        assert_eq!(snapshots.len(), 2);
        assert!(
            (snapshots[1].1 - (777.0 * 0.3125 / (WINDOW_FRAMES * 2) as f64).sqrt()).abs() < 1e-12
        );
        assert!(levels.push(&[0; 7], |_, _| {}).is_err());
        let invalid = [f32::NAN.to_le_bytes(), 0f32.to_le_bytes()].concat();
        assert!(levels.push(&invalid, |_, _| {}).is_err());
    }

    fn wait_stopped(meter: &MicrophoneMeter) {
        let until = Instant::now() + Duration::from_secs(2);
        while meter.snapshot().active && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!meter.snapshot().active);
    }

    #[test]
    fn startup_timeout_lifetime_and_drop_stop_only_owned_fake_worker() {
        let mut timeout = MicrophoneMeter::spawn(
            |stop, _| {
                while !stop.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(2));
                }
                Ok(())
            },
            Duration::from_millis(30),
            Duration::from_secs(1),
        )
        .unwrap();
        wait_stopped(&timeout);
        assert!(timeout.snapshot().error.unwrap().contains("ready"));
        timeout.stop().unwrap();
        let (sent, received) = mpsc::channel();
        let mut lifetime = MicrophoneMeter::spawn(
            move |stop, state| {
                state.lock().unwrap().snapshot.ready = true;
                while !stop.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(2));
                }
                sent.send(()).unwrap();
                Ok(())
            },
            Duration::from_secs(1),
            Duration::from_millis(35),
        )
        .unwrap();
        wait_stopped(&lifetime);
        assert!(lifetime.snapshot().error.is_none());
        lifetime.stop().unwrap();
        received.recv_timeout(Duration::from_secs(1)).unwrap();
        let (sent, received) = mpsc::channel();
        let meter = MicrophoneMeter::spawn(
            move |stop, _| {
                while !stop.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(2));
                }
                sent.send(()).unwrap();
                Ok(())
            },
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        drop(meter);
        received.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn native_failure_and_panic_are_visible_without_opening_a_device() {
        for panic in [false, true] {
            let mut meter = MicrophoneMeter::spawn(
                move |_, _| {
                    assert!(!panic, "injected worker panic");
                    Err("Injected unplug".into())
                },
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
            .unwrap();
            wait_stopped(&meter);
            assert!(meter.snapshot().error.is_some());
            meter.stop().unwrap();
        }
        assert!(MicrophoneMeter::start("").is_err());
    }

    #[test]
    fn blocked_native_cleanup_returns_at_deadline_and_retains_worker_ownership() {
        let (release, blocked) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        let mut meter = MicrophoneMeter::spawn(
            move |_, _| {
                blocked.recv().unwrap();
                done.send(()).unwrap();
                Ok(())
            },
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let started = Instant::now();
        assert!(meter.stop_with_deadline(Duration::from_millis(20)).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!meter.snapshot().active);
        release.send(()).unwrap();
        finished.recv_timeout(Duration::from_secs(1)).unwrap();
    }
}
