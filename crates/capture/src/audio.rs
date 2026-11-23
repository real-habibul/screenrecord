use screenrecord_core::{Device, RecordingOptions, SystemAudioScope};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use wasapi::{BufferInfo, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

mod application;

const RATE: u32 = 48_000;
const CHANNELS: usize = 2;

#[derive(Debug, PartialEq)]
enum Source {
    Endpoint(Option<String>),
    Application(crate::apps::AppWindow),
}

fn system_source(options: &RecordingOptions) -> Result<Source, String> {
    if !options.system_audio {
        return Err("Audio recording is disabled.".into());
    }
    match options.system_audio_scope {
        SystemAudioScope::System => Ok(Source::Endpoint(None)),
        SystemAudioScope::Application => {
            crate::apps::AppWindow::parse(&options.source_id).map(Source::Application)
        }
    }
}

pub(crate) struct Com;
impl Com {
    pub(crate) fn initialize() -> Result<Self, String> {
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

pub fn microphones() -> Result<Vec<Device>, String> {
    thread::spawn(|| {
        let _com = Com::initialize()?;
        let enumerator = DeviceEnumerator::new().map_err(|error| error.to_string())?;
        let collection = enumerator
            .get_device_collection(Direction::Capture)
            .map_err(|error| error.to_string())?;
        let mut devices = Vec::new();
        for item in &collection {
            let item = item.map_err(|error| error.to_string())?;
            devices.push(Device {
                id: item.get_id().map_err(|error| error.to_string())?,
                name: item.get_friendlyname().map_err(|error| error.to_string())?,
            });
        }
        Ok(devices)
    })
    .join()
    .map_err(|_| "Audio enumeration thread failed.".to_string())?
}

pub struct AudioWorker {
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl AudioWorker {
    pub fn start(
        path: &Path,
        device_id: Option<String>,
        epoch: Arc<AtomicI64>,
        stop: Arc<AtomicBool>,
        end: Arc<AtomicI64>,
    ) -> Result<Self, String> {
        Self::start_source(path, Source::Endpoint(device_id), epoch, stop, end)
    }

    pub(crate) fn start_system(
        path: &Path,
        options: &RecordingOptions,
        epoch: Arc<AtomicI64>,
        stop: Arc<AtomicBool>,
        end: Arc<AtomicI64>,
    ) -> Result<Self, String> {
        Self::start_source(path, system_source(options)?, epoch, stop, end)
    }

    fn start_source(
        path: &Path,
        source: Source,
        epoch: Arc<AtomicI64>,
        stop: Arc<AtomicBool>,
        end: Arc<AtomicI64>,
    ) -> Result<Self, String> {
        let path = path.to_owned();
        let worker_path = path.clone();
        let stop_on_error = stop.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                let result = capture(&worker_path, source, epoch, stop, end, &ready_tx);
                if let Err(error) = &result {
                    let _ = ready_tx.try_send(Err(error.clone()));
                }
                result
            })
            .map_err(|error| error.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self {
                thread: Some(worker),
            }),
            Ok(Err(error)) => {
                stop_on_error.store(true, Ordering::Release);
                finish_failed_start(&path, worker, error)
            }
            Err(_) => {
                stop_on_error.store(true, Ordering::Release);
                finish_failed_start(
                    &path,
                    worker,
                    "Audio device did not become ready within 10 seconds.".into(),
                )
            }
        }
    }

    pub fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(|worker| worker.is_finished())
    }

    pub fn finish(&mut self) -> Result<(), String> {
        if let Some(worker) = self.thread.take() {
            worker
                .join()
                .map_err(|_| "Audio worker panicked.".to_string())??;
        }
        Ok(())
    }
}

fn finish_failed_start(
    path: &Path,
    worker: JoinHandle<Result<(), String>>,
    error: String,
) -> Result<AudioWorker, String> {
    let joined = crate::cleanup::run(
        path.parent().unwrap_or(path),
        crate::cleanup::DEADLINE,
        move || {
            worker
                .join()
                .map_err(|_| "Audio worker panicked.".to_owned())?
        },
    );
    Err(match joined {
        Ok(()) => error,
        Err(cleanup_error) => format!("{error} {cleanup_error}"),
    })
}

fn capture(
    path: &Path,
    source: Source,
    epoch: Arc<AtomicI64>,
    stop: Arc<AtomicBool>,
    end: Arc<AtomicI64>,
    ready: &mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let _com = Com::initialize()?;
    let target = match source {
        Source::Application(app) => Some(application::Target::open(app)?),
        Source::Endpoint(_) => None,
    };
    let mut client = match &source {
        Source::Application(app) => application::activate(app.pid, &stop, ready)?,
        Source::Endpoint(device_id) => {
            let enumerator = DeviceEnumerator::new().map_err(|error| error.to_string())?;
            let device = match device_id {
                Some(id) => enumerator.get_device(id),
                None => enumerator.get_default_device(Direction::Render),
            }
            .map_err(|error| format!("Audio device is unavailable: {error}"))?;
            device
                .get_iaudioclient()
                .map_err(|error| error.to_string())?
        }
    };
    let format = WaveFormat::new(32, 32, SampleType::Float, RATE as usize, CHANNELS, None);
    let mode = if target.is_some() {
        // Process loopback has no physical endpoint or usable device-period query.
        StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 0,
        }
    } else {
        let (period, _) = client
            .get_device_period()
            .map_err(|error| error.to_string())?;
        StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: period.max(2_000_000),
        }
    };
    client
        .initialize_client(&format, Direction::Capture, mode, None)
        .map_err(|error| format!("Cannot initialize audio recording: {error}"))?;
    let event = if target.is_some() {
        Some(application::create_event(&client)?)
    } else {
        None
    };
    let capture = client
        .get_audiocaptureclient()
        .map_err(|error| error.to_string())?;
    let writer = PacketWriter::start(path, epoch.clone(), end.clone())?;
    let mut buffer = vec![0u8; 4096 * CHANNELS * 4];
    let mut clock = PacketClock::default();
    if let Some(target) = &target {
        target.validate()?;
    }
    if stop.load(Ordering::Acquire) {
        return Err("Audio startup was cancelled.".into());
    }
    client.start_stream().map_err(|error| error.to_string())?;
    let mut drain = || {
        for _ in 0..256 {
            let frames = capture
                .get_next_packet_size()
                .map_err(|error| error.to_string())?
                .unwrap_or(0);
            if frames == 0 {
                return Ok(());
            }
            ensure_packet_buffer(&mut buffer, frames)?;
            let (count, info) = capture
                .read_from_device(&mut buffer)
                .map_err(|error| format!("Audio device stopped responding: {error}"))?;
            let start = epoch.load(Ordering::Acquire);
            if start == 0 || count == 0 {
                continue;
            }
            let packet_start = clock.position(&info, count, start)?;
            writer.enqueue(&buffer[..count as usize * 8], packet_start)?;
        }
        Err("Audio packet queue did not drain within its packet limit.".to_owned())
    };
    let result = (|| {
        ready.send(Ok(())).map_err(|error| error.to_string())?;
        let mut last_identity_check = Instant::now();
        while !stop.load(Ordering::Acquire) {
            if last_identity_check.elapsed() >= Duration::from_millis(250) {
                if let Some(target) = &target {
                    target.validate()?;
                }
                last_identity_check = Instant::now();
            }
            drain()?;
            if writer.is_finished() {
                return Err("Audio disk writer stopped before recording ended.".to_owned());
            }
            if let Some(event) = &event {
                application::wait_event(event)?;
            } else {
                thread::sleep(Duration::from_millis(4));
            }
        }
        Ok(())
    })();
    let stopped = stop_and_drain(
        || client.stop_stream().map_err(|error| error.to_string()),
        drain,
    );
    let saved = writer.finish();
    result.and(stopped).and(saved)
}

fn ensure_packet_buffer(buffer: &mut Vec<u8>, frames: u32) -> Result<(), String> {
    if frames > RATE * 2 {
        return Err("Audio packet exceeds the two-second memory limit.".into());
    }
    let bytes = frames as usize * CHANNELS * 4;
    if bytes > buffer.len() {
        buffer.resize(bytes, 0);
    }
    Ok(())
}

fn stop_and_drain(
    stop: impl FnOnce() -> Result<(), String>,
    drain: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let stopped = stop();
    let drained = drain();
    stopped.and(drained)
}

struct AudioPacket {
    bytes: Vec<u8>,
    position: i64,
}

struct PacketWriter {
    sender: Option<mpsc::SyncSender<AudioPacket>>,
    worker: Option<JoinHandle<Result<(), String>>>,
    queued_bytes: Arc<AtomicUsize>,
}

const MAX_QUEUED_BYTES: usize = RATE as usize * 8 * 2;

impl PacketWriter {
    fn start(path: &Path, epoch: Arc<AtomicI64>, end: Arc<AtomicI64>) -> Result<Self, String> {
        let writer = crate::wav::PcmWriter::create(path).map_err(|error| error.to_string())?;
        let (sender, receiver) = mpsc::sync_channel::<AudioPacket>(256);
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let pending = queued_bytes.clone();
        let worker = thread::Builder::new()
            .name("audio-disk-writer".into())
            .spawn(move || write_queued(writer, receiver, pending, epoch, end))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            queued_bytes,
        })
    }

    fn enqueue(&self, bytes: &[u8], position: i64) -> Result<(), String> {
        enqueue_packet(
            self.sender
                .as_ref()
                .ok_or("Audio writer has already stopped.")?,
            &self.queued_bytes,
            bytes,
            position,
        )
    }

    fn is_finished(&self) -> bool {
        self.worker
            .as_ref()
            .is_none_or(|worker| worker.is_finished())
    }

    fn finish(mut self) -> Result<(), String> {
        self.join()
    }

    fn join(&mut self) -> Result<(), String> {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "Audio disk writer panicked.".to_owned())??;
        }
        Ok(())
    }
}

impl Drop for PacketWriter {
    fn drop(&mut self) {
        let _ = self.join();
    }
}

fn enqueue_packet(
    sender: &mpsc::SyncSender<AudioPacket>,
    queued: &AtomicUsize,
    bytes: &[u8],
    position: i64,
) -> Result<(), String> {
    let overrun =
        "Audio storage could not keep up with recording; the two-second disk queue is full.";
    queued
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current
                .checked_add(bytes.len())
                .filter(|total| *total <= MAX_QUEUED_BYTES)
        })
        .map_err(|_| overrun.to_owned())?;
    let packet = AudioPacket {
        bytes: bytes.to_vec(),
        position,
    };
    if let Err(error) = sender.try_send(packet) {
        queued.fetch_sub(bytes.len(), Ordering::AcqRel);
        return Err(match error {
            mpsc::TrySendError::Full(_) => overrun.to_owned(),
            mpsc::TrySendError::Disconnected(_) => "Audio disk writer stopped unexpectedly.".into(),
        });
    }
    Ok(())
}

fn write_queued(
    mut writer: crate::wav::PcmWriter,
    receiver: mpsc::Receiver<AudioPacket>,
    queued: Arc<AtomicUsize>,
    epoch: Arc<AtomicI64>,
    end: Arc<AtomicI64>,
) -> Result<(), String> {
    let mut written = 0;
    let mut flushed = Instant::now();
    let result = (|| {
        loop {
            match receiver.recv_timeout(Duration::from_millis(250)) {
                Ok(packet) => {
                    let result = write_packet(
                        &mut writer,
                        &mut written,
                        &packet.bytes,
                        (packet.bytes.len() / 8) as u32,
                        packet.position,
                    );
                    queued.fetch_sub(packet.bytes.len(), Ordering::AcqRel);
                    result?;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if flushed.elapsed() >= Duration::from_secs(1) {
                writer.flush().map_err(|error| error.to_string())?;
                flushed = Instant::now();
            }
        }
        let start = epoch.load(Ordering::Acquire);
        let finish = end.load(Ordering::Acquire);
        if start > 0 && finish > start {
            let frames = qpc_to_frame(finish - start).max(0) as u64;
            writer
                .truncate_frames(frames)
                .map_err(|error| error.to_string())?;
            written = written.min(frames);
            pad(&mut writer, &mut written, frames)?;
        }
        Ok(())
    })();
    let finalized = writer.finalize().map_err(|error| error.to_string());
    result.and(finalized)
}

#[derive(Default)]
struct PacketClock {
    next_device_frame: Option<u64>,
    next_output_frame: i64,
}

impl PacketClock {
    fn position(&mut self, info: &BufferInfo, count: u32, epoch: i64) -> Result<i64, String> {
        let timestamp_position = || {
            if info.flags.timestamp_error || info.timestamp == 0 || info.timestamp > i64::MAX as u64
            {
                return Err(
                    "Audio device reported an invalid timestamp at a stream boundary.".to_owned(),
                );
            }
            Ok(qpc_to_frame(info.timestamp as i64 - epoch))
        };
        let position = match self.next_device_frame {
            None => timestamp_position()?,
            Some(expected) if info.flags.data_discontinuity => {
                let timestamp_frame = timestamp_position()?;
                if info.index == expected
                    && timestamp_frame.abs_diff(self.next_output_frame) <= u64::from(RATE / 1000)
                {
                    self.next_output_frame
                } else {
                    timestamp_frame
                }
            }
            Some(expected) => {
                let gap = info.index.checked_sub(expected).ok_or(
                    "Audio device sample position moved backwards without a stream discontinuity.",
                )?;
                let gap = i64::try_from(gap).map_err(|error| error.to_string())?;
                // QPC jitter must never insert zeros or remove samples inside a continuous stream.
                self.next_output_frame
                    .checked_add(gap)
                    .ok_or("Audio sample position overflowed.")?
            }
        };
        self.next_device_frame = Some(
            info.index
                .checked_add(u64::from(count))
                .ok_or("Audio device sample position overflowed.")?,
        );
        self.next_output_frame = position
            .checked_add(i64::from(count))
            .ok_or("Audio sample position overflowed.")?;
        Ok(position)
    }
}

fn write_packet(
    writer: &mut crate::wav::PcmWriter,
    written_frames: &mut u64,
    buffer: &[u8],
    count: u32,
    packet_start: i64,
) -> Result<(), String> {
    let packet_end = packet_start
        .checked_add(i64::from(count))
        .ok_or("Audio packet position overflowed.")?;
    if packet_end <= *written_frames as i64 {
        return Ok(());
    }
    if packet_start > *written_frames as i64 {
        pad(writer, written_frames, packet_start as u64)?;
    }
    let skip = (*written_frames as i64 - packet_start).max(0) as usize;
    let samples = buffer
        .get(skip.min(count as usize) * 8..count as usize * 8)
        .ok_or("Audio packet is shorter than its reported sample count.")?;
    let clean;
    let samples = if samples
        .as_chunks::<4>()
        .0
        .iter()
        .all(|sample| f32::from_le_bytes(*sample).is_finite())
    {
        samples
    } else {
        clean = samples
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|sample| {
                let value = f32::from_le_bytes(*sample);
                if value.is_finite() {
                    value.to_le_bytes()
                } else {
                    0f32.to_le_bytes()
                }
            })
            .collect::<Vec<_>>();
        &clean
    };
    writer
        .write_frames(samples)
        .map_err(|error| error.to_string())?;
    *written_frames = (*written_frames).max(packet_end.max(0) as u64);
    Ok(())
}

fn qpc_to_frame(ticks: i64) -> i64 {
    ((ticks as i128 * RATE as i128) / 10_000_000) as i64
}

fn pad(writer: &mut crate::wav::PcmWriter, written: &mut u64, target: u64) -> Result<(), String> {
    if target.saturating_sub(*written) > RATE as u64 * 60 * 60 {
        return Err("Audio timestamp discontinuity exceeded one hour.".into());
    }
    let zeros = [0u8; 8192];
    let mut remaining = target.saturating_sub(*written);
    while remaining > 0 {
        let frames = remaining.min(1024);
        writer
            .write_frames(&zeros[..frames as usize * 8])
            .map_err(|error| error.to_string())?;
        remaining -= frames;
    }
    *written = (*written).max(target);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn failed_audio_during_screen_startup_preserves_the_error_and_never_acknowledges() {
        use crate::{startup, Journal, Recorder};
        use std::sync::atomic::{AtomicBool, AtomicI64};
        use std::sync::{mpsc, Arc};
        use std::time::Duration;

        for queued_frame in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let audio = std::thread::spawn(|| Err("injected WASAPI device loss".into()));
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !audio.is_finished() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            let mut recorder = Recorder {
                directory: directory.path().to_owned(),
                ffmpeg: "unused".into(),
                capture: None,
                microphone: None,
                system_audio: Some(super::AudioWorker {
                    thread: Some(audio),
                }),
                camera: None,
                cursor: None,
                maintenance: None,
                epoch: Arc::new(AtomicI64::new(0)),
                end: Arc::new(AtomicI64::new(0)),
                stop: Arc::new(AtomicBool::new(false)),
                journal: Journal {
                    version: 1,
                    source_id: "monitor:1".into(),
                    width: 320,
                    height: 180,
                    fps: 30,
                    epoch_qpc: 0,
                    camera_epoch_qpc: None,
                    camera_offset_seconds: None,
                    duration: 0.0,
                    microphone: false,
                    system_audio: true,
                    segments: vec![],
                    segment_durations: vec![],
                },
            };
            let (sender, receiver) = mpsc::sync_channel(1);
            if queued_frame {
                sender.send(Ok((320, 180))).unwrap();
            }
            let result = recorder.wait_for_screen_frame(
                &receiver,
                &startup::State::default(),
                true,
                Duration::from_millis(80),
                None,
            );
            assert!(
                result
                    .as_ref()
                    .is_err_and(|message| message.contains("System audio")
                        && message.contains("injected WASAPI device loss")),
                "{result:?}"
            );
        }
    }
    use super::*;

    const EPOCH: i64 = 100_000_000;

    #[test]
    fn application_scope_never_selects_an_endpoint_or_accepts_non_app_identity() {
        let mut options = RecordingOptions {
            source_id: "app:42:900".into(),
            fps: 30,
            microphone_id: None,
            system_audio: true,
            system_audio_scope: SystemAudioScope::Application,
            camera_id: None,
            region: None,
        };
        assert_eq!(
            system_source(&options).unwrap(),
            Source::Application(crate::apps::AppWindow {
                pid: 42,
                handle: 900
            })
        );
        for source in [
            "monitor:900",
            "window:900",
            "app:0:900",
            "app:42:0",
            "app:042:900",
            "app:42:900:1",
            "app:4294967296:900",
        ] {
            options.source_id = source.into();
            assert!(system_source(&options).is_err(), "{source}");
        }
        options.source_id = "app:42:900".into();
        options.system_audio_scope = SystemAudioScope::System;
        assert_eq!(system_source(&options).unwrap(), Source::Endpoint(None));
        options.system_audio = false;
        assert!(system_source(&options).is_err());
        options.system_audio_scope = SystemAudioScope::Application;
        assert!(system_source(&options).is_err());
    }

    #[test]
    fn packet_memory_grows_only_with_bounded_packets() {
        let mut buffer = vec![0; 4096 * 8];
        ensure_packet_buffer(&mut buffer, 480).unwrap();
        assert_eq!(buffer.len(), 4096 * 8);
        ensure_packet_buffer(&mut buffer, RATE * 2).unwrap();
        assert_eq!(buffer.len(), RATE as usize * 2 * 8);
        assert!(ensure_packet_buffer(&mut buffer, RATE * 2 + 1).is_err());
        assert!(ensure_packet_buffer(&mut buffer, u32::MAX).is_err());
        assert_eq!(buffer.len(), RATE as usize * 2 * 8);
    }

    #[test]
    fn silent_application_produces_exact_recording_duration_without_packets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("silent-application.wav");
        let writer = PacketWriter::start(
            &path,
            Arc::new(AtomicI64::new(EPOCH)),
            Arc::new(AtomicI64::new(EPOCH + 2_500_000)),
        )
        .unwrap();
        writer.finish().unwrap();
        let samples = read_samples(&path);
        assert_eq!(samples.len(), 12_000 * CHANNELS);
        assert!(samples.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn blocked_disk_queue_is_bounded_without_blocking_acquisition() {
        let (sender, receiver) = mpsc::sync_channel(256);
        let queued = AtomicUsize::new(0);
        let bytes = [0u8; 480 * 8];
        let started = Instant::now();
        for index in 0..200 {
            enqueue_packet(&sender, &queued, &bytes, index * 480).unwrap();
        }
        assert_eq!(queued.load(Ordering::Acquire), MAX_QUEUED_BYTES);
        assert!(enqueue_packet(&sender, &queued, &bytes, 96_000)
            .unwrap_err()
            .contains("two-second"));
        assert_eq!(queued.load(Ordering::Acquire), MAX_QUEUED_BYTES);
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(receiver.try_iter().count(), 200);
        drop(receiver);
        queued.store(0, Ordering::Release);
        assert!(enqueue_packet(&sender, &queued, &bytes, 0)
            .unwrap_err()
            .contains("unexpectedly"));
        assert_eq!(queued.load(Ordering::Acquire), 0);
    }

    #[test]
    fn blocked_writer_resumes_and_saves_every_accepted_packet() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("delayed-disk.wav");
        let file = crate::wav::PcmWriter::create(&path).unwrap();
        let (sender, receiver) = mpsc::sync_channel(256);
        let (release, gate) = mpsc::sync_channel(1);
        let queued = Arc::new(AtomicUsize::new(0));
        let pending = queued.clone();
        let worker = thread::spawn(move || {
            gate.recv().unwrap();
            write_queued(
                file,
                receiver,
                pending,
                Arc::new(AtomicI64::new(EPOCH)),
                Arc::new(AtomicI64::new(0)),
            )
        });
        let writer = PacketWriter {
            sender: Some(sender),
            worker: Some(worker),
            queued_bytes: queued.clone(),
        };
        for index in 0..200 {
            let bytes: Vec<u8> =
                std::iter::repeat_n((index as f32 / 200.0).to_le_bytes(), 480 * CHANNELS)
                    .flatten()
                    .collect();
            writer.enqueue(&bytes, index * 480).unwrap();
        }
        assert!(writer.enqueue(&[0u8; 8], 96_000).is_err());
        assert_eq!(queued.load(Ordering::Acquire), MAX_QUEUED_BYTES);
        release.send(()).unwrap();
        writer.finish().unwrap();
        assert_eq!(queued.load(Ordering::Acquire), 0);
        let samples = read_samples(&path);
        assert_eq!(samples.len(), 96_000 * CHANNELS);
        for (index, packet) in samples
            .as_chunks::<{ 480 * CHANNELS }>()
            .0
            .iter()
            .enumerate()
        {
            assert!(packet.iter().all(|sample| *sample == index as f32 / 200.0));
        }
    }

    #[test]
    fn stop_drains_real_tail_then_trims_at_the_requested_frame() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tail.wav");
        let epoch = Arc::new(AtomicI64::new(EPOCH));
        let end = Arc::new(AtomicI64::new(EPOCH + 720 * 10_000_000 / i64::from(RATE)));
        let writer = PacketWriter::start(&path, epoch, end).unwrap();
        writer.enqueue(&[0u8; 480 * 8], 0).unwrap();
        let stopped = std::cell::Cell::new(false);
        stop_and_drain(
            || {
                stopped.set(true);
                Ok(())
            },
            || {
                assert!(stopped.get());
                let mut tail = vec![0u8; 480 * 8];
                for index in [200usize, 239, 240, 479] {
                    tail[index * 8..index * 8 + 4].copy_from_slice(&0.75f32.to_le_bytes());
                }
                writer.enqueue(&tail, 480)
            },
        )
        .unwrap();
        writer.finish().unwrap();
        let samples = read_samples(&path);
        assert_eq!(samples.len(), 720 * 2);
        assert_eq!(samples[680 * 2], 0.75);
        assert_eq!(samples[719 * 2], 0.75);
        assert_eq!(samples.iter().filter(|sample| **sample != 0.0).count(), 2);
        assert!(crate::wav::readable(&path).unwrap());
    }

    fn packet(index: u64, frame: i64, flags: u32) -> BufferInfo {
        BufferInfo::new(
            flags,
            index,
            (EPOCH + frame * 10_000_000 / i64::from(RATE)) as u64,
        )
    }

    fn read_samples(path: &Path) -> Vec<f32> {
        hound::WavReader::open(path)
            .unwrap()
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn sample_clock_uses_signed_absolute_qpc() {
        assert_eq!(qpc_to_frame(10_000_000), 48_000);
        assert_eq!(qpc_to_frame(-5_000_000), -24_000);
    }

    #[test]
    fn jittery_packet_timestamps_preserve_the_entire_tone_bit_for_bit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tone.wav");
        let mut writer = crate::wav::PcmWriter::create(&path).unwrap();
        let mut clock = PacketClock::default();
        let mut written = 0;
        let tone: Vec<f32> = (0..RATE * 2)
            .flat_map(|index| {
                let value =
                    (index as f32 * std::f32::consts::TAU * 997.0 / RATE as f32).sin() * 0.3;
                [value, -value]
            })
            .collect();
        for (index, samples) in tone.as_chunks::<{ 480 * CHANNELS }>().0.iter().enumerate() {
            let frame = index as i64 * 480;
            let jitter = if index == 0 {
                0
            } else {
                (index as i64 * 19 % 67) - 33
            };
            let slow_drift = index as i64 / 25;
            let flags = if index > 0 && index % 17 == 0 { 4 } else { 0 };
            let info = packet(1_000_000 + frame as u64, frame + jitter + slow_drift, flags);
            let position = clock.position(&info, 480, EPOCH).unwrap();
            let bytes: Vec<u8> = samples
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            write_packet(&mut writer, &mut written, &bytes, 480, position).unwrap();
        }
        writer.finalize().unwrap();
        assert_eq!(written, u64::from(RATE) * 2);
        assert_eq!(read_samples(&path), tone);
    }

    #[test]
    fn genuine_lost_packets_and_silent_packets_keep_their_timeline() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gaps.wav");
        let mut writer = crate::wav::PcmWriter::create(&path).unwrap();
        let mut clock = PacketClock::default();
        let mut written = 0;
        for (info, value) in [
            (packet(5000, -240, 0), 0.25f32),
            (packet(5480, 239, 0), 0.5),
            (packet(6920, 1683, 0), 0.75),
            (packet(7400, 2160, 2), 0.0),
            (packet(0, 4800, 1), -0.5),
        ] {
            let position = clock.position(&info, 480, EPOCH).unwrap();
            let bytes: Vec<u8> = std::iter::repeat_n(value.to_le_bytes(), 480 * CHANNELS)
                .flatten()
                .collect();
            write_packet(&mut writer, &mut written, &bytes, 480, position).unwrap();
        }
        writer.finalize().unwrap();
        let samples = read_samples(&path);
        for (frames, value) in [
            (0..240, 0.25),
            (240..720, 0.5),
            (720..1680, 0.0),
            (1680..2160, 0.75),
            (2160..4800, 0.0),
            (4800..5280, -0.5),
        ] {
            assert!(samples[frames.start * 2..frames.end * 2]
                .iter()
                .all(|sample| *sample == value));
        }
        assert_eq!(written, 5280);
    }

    #[test]
    fn invalid_boundary_timestamps_and_unannounced_clock_resets_fail() {
        let mut clock = PacketClock::default();
        assert!(clock.position(&packet(0, 0, 4), 480, EPOCH).is_err());
        assert_eq!(clock.position(&packet(0, 0, 0), 480, EPOCH).unwrap(), 0);
        assert_eq!(
            clock.position(&packet(480, 50_000, 4), 480, EPOCH).unwrap(),
            480
        );
        assert!(clock
            .position(&packet(960, 960, 1 | 4), 480, EPOCH)
            .is_err());
        assert!(clock.position(&packet(0, 960, 0), 480, EPOCH).is_err());
    }

    #[test]
    fn discontinuity_flag_does_not_turn_small_timestamp_jitter_into_a_hole() {
        let mut clock = PacketClock::default();
        assert_eq!(clock.position(&packet(0, 0, 1), 480, EPOCH).unwrap(), 0);
        assert_eq!(
            clock.position(&packet(480, 484, 1), 480, EPOCH).unwrap(),
            480
        );
    }
}
