use screenrecord_capture::AudioWorker;

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

struct Com;
impl Com {
    fn initialize() -> Result<Self, String> {
        wasapi::initialize_mta()
            .ok()
            .map_err(|error| error.to_string())?;
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() };
    }
}

fn qpc_now() -> i64 {
    let mut ticks = 0;
    let mut frequency = 0;
    unsafe {
        QueryPerformanceCounter(&mut ticks).unwrap();
        QueryPerformanceFrequency(&mut frequency).unwrap();
    }
    (i128::from(ticks) * 10_000_000 / i128::from(frequency)) as i64
}

fn main() -> Result<(), String> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("Usage: loopback-tone <new-output.wav>. Plays a quiet 997 Hz tone and captures only computer audio for at most 5.5 seconds.")?;
    if path.exists() {
        return Err("Output already exists.".into());
    }
    let _com = Com::initialize()?;
    let device = DeviceEnumerator::new()
        .and_then(|enumerator| enumerator.get_default_device(Direction::Render))
        .map_err(|error| error.to_string())?;
    let mut client = device
        .get_iaudioclient()
        .map_err(|error| error.to_string())?;
    let format = WaveFormat::new(32, 32, SampleType::Float, 48_000, 2, None);
    client
        .initialize_client(
            &format,
            Direction::Render,
            StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: 1_000_000,
            },
            None,
        )
        .map_err(|error| error.to_string())?;
    let capacity = client
        .get_buffer_size()
        .map_err(|error| error.to_string())?;
    let render = client
        .get_audiorenderclient()
        .map_err(|error| error.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let end = Arc::new(AtomicI64::new(0));
    let epoch = Arc::new(AtomicI64::new(qpc_now()));
    let watchdog_stop = stop.clone();
    let watchdog_end = end.clone();
    let watchdog = thread::spawn(move || {
        thread::sleep(Duration::from_millis(5500));
        if !watchdog_stop.load(Ordering::Acquire) {
            watchdog_end.store(qpc_now(), Ordering::Release);
            watchdog_stop.store(true, Ordering::Release);
        }
    });
    let mut worker = AudioWorker::start(&path, None, epoch, stop.clone(), end.clone())?;
    let result = (|| -> Result<(), String> {
        thread::sleep(Duration::from_millis(150));
        client.start_stream().map_err(|error| error.to_string())?;
        let start = Instant::now();
        let mut frame = 0u64;
        while start.elapsed() < Duration::from_millis(4200) && !stop.load(Ordering::Acquire) {
            let padding = client
                .get_current_padding()
                .map_err(|error| error.to_string())?;
            let available = capacity.saturating_sub(padding);
            if available > 0 {
                let mut data = Vec::with_capacity(available as usize * 8);
                for _ in 0..available {
                    let value = (frame as f64 * std::f64::consts::TAU * 997.0 / 48_000.0).sin()
                        as f32
                        * 0.031_622_775;
                    data.extend_from_slice(&value.to_le_bytes());
                    data.extend_from_slice(&value.to_le_bytes());
                    frame += 1;
                }
                render
                    .write_to_device(&data, None)
                    .map_err(|error| error.to_string())?;
            }
            thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    })();
    let _ = client.stop_stream();
    end.store(qpc_now(), Ordering::Release);
    stop.store(true, Ordering::Release);
    let capture_result = worker.finish();
    watchdog
        .join()
        .map_err(|_| "Loopback deadline thread failed.")?;
    result?;
    capture_result?;
    println!(
        "{}",
        serde_json::json!({"path":path,"frequencyHz":997,"peakDbfs":-30,"captureStopped":true})
    );
    Ok(())
}
