pub mod analysis;
mod apps;
mod audio;
mod camera;
mod cleanup;
mod cursor;
mod media;
#[cfg(test)]
mod media_tests;
pub mod meter;
mod mouse;
pub mod preview;
mod process;
mod region;
pub mod session;
mod source_thumbnail;
mod startup;
pub mod tracking;
mod wav;

#[doc(hidden)]
pub use audio::AudioWorker;
use media::RecoveryError;
use screenrecord_core::{
    CaptureDevices, CaptureRegion, CaptureSource, CursorSample, RecordingOptions,
};
use serde::{Deserialize, Serialize};
pub use source_thumbnail::{source_thumbnail, SourceThumbnail};
pub use startup::never_started_screen;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::{
        Direct3D11::{
            ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
            D3D11_BIND_RENDER_TARGET, D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE,
            D3D11_MAP_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
        },
        Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
        Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO},
    },
    System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
    UI::HiDpi::{
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    },
};
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    d3d11::SendDirectX,
    encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
        VideoSettingsSubType,
    },
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
};

pub struct CaptureResult {
    pub screen_path: PathBuf,
    pub microphone_path: Option<PathBuf>,
    pub system_audio_path: Option<PathBuf>,
    pub camera_path: Option<PathBuf>,
    pub camera_offset_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration: f64,
    pub cursor: Vec<CursorSample>,
}

pub fn cleanup_in_progress() -> bool {
    cleanup::in_progress()
}

pub fn validate_source(id: &str) -> Result<(), String> {
    validated_target(id).map(|_| ())
}

pub fn validate_recording_source(options: &RecordingOptions) -> Result<(), String> {
    options.validate()?;
    let (_, rect) = validated_target(&options.source_id)?;
    region::dimensions(
        (
            (rect.right as i64 - rect.left as i64) as u32,
            (rect.bottom as i64 - rect.top as i64) as u32,
        ),
        options.region,
    )?;
    Ok(())
}

fn validated_target(id: &str) -> Result<(Target, RECT), String> {
    let _dpi = PhysicalCoordinates::enter();
    let target = Target::parse(id)?;
    let rect = target.rect()?;
    validate_source_dimensions(&rect)?;
    Ok((target, rect))
}

fn validate_source_dimensions(rect: &RECT) -> Result<(), String> {
    let width = rect.right as i64 - rect.left as i64;
    let height = rect.bottom as i64 - rect.top as i64;
    if !(2..=16384).contains(&width) || !(2..=16384).contains(&height) {
        return Err("Selected source has invalid dimensions.".into());
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    version: u32,
    source_id: String,
    width: u32,
    height: u32,
    fps: u32,
    epoch_qpc: i64,
    camera_epoch_qpc: Option<i64>,
    #[serde(default)]
    camera_offset_seconds: Option<f64>,
    duration: f64,
    microphone: bool,
    system_audio: bool,
    segments: Vec<String>,
    #[serde(default)]
    segment_durations: Vec<f64>,
}

pub fn list_devices(ffmpeg_path: &Path) -> Result<CaptureDevices, String> {
    let _dpi = PhysicalCoordinates::enter();
    let mut sources = Vec::new();
    let mut app_windows = Vec::new();
    for monitor in Monitor::enumerate().map_err(|error| error.to_string())? {
        let width = monitor.width().map_err(|error| error.to_string())?;
        let height = monitor.height().map_err(|error| error.to_string())?;
        sources.push(CaptureSource {
            id: format!("monitor:{}", monitor.as_raw_hmonitor() as usize),
            title: format!(
                "{} ({width} x {height})",
                monitor
                    .name()
                    .ok()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| "Display".into())
            ),
            kind: "monitor".into(),
            width,
            height,
        });
    }
    for window in Window::enumerate().map_err(|error| error.to_string())? {
        let process_id = window.process_id().ok();
        if process_id == Some(std::process::id()) {
            continue;
        }
        let title = window.title().unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }
        let target = Target::Window(window.as_raw_hwnd() as usize);
        if let Ok(rect) = target.rect() {
            let width = (rect.right - rect.left).max(0) as u32;
            let height = (rect.bottom - rect.top).max(0) as u32;
            if width >= 2 && height >= 2 {
                let source = CaptureSource {
                    id: format!("window:{}", window.as_raw_hwnd() as usize),
                    title,
                    kind: "window".into(),
                    width,
                    height,
                };
                if let Some(pid) =
                    process_id.filter(|pid| *pid != 0 && window.process_id().ok() == Some(*pid))
                {
                    app_windows.push((
                        apps::AppWindow {
                            pid,
                            handle: window.as_raw_hwnd() as usize,
                        },
                        source.clone(),
                    ));
                }
                sources.push(source);
            }
        }
    }
    Ok(CaptureDevices {
        sources,
        apps: apps::group(app_windows, apps::executable_name),
        microphones: audio::microphones()?,
        cameras: camera::enumerate(ffmpeg_path)?,
    })
}

#[derive(Clone, Copy)]
enum Target {
    Monitor(usize),
    Window(usize),
    App(apps::AppWindow),
}

impl Target {
    fn parse(id: &str) -> Result<Self, String> {
        if id.starts_with("app:") {
            let app = apps::AppWindow::parse(id)?;
            app.validate()?;
            return Ok(Self::App(app));
        }
        let (kind, handle) = id
            .split_once(':')
            .ok_or("Invalid screen source identifier.")?;
        let handle = handle
            .parse::<usize>()
            .map_err(|_| "Invalid screen source identifier.")?;
        if handle == 0 {
            return Err("The capture source is no longer available.".into());
        }
        match kind {
            "monitor" => {
                if !Monitor::enumerate()
                    .map_err(|error| error.to_string())?
                    .iter()
                    .any(|item| item.as_raw_hmonitor() as usize == handle)
                {
                    return Err("The selected display is no longer connected.".into());
                }
                Ok(Self::Monitor(handle))
            }
            "window" => {
                if !Window::enumerate()
                    .map_err(|error| error.to_string())?
                    .iter()
                    .any(|item| item.as_raw_hwnd() as usize == handle)
                {
                    return Err("The selected window is no longer available.".into());
                }
                Ok(Self::Window(handle))
            }
            _ => Err("Unknown capture source kind.".into()),
        }
    }

    fn start(self, flags: CaptureFlags) -> Result<CaptureControl<ScreenCapture, String>, String> {
        match self {
            Self::Monitor(handle) => {
                start_capture(Monitor::from_raw_hmonitor(handle as *mut _), flags)
            }
            Self::Window(handle) => start_capture(Window::from_raw_hwnd(handle as *mut _), flags),
            Self::App(app) => {
                app.validate()?;
                start_capture(app, flags)
            }
        }
    }

    fn rect(self) -> Result<RECT, String> {
        match self {
            Self::App(app) => {
                let window = Window::from_raw_hwnd(app.handle as *mut _);
                if window.process_id().ok() != Some(app.pid) {
                    return Err("The selected window no longer belongs to this app.".into());
                }
                Self::Window(app.handle).rect()
            }
            Self::Monitor(handle) => {
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if unsafe { GetMonitorInfoW(HMONITOR(handle as *mut _), &mut info) }.as_bool() {
                    Ok(info.rcMonitor)
                } else {
                    Err("Cannot read the selected display bounds.".into())
                }
            }
            Self::Window(handle) => {
                let mut rect = RECT::default();
                unsafe {
                    DwmGetWindowAttribute(
                        HWND(handle as *mut _),
                        DWMWA_EXTENDED_FRAME_BOUNDS,
                        (&mut rect as *mut RECT).cast(),
                        std::mem::size_of::<RECT>() as u32,
                    )
                }
                .map_err(|error| error.to_string())?;
                Ok(rect)
            }
        }
    }
}

fn start_capture<T: TryInto<GraphicsCaptureItemType> + Send + 'static>(
    target: T,
    flags: CaptureFlags,
) -> Result<CaptureControl<ScreenCapture, String>, String> {
    let diagnostics = flags.native.clone();
    let settings = Settings::new(
        target,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    );
    ScreenCapture::start_free_threaded_with_diagnostics(settings, diagnostics)
        .map_err(|error| format!("Cannot start screen capture: {error}"))
}

struct CaptureFlags {
    directory: PathBuf,
    journal: Journal,
    epoch: Arc<AtomicI64>,
    cursor_canvas: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    end: Arc<AtomicI64>,
    ready: mpsc::SyncSender<Result<(u32, u32), String>>,
    startup: Arc<startup::State>,
    native: Arc<windows_capture::diagnostics::CaptureDiagnostics>,
    expected_app: Option<apps::AppWindow>,
    region: Option<CaptureRegion>,
}

struct ScreenCapture {
    encoder: Option<VideoEncoder>,
    flags: CaptureFlags,
    segment_start: i64,
    last_timestamp: i64,
    finalized: bool,
    snapshot: Option<SendDirectX<GpuSnapshot>>,
}

struct GpuSnapshot {
    texture: ID3D11Texture2D,
    view: ID3D11RenderTargetView,
    staging: Option<ID3D11Texture2D>,
    bytes: Vec<u8>,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    desc: D3D11_TEXTURE2D_DESC,
}

impl GpuSnapshot {
    fn new(frame: &Frame, width: u32, height: u32) -> Result<Self, String> {
        Self::from_source(
            frame.device(),
            frame.device_context(),
            *frame.desc(),
            width,
            height,
        )
    }

    fn from_source(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        mut desc: D3D11_TEXTURE2D_DESC,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        desc.Width = width;
        desc.Height = height;
        desc.Usage = D3D11_USAGE_DEFAULT;
        desc.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
        desc.CPUAccessFlags = 0;
        desc.MiscFlags = 0;
        let mut texture = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
            .map_err(|error| error.to_string())?;
        let texture = texture.ok_or("Cannot allocate a final-frame texture.")?;
        let mut view = None;
        unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut view)) }
            .map_err(|error| error.to_string())?;
        Ok(Self {
            texture,
            view: view.ok_or("Cannot initialize a final-frame texture.")?,
            staging: None,
            bytes: Vec::new(),
            device: device.clone(),
            context: context.clone(),
            desc,
        })
    }

    fn update(&self, frame: &Frame, region: Option<CaptureRegion>) -> Result<(), String> {
        self.update_texture(
            frame.as_raw_texture(),
            (frame.width(), frame.height()),
            region,
        )
    }

    fn update_texture(
        &self,
        texture: &ID3D11Texture2D,
        size: (u32, u32),
        region: Option<CaptureRegion>,
    ) -> Result<(), String> {
        let area = if let Some(region) = region {
            region.validate_bounds(size.0, size.1)?;
            if (region.width, region.height) != (self.desc.Width, self.desc.Height) {
                return Err("Capture area does not match the recording dimensions.".into());
            }
            D3D11_BOX {
                left: region.x,
                top: region.y,
                front: 0,
                right: region.x + region.width,
                bottom: region.y + region.height,
                back: 1,
            }
        } else {
            D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: size.0.min(self.desc.Width),
                bottom: size.1.min(self.desc.Height),
                back: 1,
            }
        };
        unsafe {
            self.context
                .ClearRenderTargetView(&self.view, &[0.0, 0.0, 0.0, 1.0]);
            self.context
                .CopySubresourceRegion(&self.texture, 0, 0, 0, 0, texture, 0, Some(&area));
        }
        Ok(())
    }

    fn bottom_up_buffer(&mut self) -> Result<&[u8], String> {
        if self.staging.is_none() {
            let mut desc = self.desc;
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            unsafe {
                self.device
                    .CreateTexture2D(&desc, None, Some(&mut self.staging))
            }
            .map_err(|error| error.to_string())?;
        }
        let staging = self
            .staging
            .as_ref()
            .ok_or("Cannot read the final video frame.")?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context.CopyResource(staging, &self.texture);
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|error| error.to_string())?;
        }
        let row_bytes = self.desc.Width as usize * 4;
        let height = self.desc.Height as usize;
        self.bytes.resize(row_bytes * height, 0);
        // MediaStreamSource's raw BGRA path expects bottom-up scanlines.
        for y in 0..height {
            let source = unsafe {
                std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    row_bytes,
                )
            };
            self.bytes[(height - y - 1) * row_bytes..(height - y) * row_bytes]
                .copy_from_slice(source);
        }
        unsafe {
            self.context.Unmap(staging, 0);
        }
        Ok(&self.bytes)
    }
}

impl ScreenCapture {
    fn begin_segment(&mut self, timestamp: i64) -> Result<(), String> {
        let path = self.flags.directory.join("segments").join(format!(
            "screen-{:05}.mp4",
            self.flags.journal.segments.len()
        ));
        let bitrate = (self.flags.journal.width as u64
            * self.flags.journal.height as u64
            * self.flags.journal.fps as u64
            / 4)
        .clamp(8_000_000, 60_000_000) as u32;
        self.encoder = Some(
            VideoEncoder::new(
                VideoSettingsBuilder::new(self.flags.journal.width, self.flags.journal.height)
                    .sub_type(VideoSettingsSubType::H264)
                    .frame_rate(self.flags.journal.fps)
                    .bitrate(bitrate),
                AudioSettingsBuilder::default().disabled(true),
                ContainerSettingsBuilder::default(),
                path,
            )
            .map_err(|error| format!("Cannot initialize the Windows video encoder: {error}"))?,
        );
        self.segment_start = timestamp;
        Ok(())
    }

    fn keep_static_frame(&mut self, timestamp: i64) -> Result<(), String> {
        if self.flags.stop.load(Ordering::Acquire)
            || self.finalized
            || self.snapshot.is_none()
            || self.encoder.is_none()
            || timestamp - self.last_timestamp < 5_000_000
        {
            return Ok(());
        }
        let buffer = self.snapshot.as_mut().unwrap().0.bottom_up_buffer()?;
        self.encoder
            .as_mut()
            .unwrap()
            .send_frame_buffer(buffer, timestamp)
            .map_err(|error| error.to_string())?;
        self.last_timestamp = timestamp;
        if timestamp - self.segment_start >= 120_000_000 {
            self.finish_segment((timestamp - self.flags.journal.epoch_qpc) as f64 / 10_000_000.0)?;
            self.begin_segment(timestamp)?;
            self.encoder
                .as_mut()
                .unwrap()
                .send_frame_buffer(&self.snapshot.as_ref().unwrap().0.bytes, timestamp)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn finish_segment(&mut self, duration: f64) -> Result<(), String> {
        if let Some(encoder) = self.encoder.take() {
            encoder
                .finish()
                .map_err(|error| format!("Cannot finalize screen recording: {error}"))?;
            self.flags.journal.segments.push(format!(
                "screen-{:05}.mp4",
                self.flags.journal.segments.len()
            ));
            self.flags.journal.segment_durations.push(
                (duration - self.flags.journal.duration).max(1.0 / self.flags.journal.fps as f64),
            );
            self.flags.journal.duration = duration;
            write_journal(&self.flags.directory, &self.flags.journal)?;
        }
        Ok(())
    }

    fn finish(&mut self, end: i64) -> Result<Journal, String> {
        if !self.finalized {
            if let (Some(snapshot), Some(encoder)) = (&mut self.snapshot, &mut self.encoder) {
                if end > self.last_timestamp {
                    encoder
                        .send_frame_buffer(snapshot.0.bottom_up_buffer()?, end)
                        .map_err(|error| error.to_string())?;
                    self.last_timestamp = end;
                }
            }
            let duration =
                (self.last_timestamp - self.flags.journal.epoch_qpc) as f64 / 10_000_000.0;
            self.finish_segment(duration.max(0.0))?;
            self.finalized = true;
        }
        Ok(self.flags.journal.clone())
    }

    fn process_frame(&mut self, frame: &mut Frame) -> Result<(), String> {
        let first = self.last_timestamp == 0;
        if first {
            if let Some(app) = self.flags.expected_app {
                app.validate()?;
            }
            self.flags.startup.set(startup::Stage::FrameReceived);
        }
        let selected = region::dimensions((frame.width(), frame.height()), self.flags.region)?;
        let timestamp = frame
            .timestamp()
            .map_err(|error| error.to_string())?
            .Duration;
        if self.last_timestamp > 0
            && timestamp - self.last_timestamp < 10_000_000 / self.flags.journal.fps as i64 - 5_000
        {
            return Ok(());
        }
        if self.flags.journal.epoch_qpc == 0 {
            self.flags.journal.width = selected.0;
            self.flags.journal.height = selected.1;
            self.flags.journal.epoch_qpc = timestamp;
            cursor::publish_start(
                &self.flags.cursor_canvas,
                &self.flags.epoch,
                self.flags.journal.width,
                self.flags.journal.height,
                timestamp,
            );
            write_journal(&self.flags.directory, &self.flags.journal)?;
        }
        if self.encoder.is_some() && timestamp - self.segment_start >= 120_000_000 {
            self.submit_frame(frame)?;
            self.finish_segment((timestamp - self.flags.journal.epoch_qpc) as f64 / 10_000_000.0)?;
        }
        if self.encoder.is_none() {
            if first {
                self.flags.startup.set(startup::Stage::EncoderStarting);
            }
            self.begin_segment(timestamp)?;
        }
        if first {
            self.flags.startup.set(startup::Stage::EncoderSubmission);
        }
        self.submit_frame(frame)?;
        if self.snapshot.is_none() {
            if first {
                self.flags.startup.set(startup::Stage::Snapshot);
            }
            self.snapshot = Some(SendDirectX::new(GpuSnapshot::new(
                frame,
                self.flags.journal.width,
                self.flags.journal.height,
            )?));
        }
        self.snapshot
            .as_ref()
            .unwrap()
            .0
            .update(frame, self.flags.region)?;
        self.last_timestamp = timestamp;
        if first {
            self.flags.startup.set(startup::Stage::Ready);
        }
        let _ = self
            .flags
            .ready
            .try_send(Ok((self.flags.journal.width, self.flags.journal.height)));
        Ok(())
    }

    fn submit_frame(&mut self, frame: &Frame) -> Result<(), String> {
        let encoder = self
            .encoder
            .as_mut()
            .ok_or("Screen encoder is not initialized.")?;
        let result = match self.flags.region {
            Some(region) => {
                encoder.send_frame_region(frame, region.x, region.y, region.width, region.height)
            }
            None => encoder.send_frame(frame),
        };
        result.map_err(|error| format!("Screen encoder failed: {error}"))
    }
}

impl GraphicsCaptureApiHandler for ScreenCapture {
    type Flags = CaptureFlags;
    type Error = String;
    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        if ctx.flags.stop.load(Ordering::Acquire) {
            return Err("Screen capture startup was stopped.".into());
        }
        if let Some(app) = ctx.flags.expected_app {
            app.validate()?;
        }
        Ok(Self {
            encoder: None,
            flags: ctx.flags,
            segment_start: 0,
            last_timestamp: 0,
            finalized: false,
            snapshot: None,
        })
    }
    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.flags.stop.load(Ordering::Acquire) {
            control.stop();
            return Ok(());
        }
        let result = self.process_frame(frame);
        if let Err(error) = &result {
            let _ = self.flags.ready.try_send(Err(error.clone()));
            control.stop();
        }
        result
    }
    fn on_closed(&mut self) -> Result<(), Self::Error> {
        let end = self.flags.end.load(Ordering::Acquire);
        self.finish(if end > 0 { end } else { qpc_now() })
            .map(|_| ())
    }
}

pub struct Recorder {
    directory: PathBuf,
    ffmpeg: PathBuf,
    capture: Option<CaptureControl<ScreenCapture, String>>,
    microphone: Option<audio::AudioWorker>,
    system_audio: Option<audio::AudioWorker>,
    camera: Option<camera::CameraWorker>,
    cursor: Option<JoinHandle<Result<Vec<CursorSample>, String>>>,
    maintenance: Option<JoinHandle<Result<(), String>>>,
    epoch: Arc<AtomicI64>,
    end: Arc<AtomicI64>,
    stop: Arc<AtomicBool>,
    journal: Journal,
}

impl Recorder {
    pub fn start(
        directory: &Path,
        options: &RecordingOptions,
        ffmpeg_path: &Path,
    ) -> Result<Self, String> {
        Self::start_internal(directory, options, ffmpeg_path, None)
    }

    pub(crate) fn start_cancellable(
        directory: &Path,
        options: &RecordingOptions,
        ffmpeg_path: &Path,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let result = Self::start_internal(directory, options, ffmpeg_path, Some(cancel.clone()));
        if startup::check_cancel(Some(&cancel)).is_err() {
            drop(result);
            Err(startup::CANCELLED.into())
        } else {
            result
        }
    }

    fn start_internal(
        directory: &Path,
        options: &RecordingOptions,
        ffmpeg_path: &Path,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Self, String> {
        startup::check_cancel(cancel.as_deref())?;
        cleanup::ensure_idle()?;
        options.validate()?;
        let (target, rect) = validated_target(&options.source_id)?;
        let (width, height) = region::dimensions(
            (
                (rect.right as i64 - rect.left as i64) as u32,
                (rect.bottom as i64 - rect.top as i64) as u32,
            ),
            options.region,
        )?;
        startup::check_cancel(cancel.as_deref())?;
        fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let directory = canonical_media_directory(directory)?;
        if directory.join("capture-journal.json").exists() {
            return Err("This project already contains recording data.".into());
        }
        fs::create_dir(directory.join("segments")).map_err(|error| error.to_string())?;
        let journal = Journal {
            version: 1,
            source_id: options.source_id.clone(),
            width,
            height,
            fps: options.fps,
            epoch_qpc: 0,
            camera_epoch_qpc: None,
            camera_offset_seconds: None,
            duration: 0.0,
            microphone: options.microphone_id.is_some(),
            system_audio: options.system_audio,
            segments: Vec::new(),
            segment_durations: Vec::new(),
        };
        write_journal(&directory, &journal)?;
        let mut recorder = Self {
            directory,
            ffmpeg: ffmpeg_path.to_owned(),
            capture: None,
            microphone: None,
            system_audio: None,
            camera: None,
            cursor: None,
            maintenance: None,
            epoch: Arc::new(AtomicI64::new(0)),
            end: Arc::new(AtomicI64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            journal,
        };
        let _cancel_relay = startup::CancelRelay::start(cancel.clone(), recorder.stop.clone())?;
        startup::check_cancel(cancel.as_deref())?;
        if options.system_audio {
            recorder.system_audio = Some(audio::AudioWorker::start_system(
                &recorder.directory.join("system.wav"),
                options,
                recorder.epoch.clone(),
                recorder.stop.clone(),
                recorder.end.clone(),
            )?);
        }
        startup::check_cancel(cancel.as_deref())?;
        if let Some(id) = &options.microphone_id {
            recorder.microphone = Some(audio::AudioWorker::start(
                &recorder.directory.join("microphone.wav"),
                Some(id.clone()),
                recorder.epoch.clone(),
                recorder.stop.clone(),
                recorder.end.clone(),
            )?);
        }
        startup::check_cancel(cancel.as_deref())?;
        if let Some(id) = &options.camera_id {
            if !camera::enumerate(ffmpeg_path)?
                .iter()
                .any(|device| device.id == *id)
            {
                return Err("The selected webcam is no longer available.".into());
            }
            startup::check_cancel(cancel.as_deref())?;
            recorder.camera = Some(camera::CameraWorker::start(
                &recorder.directory,
                id,
                ffmpeg_path,
            )?);
            recorder.journal.camera_epoch_qpc = recorder.camera.as_ref().map(|camera| camera.epoch);
        }
        startup::check_cancel(cancel.as_deref())?;
        let cursor_canvas = Arc::new(AtomicU64::new(cursor::canvas_size(width, height)));
        let (cursor_worker, cursor_ready) = cursor::spawn(
            target,
            options.region,
            cursor_canvas.clone(),
            recorder.directory.clone(),
            recorder.epoch.clone(),
            recorder.stop.clone(),
            recorder.end.clone(),
        )?;
        recorder.cursor = Some(cursor_worker);
        startup::wait_cancellable(&cursor_ready, Duration::from_secs(3), cancel.as_deref())
            .map_err(|error| match error {
                startup::WaitError::Cancelled => startup::CANCELLED.to_owned(),
                _ => "Cursor metadata storage did not initialize in time.".to_owned(),
            })??;
        startup::check_cancel(cancel.as_deref())?;
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let startup = Arc::new(startup::State::default());
        let native = Arc::new(windows_capture::diagnostics::CaptureDiagnostics::default());
        let flags = CaptureFlags {
            region: options.region,
            directory: recorder.directory.clone(),
            journal: recorder.journal.clone(),
            epoch: recorder.epoch.clone(),
            cursor_canvas,
            stop: recorder.stop.clone(),
            end: recorder.end.clone(),
            ready: ready_tx,
            startup: startup.clone(),
            native: native.clone(),
            expected_app: match target {
                Target::App(app) => Some(app),
                _ => None,
            },
        };
        let launched = startup::launch_cancellable(
            &recorder.directory,
            recorder.stop.clone(),
            Duration::from_secs(15),
            cancel.clone(),
            move || target.start(flags),
            |capture| capture.stop().map_err(|error| error.to_string()),
        );
        recorder.capture = Some(launched.inspect_err(|error| {
            startup::persist_diagnostic(
                &recorder.directory,
                &recorder.journal,
                &startup,
                native.snapshot(),
                error,
                None,
            );
        })?);
        let first_frame = recorder.wait_for_screen_frame(
            &ready_rx,
            &startup,
            matches!(target, Target::Monitor(_)),
            Duration::from_secs(15),
            cancel.as_deref(),
        );
        let (width, height) = first_frame.inspect_err(|error| {
            startup::persist_diagnostic(
                &recorder.directory,
                &recorder.journal,
                &startup,
                native.snapshot(),
                error,
                recorder
                    .capture
                    .as_ref()
                    .map(|capture| capture.is_finished()),
            );
        })?;
        recorder.journal.width = width;
        recorder.journal.height = height;
        startup::check_cancel(cancel.as_deref())?;
        let callback = recorder.capture.as_ref().unwrap().callback();
        let stop = recorder.stop.clone();
        recorder.maintenance = Some(
            thread::Builder::new()
                .name("static-frame-clock".into())
                .spawn(move || {
                    let _runtime = cleanup::NativeRuntime::initialize()?;
                    let callback = callback;
                    while !stop.load(Ordering::Acquire) {
                        thread::sleep(Duration::from_millis(100));
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        callback.lock().keep_static_frame(qpc_now())?;
                    }
                    Ok(())
                })
                .map_err(|error| error.to_string())?,
        );
        startup::check_cancel(cancel.as_deref())?;
        Ok(recorder)
    }

    fn wait_for_screen_frame(
        &mut self,
        ready: &mpsc::Receiver<Result<(u32, u32), String>>,
        state: &startup::State,
        monitor: bool,
        deadline: Duration,
        cancel: Option<&AtomicBool>,
    ) -> Result<(u32, u32), String> {
        let dimensions = startup::wait_for_first_frame_cancellable(
            ready,
            state,
            monitor,
            deadline,
            cancel,
            || {
                if let Some(error) = self.startup_input_failure() {
                    return Some(Err(error));
                }
                if self
                    .capture
                    .as_ref()
                    .is_some_and(|capture| capture.is_finished())
                {
                    self.request_stop();
                    let capture = self.capture.take().unwrap();
                    Some(cleanup::run_native(
                        &self.directory,
                        Duration::from_secs(2),
                        move || capture.wait().map_err(|error| error.to_string()),
                    ))
                } else {
                    None
                }
            },
        )?;
        match self.startup_input_failure() {
            Some(error) => Err(error),
            None => Ok(dimensions),
        }
    }

    fn startup_input_failure(&mut self) -> Option<String> {
        for (name, input) in [
            ("Microphone", self.microphone.as_mut()),
            ("System audio", self.system_audio.as_mut()),
        ] {
            if let Some(input) = input.filter(|input| input.is_finished()) {
                return Some(match input.finish() {
                    Err(error) => format!("{name} failed during screen startup: {error}"),
                    Ok(()) => format!("{name} stopped before screen recording started."),
                });
            }
        }
        if let Some(camera) = self.camera.as_ref().filter(|camera| camera.is_finished()) {
            return Some(
                camera
                    .snapshot()
                    .error
                    .unwrap_or_else(|| "Webcam stopped before screen recording started.".into()),
            );
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            let cursor = self.cursor.take()?;
            return Some(match cursor.join() {
                Ok(Err(error)) => format!("Cursor capture failed during screen startup: {error}"),
                Ok(Ok(_)) => "Cursor capture stopped before screen recording started.".into(),
                Err(_) => "Cursor capture panicked during screen startup.".into(),
            });
        }
        None
    }

    pub fn elapsed(&self) -> f64 {
        let epoch = self.epoch.load(Ordering::Acquire);
        if epoch == 0 {
            0.0
        } else {
            ((qpc_now() - epoch) as f64 / 10_000_000.0).max(0.0)
        }
    }

    pub fn camera_snapshot(&self) -> preview::PreviewSnapshot {
        self.camera
            .as_ref()
            .map_or_else(session::inactive_snapshot, |camera| camera.snapshot())
    }

    pub fn health(&self) -> Result<(), String> {
        if self
            .maintenance
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return Err(
                "The screen recording clock stopped. Stop recording to save the available media."
                    .into(),
            );
        }
        if self
            .capture
            .as_ref()
            .is_some_and(|capture| capture.is_finished())
        {
            return Err("The captured window closed or the screen capture stopped. Stop recording to save the available media.".into());
        }
        if self
            .microphone
            .as_ref()
            .is_some_and(|audio| audio.is_finished())
        {
            return Err(
                "Microphone capture stopped. Stop recording to save the available media.".into(),
            );
        }
        if self
            .system_audio
            .as_ref()
            .is_some_and(|audio| audio.is_finished())
        {
            return Err(
                "System audio capture stopped. Stop recording to save the available media.".into(),
            );
        }
        if self
            .camera
            .as_ref()
            .is_some_and(|camera| camera.is_finished())
        {
            return Err(
                "Webcam capture stopped. Stop recording to save the available media.".into(),
            );
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.is_finished())
        {
            return Err(
                "Cursor capture stopped. Stop recording to save the available media.".into(),
            );
        }
        Ok(())
    }

    fn request_stop(&self) {
        let _ = self
            .end
            .compare_exchange(0, qpc_now(), Ordering::AcqRel, Ordering::Acquire);
        self.stop.store(true, Ordering::Release);
        if let Some(capture) = &self.capture {
            capture.halt_handle().store(true, Ordering::Release);
        }
    }

    fn shutdown(&mut self) -> Result<Vec<CursorSample>, String> {
        self.shutdown_with_deadline(cleanup::DEADLINE)
    }

    fn shutdown_with_deadline(&mut self, deadline: Duration) -> Result<Vec<CursorSample>, String> {
        self.request_stop();
        let mut errors = Vec::new();
        if let Some(camera) = &mut self.camera {
            if let Err(error) = camera.stop() {
                errors.push(error);
            }
        }
        let maintenance = self.maintenance.take();
        let capture = self.capture.take();
        let microphone = self.microphone.take();
        let system_audio = self.system_audio.take();
        let cursor = self.cursor.take();
        let end = self.end.load(Ordering::Acquire);
        let mut journal = self.journal.clone();
        let result = cleanup::run_native(&self.directory, deadline, move || {
            if let Some(worker) = maintenance {
                match worker.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => errors.push(error),
                    Err(_) => errors.push("Screen recording clock panicked.".into()),
                }
            }
            if let Some(capture) = capture {
                let callback = capture.callback();
                if let Err(error) = capture.stop() {
                    errors.push(error.to_string());
                }
                match callback.lock().finish(end) {
                    Ok(finished) => journal = finished,
                    Err(error) => errors.push(error),
                };
            }
            for mut worker in [microphone, system_audio].into_iter().flatten() {
                if let Err(error) = worker.finish() {
                    errors.push(error);
                }
            }
            let cursor = if let Some(cursor) = cursor {
                match cursor.join() {
                    Ok(Ok(cursor)) => cursor,
                    Ok(Err(error)) => {
                        errors.push(error);
                        vec![]
                    }
                    Err(_) => {
                        errors.push("Cursor worker panicked.".into());
                        vec![]
                    }
                }
            } else {
                vec![]
            };
            if errors.is_empty() {
                Ok((journal, cursor))
            } else {
                Err(errors.join(" "))
            }
        });
        let (journal, cursor) = result?;
        self.journal = journal;
        Ok(cursor)
    }

    pub fn stop(mut self) -> Result<CaptureResult, String> {
        let mut cursor = self.shutdown()?;
        if self.journal.segments.is_empty() {
            return Err("No video frames were recorded.".into());
        }
        let duration = merge_screen(&self.directory, &self.journal, &self.ffmpeg)?;
        cursor.retain(|sample| sample.time <= duration + 0.05);
        let camera_track = if let Some(camera) = &self.camera {
            Some(camera::finalize_recording(
                &camera.raw_path,
                &self.ffmpeg,
                camera::CameraTiming {
                    camera_epoch: camera.epoch,
                    screen_epoch: self.journal.epoch_qpc,
                    duration,
                    existing_offset: self.journal.camera_offset_seconds,
                    recovering: false,
                    require_full_validation: camera.requires_full_validation(),
                },
                |offset| {
                    self.journal.camera_offset_seconds = Some(offset);
                    write_journal(&self.directory, &self.journal)
                },
            )?)
        } else {
            None
        };
        let mut warnings = cursor::warning(&self.directory)?
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(warning) = camera_track
            .as_ref()
            .and_then(|track| track.warning.as_ref())
        {
            warnings.push(warning.clone());
        }
        if !warnings.is_empty() {
            fs::write(
                self.directory.join("recovery-warning.txt"),
                warnings.join("\n"),
            )
            .map_err(|error| error.to_string())?;
        }
        let camera_offset_seconds = camera_track.as_ref().map_or(0.0, |track| track.offset);
        let camera_path = camera_track.map(|track| track.path);
        Ok(CaptureResult {
            screen_path: self.directory.join("screen.mp4"),
            microphone_path: self
                .journal
                .microphone
                .then(|| self.directory.join("microphone.wav")),
            system_audio_path: self
                .journal
                .system_audio
                .then(|| self.directory.join("system.wav")),
            camera_path,
            camera_offset_seconds,
            width: self.journal.width,
            height: self.journal.height,
            fps: self.journal.fps,
            duration,
            cursor,
        })
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.capture.is_some()
            || self.cursor.is_some()
            || self.maintenance.is_some()
            || self.microphone.is_some()
            || self.system_audio.is_some()
        {
            let _ = self.shutdown();
        } else {
            self.request_stop();
            if let Some(camera) = &mut self.camera {
                let _ = camera.stop();
            }
        }
    }
}

pub fn recover(directory: &Path, ffmpeg_path: &Path) -> Result<CaptureResult, String> {
    recover_part(directory, ffmpeg_path).map_err(String::from)
}

fn recover_part(directory: &Path, ffmpeg_path: &Path) -> Result<CaptureResult, RecoveryError> {
    fs::metadata(directory).map_err(RecoveryError::input_io)?;
    let directory = canonical_media_directory(directory)?;
    cleanup::ensure_recovery_ready(&directory)?;
    for relative in [
        "capture-journal.json",
        "capture-journal.tmp",
        "cursor.ndjson",
        "cursor-warning.txt",
        "segments",
        "segments/concat.txt",
        "screen.mp4",
        "screen-finalizing.mp4",
        "screen-finalize.log",
        "screen-validate.log",
        "segment-validate.log",
        "microphone.wav",
        "system.wav",
        "camera.mp4",
        "camera-source.mp4",
        "camera-source-finalizing.mp4",
        "camera-validate.log",
        "camera.raw.mp4",
        "camera-finalizing.mp4",
        "camera-align.log",
        "recovery-warning.txt",
    ] {
        validate_owned_path(&directory, &directory.join(relative))?;
    }
    let file =
        File::open(directory.join("capture-journal.json")).map_err(RecoveryError::input_io)?;
    if file.metadata().map_err(|error| error.to_string())?.len() > 16 * 1024 * 1024 {
        return Err(RecoveryError::Unusable(
            "Recording journal exceeds the supported size.".into(),
        ));
    }
    let mut journal: Journal = serde_json::from_reader(file).map_err(|error| {
        if error.is_io() {
            RecoveryError::Retry(error.to_string())
        } else {
            RecoveryError::Unusable(error.to_string())
        }
    })?;
    validate_journal(&journal).map_err(RecoveryError::Unusable)?;
    for segment in &journal.segments {
        validate_owned_path(&directory, &directory.join("segments").join(segment))?;
    }
    let mut warnings = cursor::warning(&directory)?.into_iter().collect::<Vec<_>>();
    let final_path = directory.join("screen.mp4");
    let final_info = if final_path.try_exists().map_err(|error| error.to_string())? {
        media::usable(media::validate_video(
            ffmpeg_path,
            &final_path,
            &directory.join("screen-validate.log"),
        ))?
    } else {
        None
    };
    let finalized_valid = final_info
        .as_ref()
        .is_some_and(|info| validate_screen_info(info, &journal, journal.duration).is_ok());
    let mut merge_journal = journal.clone();
    if !finalized_valid {
        let mut usable = 0usize;
        let mut duration = 0.0;
        for (index, segment) in journal.segments.iter().enumerate() {
            let checked = media::usable(media::validate_video(
                ffmpeg_path,
                &directory.join("segments").join(segment),
                &directory.join("segment-validate.log"),
            ))?;
            match checked {
                Some(info) if info.width == journal.width && info.height == journal.height => {
                    let expected = journal
                        .segment_durations
                        .get(index)
                        .copied()
                        .unwrap_or(info.duration);
                    if info.duration + 0.15 < expected {
                        break;
                    }
                    duration += expected;
                    usable += 1;
                }
                _ => break,
            }
        }
        if usable == 0 {
            return Err(RecoveryError::Unusable(
                "No complete screen recording chunk could be decoded. Original media is preserved."
                    .into(),
            ));
        }
        if usable < journal.segments.len() {
            warnings.push(format!("Recovered the first {duration:.3} seconds before a missing or damaged chunk. All original chunks were preserved."));
            merge_journal.segments.truncate(usable);
            merge_journal.segment_durations.truncate(usable);
            merge_journal.duration = duration;
        }
    }
    if final_path.exists()
        && !final_info.as_ref().is_some_and(|info| {
            validate_screen_info(info, &merge_journal, merge_journal.duration).is_ok()
        })
    {
        let preserved = media::preserve_invalid(&final_path)?;
        warnings.push(format!(
            "The invalid screen video was preserved as {} and rebuilt from recording chunks.",
            preserved.file_name().unwrap().to_string_lossy()
        ));
    }
    let duration = merge_screen(&directory, &merge_journal, ffmpeg_path)?;
    let mut cursor = Vec::new();
    if let Some(file) =
        media::usable(File::open(directory.join("cursor.ndjson")).map_err(RecoveryError::input_io))?
    {
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            if std::io::Read::take(&mut reader, 4097)
                .read_line(&mut line)
                .map_err(|error| error.to_string())?
                == 0
            {
                break;
            }
            if line.len() > 4096 {
                return Err("Cursor recovery contains an oversized sample.".into());
            }
            if let Ok(sample) = serde_json::from_str::<CursorSample>(&line) {
                if let Some(sample) = cursor::recovered_sample(sample, duration) {
                    if cursor.len() >= 16_000_000 {
                        return Err("Cursor recovery exceeds the supported sample count.".into());
                    }
                    cursor.push(sample);
                }
            }
        }
    }
    cursor.sort_by(|a, b| a.time.total_cmp(&b.time));
    let audio_path = |name: &str| -> Result<Option<PathBuf>, String> {
        let path = directory.join(name);
        let readable =
            wav::readable(&path).map_err(|error| format!("Cannot read {name}: {error}"))?;
        Ok(readable.then_some(path))
    };
    let microphone_path = if journal.microphone {
        audio_path("microphone.wav")?
    } else {
        None
    };
    let system_audio_path = if journal.system_audio {
        audio_path("system.wav")?
    } else {
        None
    };
    if journal.microphone && microphone_path.is_none() {
        warnings.push("The microphone track could not be recovered.".to_owned());
    }
    if journal.system_audio && system_audio_path.is_none() {
        warnings.push("The system audio track could not be recovered.".to_owned());
    }
    let mut camera_offset_seconds = 0.0;
    let camera_path = if let Some(camera_epoch) = journal.camera_epoch_qpc {
        let raw = directory.join("camera.raw.mp4");
        match camera::finalize_recording(
            &raw,
            ffmpeg_path,
            camera::CameraTiming {
                camera_epoch,
                screen_epoch: journal.epoch_qpc,
                duration,
                existing_offset: journal.camera_offset_seconds,
                recovering: true,
                require_full_validation: false,
            },
            |offset| {
                journal.camera_offset_seconds = Some(offset);
                write_journal(&directory, &journal)
            },
        ) {
            Ok(track) => {
                camera_offset_seconds = track.offset;
                if let Some(warning) = track.warning {
                    warnings.push(warning);
                }
                Some(track.path)
            }
            Err(RecoveryError::Unusable(error)) => {
                warnings.push(format!("The webcam track could not be recovered: {error}"));
                None
            }
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    if !warnings.is_empty() {
        fs::write(
            directory.join("recovery-warning.txt"),
            format!(
                "The screen recording was recovered.\n{}\n",
                warnings.join("\n")
            ),
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(CaptureResult {
        screen_path: directory.join("screen.mp4"),
        microphone_path,
        system_audio_path,
        camera_path,
        camera_offset_seconds,
        width: journal.width,
        height: journal.height,
        fps: journal.fps,
        duration,
        cursor,
    })
}

fn merge_screen(directory: &Path, journal: &Journal, ffmpeg: &Path) -> Result<f64, String> {
    validate_journal(journal)?;
    let output = directory.join("screen.mp4");
    if output.exists()
        && !media::usable(media::probe_video(ffmpeg, &output))?
            .is_some_and(|info| validate_screen_info(&info, journal, journal.duration).is_ok())
    {
        media::preserve_invalid(&output)?;
    }
    if !output.exists() {
        let list = directory.join("segments").join("concat.txt");
        let mut file = File::create(&list).map_err(|error| error.to_string())?;
        for (index, name) in journal.segments.iter().enumerate() {
            if *name != format!("screen-{index:05}.mp4") {
                return Err("Recording journal contains an invalid segment path.".into());
            }
            if !directory.join("segments").join(name).is_file() {
                return Err(format!("Recording segment {name} is missing."));
            }
            writeln!(file, "file '{name}'").map_err(|error| error.to_string())?;
            if let Some(duration) = journal.segment_durations.get(index) {
                if !duration.is_finite() || *duration <= 0.0 || *duration > 86400.0 {
                    return Err("Recording journal contains an invalid segment duration.".into());
                }
                writeln!(file, "outpoint {duration:.7}\nduration {duration:.7}")
                    .map_err(|error| error.to_string())?;
            }
        }
        file.sync_all().map_err(|error| error.to_string())?;
        let temporary = directory.join("screen-finalizing.mp4");
        if temporary.exists() {
            fs::remove_file(&temporary).map_err(|error| error.to_string())?;
        }
        process::run_logged(
            ffmpeg,
            &[
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-n".into(),
                "-f".into(),
                "concat".into(),
                "-safe".into(),
                "1".into(),
                "-i".into(),
                list.to_string_lossy().into_owned(),
                "-map".into(),
                "0:v:0".into(),
                "-t".into(),
                format!("{:.7}", journal.duration),
                "-c".into(),
                "copy".into(),
                "-movflags".into(),
                "+faststart".into(),
                temporary.to_string_lossy().into_owned(),
            ],
            &directory.join("screen-finalize.log"),
        )?;
        validate_screen_info(
            &media::probe_video(ffmpeg, &temporary)?,
            journal,
            journal.duration,
        )?;
        fs::rename(temporary, &output).map_err(|error| error.to_string())?;
    }
    if !journal.duration.is_finite() || journal.duration <= 0.0 {
        return Err("Recording contains no playable video.".into());
    }
    Ok(journal.duration)
}

fn validate_screen_info(
    info: &media::VideoInfo,
    journal: &Journal,
    duration: f64,
) -> Result<(), String> {
    if info.width != journal.width
        || info.height != journal.height
        || (info.duration - duration).abs() > 0.15
    {
        return Err("Screen video does not match the recording dimensions or duration.".into());
    }
    Ok(())
}

fn validate_journal(journal: &Journal) -> Result<(), String> {
    if journal.version != 1
        || journal.segments.is_empty()
        || journal.segments.len() > 100_000
        || ![30, 60].contains(&journal.fps)
    {
        return Err("There are no supported finalized video segments to recover.".into());
    }
    if !(2..=16384).contains(&journal.width)
        || !(2..=16384).contains(&journal.height)
        || !journal.duration.is_finite()
        || journal.duration <= 0.0
        || journal.epoch_qpc <= 0
        || journal.camera_epoch_qpc.is_some_and(|epoch| epoch <= 0)
        || journal
            .camera_offset_seconds
            .is_some_and(|offset| !offset.is_finite() || !(0.0..=3600.0).contains(&offset))
    {
        return Err("Recording journal contains invalid timing or dimensions.".into());
    }
    if !journal.segment_durations.is_empty()
        && journal.segment_durations.len() != journal.segments.len()
    {
        return Err("Recording journal has mismatched segment durations.".into());
    }
    for (index, name) in journal.segments.iter().enumerate() {
        if *name != format!("screen-{index:05}.mp4") {
            return Err("Recording journal contains an invalid segment path.".into());
        }
    }
    if journal
        .segment_durations
        .iter()
        .any(|duration| !duration.is_finite() || *duration <= 0.0 || *duration > 86400.0)
    {
        return Err("Recording journal contains an invalid segment duration.".into());
    }
    Ok(())
}

fn validate_owned_path(directory: &Path, path: &Path) -> Result<(), String> {
    let root = fs::canonicalize(directory).map_err(|error| error.to_string())?;
    let mut existing = path;
    loop {
        match fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .ok_or("Recovery path has no valid parent.")?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let canonical = fs::canonicalize(existing).map_err(|error| error.to_string())?;
    if !canonical.starts_with(&root) {
        return Err("Recovery path resolves outside the project directory.".into());
    }
    Ok(())
}

fn write_journal(directory: &Path, journal: &Journal) -> Result<(), String> {
    let path = directory.join("capture-journal.json");
    let temporary = directory.join("capture-journal.tmp");
    let mut file = File::create(&temporary).map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut file, journal).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

fn append_cursor(
    writer: &mut impl Write,
    samples: &mut Vec<CursorSample>,
    sample: CursorSample,
) -> Result<(), String> {
    append_cursor_limited(writer, samples, sample, 16_000_000)
}

fn append_cursor_limited(
    writer: &mut impl Write,
    samples: &mut Vec<CursorSample>,
    sample: CursorSample,
    limit: usize,
) -> Result<(), String> {
    if samples.len() >= limit {
        return Err("Cursor metadata reached its supported recording length. Stop recording to save the available media.".into());
    }
    serde_json::to_writer(&mut *writer, &sample).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(|error| error.to_string())?;
    samples.push(sample);
    Ok(())
}

fn qpc_now() -> i64 {
    let mut count = 0;
    let mut frequency = 0;
    unsafe {
        let _ = QueryPerformanceCounter(&mut count);
        let _ = QueryPerformanceFrequency(&mut frequency);
    }
    if frequency <= 0 {
        return 0;
    }
    ((count as i128 * 10_000_000) / frequency as i128) as i64
}

fn canonical_media_directory(path: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
    let text = canonical.to_string_lossy();
    if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
        Ok(PathBuf::from(format!("\\\\{unc}")))
    } else if let Some(local) = text.strip_prefix("\\\\?\\") {
        Ok(PathBuf::from(local))
    } else {
        Ok(canonical)
    }
}

struct PhysicalCoordinates(DPI_AWARENESS_CONTEXT);
impl PhysicalCoordinates {
    fn enter() -> Self {
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}
impl Drop for PhysicalCoordinates {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            unsafe {
                SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_camera_precedes_blocked_native_cleanup_and_recovery_stays_locked() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };

        let temporary = tempfile::tempdir().unwrap();
        let directory = canonical_media_directory(temporary.path()).unwrap();
        let camera = camera::CameraWorker::fixture(&directory);
        let process = unsafe {
            let handle = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                camera.process_id(),
            )
            .unwrap();
            OwnedHandle::from_raw_handle(handle.0)
        };
        let stop = Arc::new(AtomicBool::new(false));
        let end = Arc::new(AtomicI64::new(0));
        let (release, blocked) = mpsc::channel();
        let completed_file = directory.join("cleanup-completed");
        let worker_file = completed_file.clone();
        let maintenance = thread::spawn(move || {
            blocked.recv().map_err(|error| error.to_string())?;
            fs::write(worker_file, b"complete").map_err(|error| error.to_string())
        });
        let mut recorder = Recorder {
            directory: directory.clone(),
            ffmpeg: "unused-ffmpeg.exe".into(),
            capture: None,
            microphone: None,
            system_audio: None,
            camera: Some(camera),
            cursor: None,
            maintenance: Some(maintenance),
            epoch: Arc::new(AtomicI64::new(1)),
            end: end.clone(),
            stop: stop.clone(),
            journal: Journal {
                version: 1,
                source_id: "monitor:1".into(),
                width: 1920,
                height: 1080,
                fps: 30,
                epoch_qpc: 1,
                camera_epoch_qpc: Some(1),
                camera_offset_seconds: None,
                duration: 1.0,
                microphone: false,
                system_audio: false,
                segments: vec!["screen-00000.mp4".into()],
                segment_durations: vec![1.0],
            },
        };
        let started = std::time::Instant::now();
        let error = recorder
            .shutdown_with_deadline(Duration::from_millis(40))
            .unwrap_err();
        assert!(error.contains("still stopping"));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(stop.load(Ordering::Acquire));
        let stopped_at = end.load(Ordering::Acquire);
        assert!(stopped_at > 0);
        let mut exit_code = 259;
        unsafe {
            GetExitCodeProcess(HANDLE(process.as_raw_handle()), &mut exit_code).unwrap();
        }
        assert_ne!(
            exit_code, 259,
            "Camera was left alive while screen finalization was blocked."
        );
        assert!(!completed_file.exists());
        assert!(recover(&directory, Path::new("unused-ffmpeg.exe"))
            .err()
            .unwrap()
            .contains("still stopping"));
        let options = RecordingOptions {
            source_id: "monitor:0".into(),
            fps: 30,
            microphone_id: None,
            system_audio: false,
            system_audio_scope: Default::default(),
            region: None,
            camera_id: None,
        };
        assert!(
            Recorder::start(&directory, &options, Path::new("unused-ffmpeg.exe"))
                .err()
                .unwrap()
                .contains("previous recording is still stopping")
        );
        drop(recorder);
        assert_eq!(end.load(Ordering::Acquire), stopped_at);
        release.send(()).unwrap();
        for _ in 0..200 {
            if cleanup::ensure_recovery_ready(&directory).is_ok() {
                assert!(completed_file.exists());
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("Recovery stayed locked after the native writer completed.");
    }

    #[test]
    fn recorder_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Recorder>();
    }
    #[test]
    fn malformed_source_ids_are_rejected() {
        assert!(Target::parse("monitor:0").is_err());
        assert!(Target::parse("window:../foo").is_err());
    }

    #[test]
    fn source_preflight_rejects_stale_apps_without_creating_recording_files() {
        let directory = tempfile::tempdir().unwrap();
        let before = fs::read_dir(directory.path()).unwrap().count();
        for source in ["app:0:1", "app:1:0", "app:4294967295:18446744073709551615"] {
            let result = validate_source(source);
            assert!(result.is_err(), "{source}");
            assert!(!result.unwrap_err().contains("encoder"));
        }
        assert_eq!(before, 0);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn source_preflight_dimensions_match_capture_bounds_without_integer_overflow() {
        for (width, height, expected) in [
            (2, 2, true),
            (16384, 16384, true),
            (1, 100, false),
            (100, 0, false),
            (16385, 100, false),
            (-1, 100, false),
        ] {
            let rect = RECT {
                left: -1920,
                top: -1080,
                right: -1920 + width,
                bottom: -1080 + height,
            };
            assert_eq!(validate_source_dimensions(&rect).is_ok(), expected);
        }
        assert!(validate_source_dimensions(&RECT {
            left: i32::MIN,
            right: i32::MAX,
            top: 0,
            bottom: 1080,
        })
        .is_err());
    }
    #[test]
    fn clock_is_monotonic() {
        let start = qpc_now();
        thread::sleep(Duration::from_millis(2));
        assert!(qpc_now() > start);
    }

    #[test]
    fn recovery_rejects_traversal_even_when_final_video_exists() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("screen.mp4"), b"preserved").unwrap();
        let journal = Journal {
            version: 1,
            source_id: "monitor:1".into(),
            width: 1920,
            height: 1080,
            fps: 30,
            epoch_qpc: 1,
            camera_epoch_qpc: None,
            camera_offset_seconds: None,
            duration: 1.0,
            microphone: false,
            system_audio: false,
            segments: vec!["../outside.mp4".into()],
            segment_durations: vec![1.0],
        };
        write_journal(directory.path(), &journal).unwrap();
        let error = recover(directory.path(), Path::new("unavailable-ffmpeg.exe"))
            .err()
            .unwrap();
        assert!(error.contains("invalid segment path"));
        assert_eq!(
            fs::read(directory.path().join("screen.mp4")).unwrap(),
            b"preserved"
        );
        assert!(!directory.path().join("segments").exists());
    }

    #[test]
    fn journal_rejects_absolute_and_injected_segment_paths() {
        let mut journal = Journal {
            version: 1,
            source_id: "monitor:1".into(),
            width: 1920,
            height: 1080,
            fps: 30,
            epoch_qpc: 1,
            camera_epoch_qpc: None,
            camera_offset_seconds: None,
            duration: 1.0,
            microphone: false,
            system_audio: false,
            segments: vec!["screen-00000.mp4".into()],
            segment_durations: vec![1.0],
        };
        assert!(validate_journal(&journal).is_ok());
        for name in [
            "../outside.mp4",
            "C:\\outside.mp4",
            "screen-00000.mp4'\nfile '../outside.mp4",
            "screen-00001.mp4",
        ] {
            journal.segments[0] = name.into();
            assert!(validate_journal(&journal).is_err());
        }
    }
}
