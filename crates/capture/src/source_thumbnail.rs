use super::{cleanup, Target};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    encoder::{ImageEncoder, ImageEncoderPixelFormat, ImageFormat},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
};

const MAX_SOURCE_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_JPEG_BYTES: usize = 2 * 1024 * 1024;
const MAX_EDGE: u32 = 960;

pub struct SourceThumbnail {
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
}

pub fn source_thumbnail(source_id: &str) -> Result<SourceThumbnail, String> {
    cleanup::ensure_idle()?;
    let (target, _) = super::validated_target(source_id)?;
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let result = cleanup::run_native(
        Path::new("source-thumbnail"),
        Duration::from_secs(12),
        move || {
            let (sender, receiver) = mpsc::sync_channel(1);
            let flags = Flags {
                target,
                stop: worker_stop.clone(),
                sender,
            };
            let capture = match target {
                Target::Monitor(handle) => {
                    start(Monitor::from_raw_hmonitor(handle as *mut _), flags)
                }
                Target::Window(handle) => start(Window::from_raw_hwnd(handle as *mut _), flags),
                Target::App(app) => {
                    app.validate()?;
                    start(app, flags)
                }
            }?;
            finish_snapshot(&receiver, &worker_stop, Duration::from_secs(6), || {
                capture
                    .stop()
                    .map_err(|error| format!("Cannot close source preview: {error}"))
            })
        },
    );
    stop.store(true, Ordering::Release);
    result
}

fn finish_snapshot(
    receiver: &mpsc::Receiver<Result<SourceThumbnail, String>>,
    stop: &AtomicBool,
    deadline: Duration,
    close: impl FnOnce() -> Result<(), String>,
) -> Result<SourceThumbnail, String> {
    let image = if stop.load(Ordering::Acquire) {
        Err("Source preview timed out.".into())
    } else {
        receiver.recv_timeout(deadline).unwrap_or_else(|_| {
            Err("The selected source did not provide a preview frame in time.".into())
        })
    };
    stop.store(true, Ordering::Release);
    close()?;
    image
}

struct Flags {
    target: Target,
    stop: Arc<AtomicBool>,
    sender: mpsc::SyncSender<Result<SourceThumbnail, String>>,
}
struct Snapshot {
    flags: Flags,
    finished: bool,
}

fn start<T: TryInto<GraphicsCaptureItemType> + Send + 'static>(
    target: T,
    flags: Flags,
) -> Result<CaptureControl<Snapshot, String>, String> {
    Snapshot::start_free_threaded(Settings::new(
        target,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    ))
    .map_err(|error| format!("Cannot preview the selected source: {error}"))
}

impl GraphicsCaptureApiHandler for Snapshot {
    type Flags = Flags;
    type Error = String;
    fn new(ctx: Context<Flags>) -> Result<Self, String> {
        if ctx.flags.stop.load(Ordering::Acquire) {
            return Err("Source preview timed out.".into());
        }
        if let Target::App(app) = ctx.flags.target {
            app.validate()?;
        }
        Ok(Self {
            flags: ctx.flags,
            finished: false,
        })
    }
    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), String> {
        if self.finished || self.flags.stop.load(Ordering::Acquire) {
            control.stop();
            return Ok(());
        }
        self.finished = true;
        let result = (|| {
            if let Target::App(app) = self.flags.target {
                app.validate()?;
            }
            let (width, height) = (frame.width(), frame.height());
            thumbnail_dimensions(width, height)?;
            let mut buffer = frame.buffer().map_err(|error| error.to_string())?;
            let stride = buffer.row_pitch() as usize;
            encode_thumbnail(buffer.as_raw_buffer(), width, height, stride)
        })();
        let _ = self.flags.sender.try_send(result);
        control.stop();
        Ok(())
    }
    fn on_closed(&mut self) -> Result<(), String> {
        if !self.finished {
            let _ = self.flags.sender.try_send(Err(
                "The selected source closed before its preview was ready.".into(),
            ));
        }
        Ok(())
    }
}

fn thumbnail_dimensions(width: u32, height: u32) -> Result<(u32, u32), String> {
    super::region::dimensions((width, height), None)?;
    if u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        return Err("This source is too large to preview. Choose a smaller source.".into());
    }
    let scale = (MAX_EDGE as f64 / f64::from(width.max(height))).min(1.0);
    Ok((
        (width as f64 * scale).round().max(1.0) as u32,
        (height as f64 * scale).round().max(1.0) as u32,
    ))
}

fn downsample(
    bytes: &[u8],
    width: u32,
    height: u32,
    stride: usize,
) -> Result<(Vec<u8>, u32, u32), String> {
    let (out_width, out_height) = thumbnail_dimensions(width, height)?;
    let required = stride
        .checked_mul(height as usize)
        .ok_or("Preview frame size overflow.")?;
    if stride < width as usize * 4 || bytes.len() < required {
        return Err("The source preview returned an incomplete frame.".into());
    }
    let mut output = vec![0; out_width as usize * out_height as usize * 4];
    for y in 0..out_height {
        let source_y =
            ((u64::from(y) * 2 + 1) * u64::from(height) / (u64::from(out_height) * 2)) as usize;
        for x in 0..out_width {
            let source_x =
                ((u64::from(x) * 2 + 1) * u64::from(width) / (u64::from(out_width) * 2)) as usize;
            let source = source_y * stride + source_x * 4;
            let dest = (y as usize * out_width as usize + x as usize) * 4;
            output[dest..dest + 3].copy_from_slice(&bytes[source..source + 3]);
            output[dest + 3] = 255;
        }
    }
    Ok((output, out_width, out_height))
}

fn encode_thumbnail(
    bytes: &[u8],
    width: u32,
    height: u32,
    stride: usize,
) -> Result<SourceThumbnail, String> {
    let (pixels, out_width, out_height) = downsample(bytes, width, height, stride)?;
    let jpeg = ImageEncoder::new(ImageFormat::Jpeg, ImageEncoderPixelFormat::Bgra8)
        .and_then(|encoder| encoder.encode(&pixels, out_width, out_height))
        .map_err(|error| format!("Cannot encode source preview: {error}"))?;
    if jpeg.len() > MAX_JPEG_BYTES {
        return Err("The source preview exceeds its memory limit.".into());
    }
    Ok(SourceThumbnail {
        width,
        height,
        jpeg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_preserves_top_down_geometry_and_ignores_row_padding() {
        let mut bytes = vec![99; 3 * 16];
        for y in 0..3 {
            for x in 0..2 {
                let index = y * 16 + x * 4;
                bytes[index..index + 4].copy_from_slice(&[x as u8, y as u8, 200, 0]);
            }
        }
        let (pixels, width, height) = downsample(&bytes, 2, 3, 16).unwrap();
        assert_eq!((width, height), (2, 3));
        for y in 0..3 {
            for x in 0..2 {
                assert_eq!(
                    &pixels[(y * 2 + x) * 4..(y * 2 + x + 1) * 4],
                    &[x as u8, y as u8, 200, 255]
                );
            }
        }
        assert!(downsample(&bytes[..30], 2, 3, 16).is_err());
        assert!(downsample(&bytes, 2, 3, 7).is_err());
        assert_eq!(thumbnail_dimensions(3840, 2160).unwrap(), (960, 540));
        assert_eq!(thumbnail_dimensions(1080, 1920).unwrap(), (540, 960));
        assert_eq!(thumbnail_dimensions(16384, 2).unwrap(), (960, 1));
        assert!(thumbnail_dimensions(16384, 16384).is_err());
    }

    #[test]
    fn synthetic_jpeg_is_bounded_and_keeps_original_dimensions() {
        use windows::{
            Graphics::Imaging::{
                BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform,
                ColorManagementMode, ExifOrientationMode,
            },
            Storage::Streams::{DataWriter, InMemoryRandomAccessStream},
        };
        let _runtime = cleanup::NativeRuntime::initialize().unwrap();
        let mut pixels = [24, 80, 180, 255].repeat(1920 * 1080);
        for y in 540..1080 {
            for x in 0..1920 {
                pixels[(y * 1920 + x) * 4..(y * 1920 + x + 1) * 4]
                    .copy_from_slice(&[180, 80, 24, 255]);
            }
        }
        let started = std::time::Instant::now();
        let preview = encode_thumbnail(&pixels, 1920, 1080, 1920 * 4).unwrap();
        eprintln!(
            "thumbnail synthetic 1920x1080 to 960x540: {:.3} ms, {} bytes",
            started.elapsed().as_secs_f64() * 1000.0,
            preview.jpeg.len()
        );
        assert_eq!((preview.width, preview.height), (1920, 1080));
        assert!(preview.jpeg.starts_with(&[0xff, 0xd8]));
        assert!(preview.jpeg.ends_with(&[0xff, 0xd9]));
        assert!(preview.jpeg.len() < MAX_JPEG_BYTES);
        let stream = InMemoryRandomAccessStream::new().unwrap();
        let writer = DataWriter::CreateDataWriter(&stream.GetOutputStreamAt(0).unwrap()).unwrap();
        writer.WriteBytes(&preview.jpeg).unwrap();
        writer.StoreAsync().unwrap().join().unwrap();
        stream.Seek(0).unwrap();
        let decoder = BitmapDecoder::CreateAsync(&stream).unwrap().join().unwrap();
        assert_eq!(
            (
                decoder.PixelWidth().unwrap(),
                decoder.PixelHeight().unwrap()
            ),
            (960, 540)
        );
        let decoded = decoder
            .GetPixelDataTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Ignore,
                &BitmapTransform::new().unwrap(),
                ExifOrientationMode::IgnoreExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )
            .unwrap()
            .join()
            .unwrap()
            .DetachPixelData()
            .unwrap();
        for (x, y, expected) in [(100, 100, [24u8, 80, 180]), (800, 400, [180, 80, 24])] {
            let pixel = &decoded[(y * 960 + x) * 4..(y * 960 + x) * 4 + 3];
            assert!(
                pixel
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 3),
                "{pixel:?} != {expected:?}"
            );
        }
    }

    #[test]
    fn snapshot_failure_timeout_and_prior_cancellation_always_close_owned_capture() {
        for cancelled in [false, true] {
            let (_sender, receiver) = mpsc::sync_channel(1);
            let stop = AtomicBool::new(cancelled);
            let closed = AtomicBool::new(false);
            let result = finish_snapshot(&receiver, &stop, Duration::from_millis(1), || {
                assert!(stop.load(Ordering::Acquire));
                closed.store(true, Ordering::Release);
                Ok(())
            });
            assert!(result.is_err());
            assert!(closed.load(Ordering::Acquire));
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send(Ok(SourceThumbnail {
                width: 2,
                height: 2,
                jpeg: vec![],
            }))
            .unwrap();
        let result = finish_snapshot(
            &receiver,
            &AtomicBool::new(false),
            Duration::from_secs(1),
            || Err("injected close failure".into()),
        );
        assert!(matches!(result, Err(error) if error == "injected close failure"));
    }
}
