use std::fs::{self, File};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use parking_lot::Mutex;
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapEncoder, BitmapPixelFormat};
use windows::Media::Core::{
    AudioStreamDescriptor, MediaStreamSample, MediaStreamSource, MediaStreamSourceSampleRequestedEventArgs,
    MediaStreamSourceStartingEventArgs, VideoStreamDescriptor,
};
use windows::Media::MediaProperties::{
    AudioEncodingProperties, ContainerEncodingProperties, MediaEncodingProfile, MediaEncodingSubtypes,
    VideoEncodingProperties,
};
use windows::Media::Transcoding::{MediaTranscoder, PrepareTranscodeResult};
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Storage::Streams::{DataReader, IRandomAccessStream, InMemoryRandomAccessStream};
use windows::Storage::{FileAccessMode, StorageFile};
use windows::System::Threading::{ThreadPool, WorkItemHandler, WorkItemOptions, WorkItemPriority};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11SurfaceFromDXGISurface;
use windows::core::{HSTRING, Interface};

use crate::d3d11::SendDirectX;
use crate::frame::Frame;
use crate::settings::ColorFormat;

type VideoFrameReceiver = Arc<Mutex<mpsc::Receiver<Option<(VideoEncoderSource, TimeSpan)>>>>;
type AudioFrameReceiver = Arc<Mutex<mpsc::Receiver<Option<(AudioEncoderSource, TimeSpan)>>>>;
type PendingTranscode = Arc<Mutex<Option<(PrepareTranscodeResult, MediaTranscoder)>>>;

fn run_initialized_transcoder<T, A>(
    pending: Arc<Mutex<Option<T>>>,
    initialize: impl FnOnce() -> windows::core::Result<A>,
    transcode: impl FnOnce(T) -> Result<(), VideoEncoderError>,
) -> Result<(), VideoEncoderError> {
    let _runtime = initialize()?;
    let payload = pending.lock().take().ok_or(VideoEncoderError::TranscodeStopped)?;
    transcode(payload)
}

fn spawn_transcoder(
    transcode: PrepareTranscodeResult,
    transcoder: MediaTranscoder,
) -> Result<(JoinHandle<Result<(), VideoEncoderError>>, PendingTranscode), VideoEncoderError> {
    let pending = Arc::new(Mutex::new(Some((transcode, transcoder))));
    let worker_pending = pending.clone();
    let worker = thread::Builder::new().name("video-transcoder".into()).spawn(move || {
        run_initialized_transcoder(worker_pending, crate::winrt::WinRT::new, |(transcode, transcoder)| {
            let result = transcode.TranscodeAsync().and_then(|operation| operation.join());
            drop(transcoder);
            result.map_err(VideoEncoderError::from)
        })
    })?;
    Ok((worker, pending))
}

fn encoder_channel<T>() -> (mpsc::SyncSender<T>, mpsc::Receiver<T>) {
    mpsc::sync_channel(2)
}

fn join_transcoder(worker: JoinHandle<Result<(), VideoEncoderError>>) -> Result<(), VideoEncoderError> {
    worker.join().map_err(|_| VideoEncoderError::TranscodeThreadPanicked)?
}

fn check_transcoder(worker: &mut Option<JoinHandle<Result<(), VideoEncoderError>>>) -> Result<(), VideoEncoderError> {
    if worker.as_ref().is_none_or(JoinHandle::is_finished) {
        if let Some(worker) = worker.take() {
            join_transcoder(worker)?;
        }
        return Err(VideoEncoderError::TranscodeStopped);
    }
    Ok(())
}

fn send_encoder_payload<T>(
    sender: &mpsc::SyncSender<T>,
    payload: T,
    worker: &mut Option<JoinHandle<Result<(), VideoEncoderError>>>,
) -> Result<(), VideoEncoderError>
where
    VideoEncoderError: From<mpsc::SendError<T>>,
{
    send_encoder_payload_with_timeout(sender, payload, worker, Duration::from_secs(10))
}

fn send_encoder_payload_with_timeout<T>(
    sender: &mpsc::SyncSender<T>,
    mut payload: T,
    worker: &mut Option<JoinHandle<Result<(), VideoEncoderError>>>,
    timeout: Duration,
) -> Result<(), VideoEncoderError>
where
    VideoEncoderError: From<mpsc::SendError<T>>,
{
    let started = std::time::Instant::now();
    loop {
        check_transcoder(worker)?;
        match sender.try_send(payload) {
            Ok(()) => return check_transcoder(worker),
            Err(mpsc::TrySendError::Disconnected(value)) => {
                check_transcoder(worker)?;
                return Err(mpsc::SendError(value).into());
            }
            Err(mpsc::TrySendError::Full(value)) => {
                if started.elapsed() >= timeout {
                    return Err(VideoEncoderError::TranscodeQueueStalled);
                }
                payload = value;
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

fn close_encoder_channel<T>(sender: &mut mpsc::SyncSender<T>) {
    let (replacement, _) = encoder_channel();
    // Receiver::recv drains queued payloads before returning the EOS disconnect.
    drop(std::mem::replace(sender, replacement));
}

#[cfg(test)]
mod bounded_queue_tests {
    use super::*;
    use std::sync::mpsc::TrySendError;

    #[test]
    fn encoder_queue_applies_backpressure_and_preserves_end_of_stream() {
        let (sender, receiver) = encoder_channel();
        sender.send(Some(1)).unwrap();
        sender.send(Some(2)).unwrap();
        assert!(matches!(sender.try_send(Some(3)), Err(TrySendError::Full(Some(3)))));
        let producer = std::thread::spawn(move || {
            sender.send(Some(3)).unwrap();
            sender.send(None).unwrap();
        });
        let frames: Vec<_> = receiver.iter().collect();
        producer.join().unwrap();
        assert_eq!(frames, vec![Some(1), Some(2), Some(3), None]);
    }

    fn late_failure<T: Send + 'static>(payload: impl Fn() -> T)
    where
        VideoEncoderError: From<mpsc::SendError<T>>,
    {
        let (mut sender, receiver) = encoder_channel();
        sender.send(payload()).ok().unwrap();
        sender.send(payload()).ok().unwrap();
        let mut worker = Some(thread::spawn(|| {
            thread::sleep(Duration::from_millis(30));
            Err(VideoEncoderError::IoError(std::io::Error::other("injected late encoder failure")))
        }));
        let started = std::time::Instant::now();
        let error = send_encoder_payload(&sender, payload(), &mut worker).unwrap_err();
        assert!(error.to_string().contains("injected late encoder failure"));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(worker.is_none(), "Completed worker must be joined, not detached.");
        close_encoder_channel(&mut sender);
        assert_eq!(receiver.into_iter().count(), 2);
    }

    #[test]
    fn full_video_and_audio_queues_report_late_consumer_failure() {
        late_failure(|| Some((VideoEncoderSource::Buffer(vec![0; 4]), TimeSpan { Duration: 0 })));
        late_failure(|| Some((AudioEncoderSource::Buffer(vec![0; 4]), TimeSpan { Duration: 0 })));
    }

    #[test]
    fn full_queue_with_a_live_stalled_encoder_returns_without_losing_accepted_frames() {
        let (sender, receiver) = encoder_channel();
        let payload = || Some((VideoEncoderSource::Buffer(vec![0; 4]), TimeSpan { Duration: 0 }));
        sender.send(payload()).unwrap();
        sender.send(payload()).unwrap();
        let (release, wait) = mpsc::channel();
        let mut worker = Some(thread::spawn(move || {
            let _ = wait.recv_timeout(Duration::from_secs(1));
            Ok(())
        }));
        let error =
            send_encoder_payload_with_timeout(&sender, payload(), &mut worker, Duration::from_millis(25)).unwrap_err();
        assert!(worker.is_some() && error.to_string().contains("queue"), "{error}");
        release.send(()).unwrap();
        join_transcoder(worker.take().unwrap()).unwrap();
        drop(sender);
        assert_eq!(receiver.into_iter().count(), 2);
    }

    #[test]
    fn closing_full_encoder_queues_preserves_samples_and_signals_eos() {
        let (mut video, video_rx) = encoder_channel();
        let (mut audio, audio_rx) = encoder_channel();
        for value in 0..2 {
            video.send(Some(value)).unwrap();
            audio.send(Some(value + 10)).unwrap();
        }
        let (release, begin) = mpsc::sync_channel(0);
        let worker = thread::spawn(move || {
            begin.recv().unwrap();
            assert_eq!(video_rx.into_iter().collect::<Vec<_>>(), [Some(0), Some(1)]);
            assert_eq!(audio_rx.into_iter().collect::<Vec<_>>(), [Some(10), Some(11)]);
        });
        close_encoder_channel(&mut video);
        close_encoder_channel(&mut audio);
        release.send(()).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn completed_or_panicked_consumer_is_not_accepted_as_live() {
        for panics in [false, true] {
            let (sender, _receiver) = encoder_channel();
            let mut worker = Some(thread::spawn(move || {
                assert!(!panics, "injected encoder worker panic");
                Ok(())
            }));
            while !worker.as_ref().unwrap().is_finished() {
                thread::sleep(Duration::from_millis(1));
            }
            let error = send_encoder_payload(
                &sender,
                Some((VideoEncoderSource::Buffer(vec![0; 4]), TimeSpan { Duration: 0 })),
                &mut worker,
            )
            .unwrap_err();
            assert!(matches!(error, VideoEncoderError::TranscodeThreadPanicked) == panics);
            assert!(matches!(error, VideoEncoderError::TranscodeStopped) != panics);
            assert!(worker.is_none());
        }
    }
}

#[cfg(test)]
mod apartment_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::Win32::System::Com::{APTTYPE, APTTYPE_MTA, APTTYPEQUALIFIER, CoGetApartmentType};

    fn assert_mta() {
        let mut apartment = APTTYPE::default();
        let mut qualifier = APTTYPEQUALIFIER::default();
        unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) }.unwrap();
        assert_eq!(apartment, APTTYPE_MTA);
    }

    struct Payload {
        transcoder: MediaTranscoder,
        destroyed: Arc<AtomicBool>,
    }

    impl Drop for Payload {
        fn drop(&mut self) {
            assert_mta();
            self.transcoder.HardwareAccelerationEnabled().unwrap();
            self.destroyed.store(true, Ordering::Release);
        }
    }

    #[test]
    fn transcoder_worker_initializes_before_work_and_releases_payload_before_uninitialize() {
        let _runtime = crate::winrt::WinRT::new().unwrap();
        let destroyed = Arc::new(AtomicBool::new(false));
        let pending = Arc::new(Mutex::new(Some(Payload {
            transcoder: MediaTranscoder::new().unwrap(),
            destroyed: destroyed.clone(),
        })));
        let worker_pending = pending.clone();
        let worker = thread::spawn(move || {
            run_initialized_transcoder(worker_pending, crate::winrt::WinRT::new, |payload| {
                assert_mta();
                payload.transcoder.SetHardwareAccelerationEnabled(true)?;
                Err(VideoEncoderError::TranscodeStopped)
            })
        });
        assert!(matches!(join_transcoder(worker), Err(VideoEncoderError::TranscodeStopped)));
        assert!(pending.lock().is_none());
        assert!(destroyed.load(Ordering::Acquire));
    }

    #[test]
    fn failed_transcoder_initialization_keeps_winrt_payload_for_parent_cleanup() {
        let _runtime = crate::winrt::WinRT::new().unwrap();
        let destroyed = Arc::new(AtomicBool::new(false));
        let pending = Arc::new(Mutex::new(Some(Payload {
            transcoder: MediaTranscoder::new().unwrap(),
            destroyed: destroyed.clone(),
        })));
        let worker_pending = pending.clone();
        let worker = thread::spawn(move || {
            run_initialized_transcoder(
                worker_pending,
                || Err::<(), _>(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL)),
                |_| panic!("Uninitialized worker must not consume the payload."),
            )
        });
        assert!(matches!(join_transcoder(worker), Err(VideoEncoderError::WindowsError(_))));
        assert!(pending.lock().is_some());
        assert!(!destroyed.load(Ordering::Acquire));
        drop(pending.lock().take());
        assert!(destroyed.load(Ordering::Acquire));
    }
}

#[derive(thiserror::Error, Debug)]
/// Errors that can occur when encoding raw buffers to images via [`ImageEncoder`].
pub enum ImageEncoderError {
    /// The provided source pixel format is not supported for image encoding.
    ///
    /// This occurs for formats such as [`crate::settings::ColorFormat::Rgba16F`].
    #[error("This color format is not supported for saving as an image")]
    UnsupportedFormat,
    /// An I/O error occurred while writing the image to disk.
    ///
    /// Wraps [`std::io::Error`].
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    /// An integer conversion failed during buffer sizing or Windows API calls.
    ///
    /// Wraps [`std::num::TryFromIntError`].
    #[error("Integer conversion error: {0}")]
    IntConversionError(#[from] std::num::TryFromIntError),
    /// A Windows Runtime/Win32 API call failed.
    ///
    /// Wraps [`windows::core::Error`].
    #[error("Windows API error: {0}")]
    WindowsError(#[from] windows::core::Error),
}

#[derive(Eq, PartialEq, Clone, Copy, Debug)]
/// Supported output image formats for [`crate::encoder::ImageEncoder`].
pub enum ImageFormat {
    /// JPEG (lossy).
    Jpeg,
    /// PNG (lossless).
    Png,
    /// GIF (palette-based).
    Gif,
    /// TIFF (Tagged Image File Format).
    Tiff,
    /// BMP (Bitmap).
    Bmp,
    /// JPEG XR (HD Photo).
    JpegXr,
}

/// Pixel formats supported by the Windows API for image encoding.
#[derive(Eq, PartialEq, Clone, Copy, Debug)]
pub enum ImageEncoderPixelFormat {
    /// 16-bit floating-point RGBA format.
    Rgb16F,
    /// 8-bit unsigned integer BGRA format.
    Bgra8,
    /// 8-bit unsigned integer RGBA format.
    Rgba8,
}

/// Encodes raw image buffers into encoded bytes for common formats.
///
/// Supports saving as PNG, JPEG, GIF, TIFF, BMP, and JPEG XR when the input
/// color format is compatible.
///
/// # Example
/// ```no_run
/// use windows_capture::encoder::{ImageEncoder, ImageEncoderPixelFormat, ImageFormat};
///
/// let width = 320u32;
/// let height = 240u32;
/// // BGRA8 buffer (e.g., from a frame)
/// let bgra = vec![0u8; (width * height * 4) as usize];
///
/// let png_bytes = ImageEncoder::new(ImageFormat::Png, ImageEncoderPixelFormat::Bgra8)
///     .unwrap()
///     .encode(&bgra, width, height)
///     .unwrap();
///
/// std::fs::write("example.png", png_bytes).unwrap();
/// ```
pub struct ImageEncoder {
    encoder: windows::core::GUID,
    pixel_format: BitmapPixelFormat,
}

impl ImageEncoder {
    /// Constructs a new [`ImageEncoder`].
    #[inline]
    pub fn new(format: ImageFormat, pixel_format: ImageEncoderPixelFormat) -> Result<Self, ImageEncoderError> {
        let encoder = match format {
            ImageFormat::Jpeg => BitmapEncoder::JpegEncoderId()?,
            ImageFormat::Png => BitmapEncoder::PngEncoderId()?,
            ImageFormat::Gif => BitmapEncoder::GifEncoderId()?,
            ImageFormat::Tiff => BitmapEncoder::TiffEncoderId()?,
            ImageFormat::Bmp => BitmapEncoder::BmpEncoderId()?,
            ImageFormat::JpegXr => BitmapEncoder::JpegXREncoderId()?,
        };

        let pixel_format = match pixel_format {
            ImageEncoderPixelFormat::Bgra8 => BitmapPixelFormat::Bgra8,
            ImageEncoderPixelFormat::Rgba8 => BitmapPixelFormat::Rgba8,
            ImageEncoderPixelFormat::Rgb16F => BitmapPixelFormat::Rgba16,
        };

        Ok(Self { pixel_format, encoder })
    }

    /// Encodes the provided pixel buffer into the configured output [`ImageFormat`].
    ///
    /// The input buffer must match the specified source [`crate::settings::ColorFormat`]
    /// and dimensions. For packed 8-bit formats (e.g., [`crate::settings::ColorFormat::Bgra8`]),
    /// the buffer length should be `width * height * 4`.
    ///
    /// # Errors
    ///
    /// - [`ImageEncoderError::UnsupportedFormat`] when the source format is unsupported for images
    ///   (e.g., [`crate::settings::ColorFormat::Rgba16F`])
    /// - [`ImageEncoderError::WindowsError`] when Windows Imaging API calls fail
    /// - [`ImageEncoderError::IntConversionError`] on integer conversion failures
    #[inline]
    pub fn encode(&self, image_buffer: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ImageEncoderError> {
        let stream = InMemoryRandomAccessStream::new()?;

        let encoder = BitmapEncoder::CreateAsync(self.encoder, &stream)?.join()?;

        encoder.SetPixelData(
            self.pixel_format,
            BitmapAlphaMode::Premultiplied,
            width,
            height,
            1.0,
            1.0,
            image_buffer,
        )?;
        encoder.FlushAsync()?.join()?;

        let size = stream.Size()?;
        let input = stream.GetInputStreamAt(0)?;
        let reader = DataReader::CreateDataReader(&input)?;
        reader.LoadAsync(size as u32)?.join()?;

        let mut bytes = vec![0u8; size as usize];
        reader.ReadBytes(&mut bytes)?;

        Ok(bytes)
    }
}

#[derive(thiserror::Error, Debug)]
/// Errors emitted by [`VideoEncoder`] during configuration, streaming, or finalization.
pub enum VideoEncoderError {
    /// A Windows Runtime/Win32 API call failed.
    ///
    /// Wraps [`windows::core::Error`].
    #[error("Windows API error: {0}")]
    WindowsError(#[from] windows::core::Error),
    /// Failed to send a video sample into the internal pipeline.
    ///
    /// Typically indicates the internal channel is closed.
    #[error("Failed to send frame: {0}")]
    FrameSendError(#[from] mpsc::SendError<Option<(VideoEncoderSource, TimeSpan)>>),
    /// Failed to send an audio sample into the internal pipeline.
    ///
    /// Typically indicates the internal channel is closed.
    #[error("Failed to send audio: {0}")]
    AudioSendError(#[from] mpsc::SendError<Option<(AudioEncoderSource, TimeSpan)>>),
    /// Video encoding was disabled via [`VideoSettingsBuilder::disabled`].
    #[error("Video encoding is disabled")]
    VideoDisabled,
    /// Audio encoding was disabled via [`AudioSettingsBuilder::disabled`].
    #[error("Audio encoding is disabled")]
    AudioDisabled,
    /// An I/O error occurred during file creation or writing.
    ///
    /// Wraps [`std::io::Error`].
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    /// The provided frame color format is unsupported by the encoder path.
    ///
    /// See [`crate::settings::ColorFormat`].
    #[error("Unsupported frame color format: {0:?}")]
    UnsupportedFrameFormat(ColorFormat),
    /// The native transcoder exited before the producer finished sending samples.
    #[error("The Windows media transcoder stopped accepting samples")]
    TranscodeStopped,
    /// The live transcoder left its bounded sample queue full throughout the submission deadline.
    #[error("The Windows media encoder queue stayed full and recording could not continue")]
    TranscodeQueueStalled,
    /// The owned transcoder worker panicked.
    #[error("The Windows media transcoder worker failed")]
    TranscodeThreadPanicked,
}

unsafe impl Send for VideoEncoderError {}
unsafe impl Sync for VideoEncoderError {}

/// Video sources used by [`VideoEncoder`].
///
/// - For [`VideoEncoderSource::DirectX`], the COM surface pointer is ref-counted; holding the
///   pointer is sufficient.
/// - For [`VideoEncoderSource::Buffer`], the encoder owns the queued bytes. Submission waits when
///   the bounded queue is full.
pub enum VideoEncoderSource {
    /// A Direct3D surface sample.
    DirectX(SendDirectX<IDirect3DSurface>),
    /// A raw BGRA sample buffer.
    Buffer(Vec<u8>),
}

/// Audio sources used by [`VideoEncoder`]. The encoder takes ownership of the bytes.
pub enum AudioEncoderSource {
    /// Interleaved PCM bytes.
    Buffer(Vec<u8>),
}

struct OwnedSurface {
    texture: SendDirectX<ID3D11Texture2D>,
    surface: SendDirectX<IDirect3DSurface>,
    render_target_view: Option<SendDirectX<ID3D11RenderTargetView>>,
}

/// Builder for configuring video encoder settings.
pub struct VideoSettingsBuilder {
    sub_type: VideoSettingsSubType,
    bitrate: u32,
    width: u32,
    height: u32,
    frame_rate: u32,
    pixel_aspect_ratio: (u32, u32),
    disabled: bool,
}

impl VideoSettingsBuilder {
    /// Constructs a new [`VideoSettingsBuilder`] with required geometry.
    ///
    /// Defaults:
    /// - Subtype: [`VideoSettingsSubType::HEVC`]
    /// - Bitrate: 15 Mbps
    /// - Frame rate: 60 fps
    /// - Pixel aspect ratio: 1:1
    /// - Disabled: false
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            bitrate: 15_000_000,
            frame_rate: 60,
            pixel_aspect_ratio: (1, 1),
            sub_type: VideoSettingsSubType::HEVC,
            width,
            height,
            disabled: false,
        }
    }

    /// Sets the video codec/subtype (e.g., [`VideoSettingsSubType::HEVC`]).
    pub const fn sub_type(mut self, sub_type: VideoSettingsSubType) -> Self {
        self.sub_type = sub_type;
        self
    }

    /// Sets target bitrate in bits per second.
    pub const fn bitrate(mut self, bitrate: u32) -> Self {
        self.bitrate = bitrate;
        self
    }

    /// Sets target frame width in pixels.
    pub const fn width(mut self, width: u32) -> Self {
        self.width = width;
        self
    }

    /// Sets target frame height in pixels.
    pub const fn height(mut self, height: u32) -> Self {
        self.height = height;
        self
    }

    /// Sets target frame rate (numerator; denominator is fixed to 1).
    pub const fn frame_rate(mut self, frame_rate: u32) -> Self {
        self.frame_rate = frame_rate;
        self
    }

    /// Sets pixel aspect ratio as (numerator, denominator).
    pub const fn pixel_aspect_ratio(mut self, par: (u32, u32)) -> Self {
        self.pixel_aspect_ratio = par;
        self
    }
    /// Disables or enables video encoding.
    ///
    /// When `true`, calls to send frames still succeed but produce no video samples.
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn build(self) -> Result<(VideoEncodingProperties, bool), VideoEncoderError> {
        let properties = VideoEncodingProperties::new()?;
        properties.SetSubtype(&self.sub_type.to_hstring())?;
        properties.SetBitrate(self.bitrate)?;
        properties.SetWidth(self.width)?;
        properties.SetHeight(self.height)?;
        properties.FrameRate()?.SetNumerator(self.frame_rate)?;
        properties.FrameRate()?.SetDenominator(1)?;
        properties.PixelAspectRatio()?.SetNumerator(self.pixel_aspect_ratio.0)?;
        properties.PixelAspectRatio()?.SetDenominator(self.pixel_aspect_ratio.1)?;
        Ok((properties, self.disabled))
    }
}

/// Builder for configuring audio encoder settings.
pub struct AudioSettingsBuilder {
    bitrate: u32,
    channel_count: u32,
    sample_rate: u32,
    bit_per_sample: u32,
    sub_type: AudioSettingsSubType,
    disabled: bool,
}

impl AudioSettingsBuilder {
    /// Constructs a new [`AudioSettingsBuilder`] with common defaults.
    ///
    /// Defaults:
    /// - Bitrate: 192 kbps
    /// - Channels: 2
    /// - Sample rate: 48 kHz
    /// - Bits per sample: 16
    /// - Subtype: [`AudioSettingsSubType::AAC`]
    /// - Disabled: false
    pub const fn new() -> Self {
        Self {
            bitrate: 192_000,
            channel_count: 2,
            sample_rate: 48_000,
            bit_per_sample: 16,
            sub_type: AudioSettingsSubType::AAC,
            disabled: false,
        }
    }
    /// Sets audio bitrate in bits per second.
    pub const fn bitrate(mut self, bitrate: u32) -> Self {
        self.bitrate = bitrate;
        self
    }
    /// Sets number of interleaved channels.
    pub const fn channel_count(mut self, channel_count: u32) -> Self {
        self.channel_count = channel_count;
        self
    }
    /// Sets sample rate in Hz.
    pub const fn sample_rate(mut self, sample_rate: u32) -> Self {
        self.sample_rate = sample_rate;
        self
    }
    /// Sets bits per sample.
    pub const fn bit_per_sample(mut self, bit_per_sample: u32) -> Self {
        self.bit_per_sample = bit_per_sample;
        self
    }
    /// Sets audio codec/subtype (e.g., [`AudioSettingsSubType::AAC`]).
    pub const fn sub_type(mut self, sub_type: AudioSettingsSubType) -> Self {
        self.sub_type = sub_type;
        self
    }
    /// Disables or enables audio encoding.
    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn build(self) -> Result<(AudioEncodingProperties, bool), VideoEncoderError> {
        let properties = AudioEncodingProperties::new()?;
        properties.SetBitrate(self.bitrate)?;
        properties.SetChannelCount(self.channel_count)?;
        properties.SetSampleRate(self.sample_rate)?;
        properties.SetBitsPerSample(self.bit_per_sample)?;
        properties.SetSubtype(&self.sub_type.to_hstring())?;
        Ok((properties, self.disabled))
    }
}

impl Default for AudioSettingsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for configuring container settings.
pub struct ContainerSettingsBuilder {
    sub_type: ContainerSettingsSubType,
}
impl ContainerSettingsBuilder {
    /// Constructs a new [`ContainerSettingsBuilder`].
    ///
    /// Default subtype: [`ContainerSettingsSubType::MPEG4`].
    pub const fn new() -> Self {
        Self { sub_type: ContainerSettingsSubType::MPEG4 }
    }
    /// Sets the container subtype (e.g., [`ContainerSettingsSubType::MPEG4`]).
    pub const fn sub_type(mut self, sub_type: ContainerSettingsSubType) -> Self {
        self.sub_type = sub_type;
        self
    }
    fn build(self) -> Result<ContainerEncodingProperties, VideoEncoderError> {
        let properties = ContainerEncodingProperties::new()?;
        properties.SetSubtype(&self.sub_type.to_hstring())?;
        Ok(properties)
    }
}
impl Default for ContainerSettingsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Video encoder subtypes.
#[derive(Eq, PartialEq, Clone, Copy, Debug)]
pub enum VideoSettingsSubType {
    /// Uncompressed 32-bit ARGB (8:8:8:8).
    ARGB32,
    /// Uncompressed 32-bit BGRA (8:8:8:8).
    BGRA8,
    /// 16-bit depth format.
    D16,
    /// H.263 video.
    H263,
    /// H.264/AVC video.
    H264,
    /// H.264 elementary stream.
    H264ES,
    /// H.265/HEVC video.
    HEVC,
    /// H.265/HEVC elementary stream.
    HEVCES,
    /// Planar YUV 4:2:0 (IYUV).
    IYUV,
    /// 8-bit luminance (grayscale).
    L8,
    /// 16-bit luminance (grayscale).
    L16,
    /// Motion JPEG.
    MJPG,
    /// NV12 YUV 4:2:0 (semi-planar).
    NV12,
    /// MPEG-1 video.
    MPEG1,
    /// MPEG-2 video.
    MPEG2,
    /// 24-bit RGB.
    RGB24,
    /// 32-bit RGB.
    RGB32,
    /// Windows Media Video 9 (WMV3).
    WMV3,
    /// Windows Media Video Advanced Profile (VC-1).
    WVC1,
    /// VP9 video.
    VP9,
    /// Packed YUY2 4:2:2.
    YUY2,
    /// Planar YV12 4:2:0.
    YV12,
}
impl VideoSettingsSubType {
    /// Returns the Windows Media subtype identifier string for this [`VideoSettingsSubType`].
    pub fn to_hstring(&self) -> HSTRING {
        let s = match self {
            Self::ARGB32 => "ARGB32",
            Self::BGRA8 => "BGRA8",
            Self::D16 => "D16",
            Self::H263 => "H263",
            Self::H264 => "H264",
            Self::H264ES => "H264ES",
            Self::HEVC => "HEVC",
            Self::HEVCES => "HEVCES",
            Self::IYUV => "IYUV",
            Self::L8 => "L8",
            Self::L16 => "L16",
            Self::MJPG => "MJPG",
            Self::NV12 => "NV12",
            Self::MPEG1 => "MPEG1",
            Self::MPEG2 => "MPEG2",
            Self::RGB24 => "RGB24",
            Self::RGB32 => "RGB32",
            Self::WMV3 => "WMV3",
            Self::WVC1 => "WVC1",
            Self::VP9 => "VP9",
            Self::YUY2 => "YUY2",
            Self::YV12 => "YV12",
        };
        HSTRING::from(s)
    }
}

/// Audio encoder subtypes.
#[derive(Eq, PartialEq, Clone, Copy, Debug)]
pub enum AudioSettingsSubType {
    /// Advanced Audio Coding (AAC).
    AAC,
    /// Dolby Digital (AC-3).
    AC3,
    /// AAC framed with ADTS headers.
    AACADTS,
    /// AAC with HDCP protection.
    AACHDCP,
    /// AC-3 over S/PDIF.
    AC3SPDIF,
    /// AC-3 with HDCP protection.
    AC3HDCP,
    /// ADTS (Audio Data Transport Stream).
    ADTS,
    /// Apple Lossless Audio Codec (ALAC).
    ALAC,
    /// Adaptive Multi-Rate Narrowband (AMR-NB).
    AMRNB,
    /// Adaptive Multi-Rate Wideband (AMR-WB).
    AWRWB,
    /// DTS audio.
    DTS,
    /// Enhanced AC-3 (E-AC-3).
    EAC3,
    /// Free Lossless Audio Codec (FLAC).
    FLAC,
    /// 32-bit floating-point PCM.
    Float,
    /// MPEG-1/2 Layer III (MP3).
    MP3,
    /// Generic MPEG audio.
    MPEG,
    /// Opus audio.
    OPUS,
    /// Pulse-code modulation (PCM).
    PCM,
    /// Windows Media Audio 8.
    WMA8,
    /// Windows Media Audio 9.
    WMA9,
    /// Vorbis audio.
    Vorbis,
}
impl AudioSettingsSubType {
    /// Returns the Windows Media subtype identifier string for this [`AudioSettingsSubType`].
    pub fn to_hstring(&self) -> HSTRING {
        let s = match self {
            Self::AAC => "AAC",
            Self::AC3 => "AC3",
            Self::AACADTS => "AACADTS",
            Self::AACHDCP => "AACHDCP",
            Self::AC3SPDIF => "AC3SPDIF",
            Self::AC3HDCP => "AC3HDCP",
            Self::ADTS => "ADTS",
            Self::ALAC => "ALAC",
            Self::AMRNB => "AMRNB",
            Self::AWRWB => "AWRWB",
            Self::DTS => "DTS",
            Self::EAC3 => "EAC3",
            Self::FLAC => "FLAC",
            Self::Float => "Float",
            Self::MP3 => "MP3",
            Self::MPEG => "MPEG",
            Self::OPUS => "OPUS",
            Self::PCM => "PCM",
            Self::WMA8 => "WMA8",
            Self::WMA9 => "WMA9",
            Self::Vorbis => "Vorbis",
        };
        HSTRING::from(s)
    }
}

/// Container subtypes.
#[derive(Eq, PartialEq, Clone, Copy, Debug)]
pub enum ContainerSettingsSubType {
    /// Advanced Systems Format (ASF).
    ASF,
    /// Raw MP3 container.
    MP3,
    /// MPEG-4 container (e.g., MP4).
    MPEG4,
    /// Audio Video Interleave (AVI).
    AVI,
    /// MPEG-2 container.
    MPEG2,
    /// WAVE (WAV) container.
    WAVE,
    /// AAC ADTS stream.
    AACADTS,
    /// ADTS container.
    ADTS,
    /// 3GP container.
    GP3,
    /// AMR container.
    AMR,
    /// FLAC container.
    FLAC,
}
impl ContainerSettingsSubType {
    /// Returns the Windows Media container subtype identifier string for this
    /// [`ContainerSettingsSubType`].
    pub fn to_hstring(&self) -> HSTRING {
        match self {
            Self::ASF => HSTRING::from("ASF"),
            Self::MP3 => HSTRING::from("MP3"),
            Self::MPEG4 => HSTRING::from("MPEG4"),
            Self::AVI => HSTRING::from("AVI"),
            Self::MPEG2 => HSTRING::from("MPEG2"),
            Self::WAVE => HSTRING::from("WAVE"),
            Self::AACADTS => HSTRING::from("AACADTS"),
            Self::ADTS => HSTRING::from("ADTS"),
            Self::GP3 => HSTRING::from("3GP"),
            Self::AMR => HSTRING::from("AMR"),
            Self::FLAC => HSTRING::from("FLAC"),
        }
    }
}

/// Encodes video frames (and optional audio) and writes them to a file or stream.
///
/// Frames are provided as Direct3D surfaces or raw BGRA buffers. Audio can be pushed
/// as interleaved PCM bytes.
///
/// - Use [`VideoEncoder::new`] for file output or [`VideoEncoder::new_from_stream`] for stream
///   output.
/// - Push frames with [`VideoEncoder::send_frame`] or [`VideoEncoder::send_frame_buffer`].
/// - Optionally push audio with [`VideoEncoder::send_audio_buffer`] or use
///   [`VideoEncoder::send_frame_with_audio`].
/// - Call [`VideoEncoder::finish`] to finalize the container.
///
/// # Example
/// ```no_run
/// use windows_capture::encoder::{
///     AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
/// };
///
/// // Create an encoder that outputs H.265 in an MP4 container
/// let mut encoder = VideoEncoder::new(
///     VideoSettingsBuilder::new(1920, 1080),
///     AudioSettingsBuilder::new().disabled(true),
///     ContainerSettingsBuilder::new(),
///     "capture.mp4",
/// )
/// .unwrap();
///
/// // In your capture loop, push frames:
/// // encoder.send_frame(&frame).unwrap();
///
/// // When done:
/// // encoder.finish().unwrap();
/// ```
pub struct VideoEncoder {
    // Video timing
    first_timestamp: Option<TimeSpan>,

    // Channels
    frame_sender: mpsc::SyncSender<Option<(VideoEncoderSource, TimeSpan)>>,
    audio_sender: mpsc::SyncSender<Option<(AudioEncoderSource, TimeSpan)>>,

    // MSS event tokens
    sample_requested: i64,
    media_stream_source: MediaStreamSource,
    starting: i64,

    // Transcode worker
    transcode_thread: Option<JoinHandle<Result<(), VideoEncoderError>>>,
    // Failed worker initialization leaves WinRT objects owned by the initialized caller.
    _pending_transcode: PendingTranscode,

    // Feature toggles
    is_video_disabled: bool,
    is_audio_disabled: bool,

    // --- NEW: audio clock & format bookkeeping (monotonic timing) ---
    audio_sample_rate: u32,  // Hz (frames per second)
    audio_block_align: u32,  // bytes per interleaved sample frame (channels * (bits/8))
    audio_samples_sent: u64, // number of sample frames (not bytes) emitted so far

    // Video sizing constraints
    target_width: u32,
    target_height: u32,
}

impl VideoEncoder {
    fn create_owned_surface(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        format: ColorFormat,
    ) -> Result<OwnedSurface, VideoEncoderError> {
        let texture_desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT(format as i32),
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };

        let mut texture = None;
        unsafe {
            device.CreateTexture2D(&texture_desc, None, Some(&mut texture))?;
        }
        let texture = texture.expect("CreateTexture2D returned None");

        let mut render_target = None;
        unsafe {
            device.CreateRenderTargetView(&texture, None, Some(&mut render_target))?;
        }
        let render_target_view = render_target.map(SendDirectX::new);

        let dxgi_surface: IDXGISurface = texture.cast()?;
        let inspectable = unsafe { CreateDirect3D11SurfaceFromDXGISurface(&dxgi_surface)? };
        let surface: IDirect3DSurface = inspectable.cast()?;

        Ok(OwnedSurface { texture: SendDirectX::new(texture), surface: SendDirectX::new(surface), render_target_view })
    }

    fn attach_sample_requested_handlers(
        media_stream_source: &MediaStreamSource,
        is_video_disabled: bool,
        is_audio_disabled: bool,
        frame_receiver: VideoFrameReceiver,
        audio_receiver: AudioFrameReceiver,
        audio_block_align: u32,
        audio_sample_rate: u32,
    ) -> Result<i64, VideoEncoderError> {
        let token = media_stream_source.SampleRequested(&TypedEventHandler::<
            MediaStreamSource,
            MediaStreamSourceSampleRequestedEventArgs,
        >::new(move |_, sample_requested| {
            let sample_requested = sample_requested
                .as_ref()
                .expect("MediaStreamSource SampleRequested parameter was None. This should not happen.");

            let request = sample_requested.Request()?;
            let is_audio = request.StreamDescriptor()?.cast::<AudioStreamDescriptor>().is_ok();

            // Always offload blocking work to the thread pool; never block the MSS event
            // thread.
            let deferral = request.GetDeferral()?;

            if is_audio {
                if is_audio_disabled {
                    request.SetSample(None)?;
                    deferral.Complete()?;
                } else {
                    let request_clone = request;
                    let audio_receiver = audio_receiver.clone();
                    ThreadPool::RunWithPriorityAndOptionsAsync(
                        &WorkItemHandler::new(move |_| {
                            let value = audio_receiver.lock().recv();
                            match value {
                                Ok(Some((source, timestamp))) => {
                                    let sample = match source {
                                        AudioEncoderSource::Buffer(bytes) => {
                                            let buf = CryptographicBuffer::CreateFromByteArray(&bytes)?;
                                            let sample = MediaStreamSample::CreateFromBuffer(&buf, timestamp)?;
                                            // Duration = (frames / sample_rate) in 100ns ticks
                                            // frames = bytes / block_align
                                            let frames = (bytes.len() as u32) / audio_block_align;
                                            let duration_ticks =
                                                (frames as i64) * 10_000_000i64 / (audio_sample_rate as i64);
                                            sample.SetDuration(TimeSpan { Duration: duration_ticks })?;
                                            sample
                                        }
                                    };
                                    request_clone.SetSample(&sample)?;
                                }
                                Ok(None) | Err(_) => {
                                    request_clone.SetSample(None)?;
                                }
                            }
                            deferral.Complete()?;
                            Ok(())
                        }),
                        WorkItemPriority::Normal,
                        WorkItemOptions::None,
                    )?;
                }
            } else if is_video_disabled {
                request.SetSample(None)?;
                deferral.Complete()?;
            } else {
                let request_clone = request;
                let frame_receiver = frame_receiver.clone();
                ThreadPool::RunWithPriorityAndOptionsAsync(
                    &WorkItemHandler::new(move |_| {
                        let value = frame_receiver.lock().recv();
                        match value {
                            Ok(Some((source, timestamp))) => {
                                let sample = match source {
                                    VideoEncoderSource::DirectX(surface) => {
                                        MediaStreamSample::CreateFromDirect3D11Surface(&surface.0, timestamp)?
                                    }
                                    VideoEncoderSource::Buffer(bytes) => {
                                        let buf = CryptographicBuffer::CreateFromByteArray(&bytes)?;
                                        MediaStreamSample::CreateFromBuffer(&buf, timestamp)?
                                    }
                                };
                                request_clone.SetSample(&sample)?;
                            }
                            Ok(None) | Err(_) => {
                                request_clone.SetSample(None)?;
                            }
                        }
                        deferral.Complete()?;
                        Ok(())
                    }),
                    WorkItemPriority::Normal,
                    WorkItemOptions::None,
                )?;
            }

            Ok(())
        }))?;
        Ok(token)
    }

    /// Constructs a new `VideoEncoder` that writes to a file path.
    #[inline]
    pub fn new<P: AsRef<Path>>(
        video_settings: VideoSettingsBuilder,
        audio_settings: AudioSettingsBuilder,
        container_settings: ContainerSettingsBuilder,
        path: P,
    ) -> Result<Self, VideoEncoderError> {
        let path = path.as_ref();
        let media_encoding_profile = MediaEncodingProfile::new()?;

        let (video_encoding_properties_cfg, is_video_disabled) = video_settings.build()?;
        media_encoding_profile.SetVideo(&video_encoding_properties_cfg)?;
        let (audio_encoding_properties_cfg, is_audio_disabled) = audio_settings.build()?;
        media_encoding_profile.SetAudio(&audio_encoding_properties_cfg)?;
        let container_encoding_properties = container_settings.build()?;
        media_encoding_profile.SetContainer(&container_encoding_properties)?;

        let target_width = video_encoding_properties_cfg.Width()?;
        let target_height = video_encoding_properties_cfg.Height()?;

        let video_encoding_properties = VideoEncodingProperties::CreateUncompressed(
            &MediaEncodingSubtypes::Bgra8()?,
            video_encoding_properties_cfg.Width()?,
            video_encoding_properties_cfg.Height()?,
        )?;
        let video_stream_descriptor = VideoStreamDescriptor::Create(&video_encoding_properties)?;

        // Stream descriptor uses PCM; the profile still encodes to AAC/OPUS/etc.
        let audio_desc_props = AudioEncodingProperties::CreatePcm(
            audio_encoding_properties_cfg.SampleRate()?,
            audio_encoding_properties_cfg.ChannelCount()?,
            audio_encoding_properties_cfg.BitsPerSample()?,
        )?;
        let audio_stream_descriptor = AudioStreamDescriptor::Create(&audio_desc_props)?;

        // Compute audio block align/sample-rate for monotonic clock
        let audio_sr = audio_desc_props.SampleRate()?;
        let audio_ch = audio_desc_props.ChannelCount()?;
        let audio_bps = audio_desc_props.BitsPerSample()?;
        let audio_block_align = (audio_bps / 8) * audio_ch;

        let media_stream_source =
            MediaStreamSource::CreateFromDescriptors(&video_stream_descriptor, &audio_stream_descriptor)?;
        // Keep a modest buffer (30ms)
        media_stream_source.SetBufferTime(Duration::from_millis(30).into())?;

        let starting = media_stream_source.Starting(&TypedEventHandler::<
            MediaStreamSource,
            MediaStreamSourceStartingEventArgs,
        >::new(move |_, stream_start| {
            let stream_start =
                stream_start.as_ref().expect("MediaStreamSource Starting parameter was None. This should not happen.");
            stream_start.Request()?.SetActualStartPosition(TimeSpan { Duration: 0 })?;
            Ok(())
        }))?;

        let (frame_sender, frame_receiver_raw) = encoder_channel::<Option<(VideoEncoderSource, TimeSpan)>>();
        let (audio_sender, audio_receiver_raw) = encoder_channel::<Option<(AudioEncoderSource, TimeSpan)>>();

        let frame_receiver = Arc::new(Mutex::new(frame_receiver_raw));
        let audio_receiver = Arc::new(Mutex::new(audio_receiver_raw));

        let sample_requested = Self::attach_sample_requested_handlers(
            &media_stream_source,
            is_video_disabled,
            is_audio_disabled,
            frame_receiver,
            audio_receiver,
            audio_block_align,
            audio_sr,
        )?;

        let media_transcoder = MediaTranscoder::new()?;
        media_transcoder.SetHardwareAccelerationEnabled(true)?;

        File::create(path)?;
        let path = fs::canonicalize(path)?.to_string_lossy()[4..].to_string();
        let path = Path::new(&path);
        let path = &HSTRING::from(path.as_os_str().to_os_string());

        let file = StorageFile::GetFileFromPathAsync(path)?.join()?;
        let media_stream_output = file.OpenAsync(FileAccessMode::ReadWrite)?.join()?;

        let transcode = media_transcoder
            .PrepareMediaStreamSourceTranscodeAsync(
                &media_stream_source,
                &media_stream_output,
                &media_encoding_profile,
            )?
            .join()?;

        let (transcode_thread, pending_transcode) = spawn_transcoder(transcode, media_transcoder)?;

        Ok(Self {
            first_timestamp: None,
            frame_sender,
            audio_sender,
            sample_requested,
            media_stream_source,
            starting,
            transcode_thread: Some(transcode_thread),
            _pending_transcode: pending_transcode,
            is_video_disabled,
            is_audio_disabled,
            audio_sample_rate: audio_sr,
            audio_block_align,
            audio_samples_sent: 0,
            target_width,
            target_height,
        })
    }

    /// Constructs a new `VideoEncoder` that writes to the given stream.
    ///
    /// Unlike [`VideoEncoder::new`], which writes directly to a file, this constructor writes
    /// encoded output into any [`IRandomAccessStream`]. Use [`InMemoryRandomAccessStream`] to
    /// keep the encoded video in memory (e.g., for network streaming or further processing).
    ///
    /// # Example
    /// ```no_run
    /// use windows::Storage::Streams::InMemoryRandomAccessStream;
    /// use windows::core::Interface;
    /// use windows_capture::encoder::{
    ///     AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
    /// };
    ///
    /// let stream = InMemoryRandomAccessStream::new().unwrap();
    ///
    /// let encoder = VideoEncoder::new_from_stream(
    ///     VideoSettingsBuilder::new(1920, 1080),
    ///     AudioSettingsBuilder::new().disabled(true),
    ///     ContainerSettingsBuilder::new(),
    ///     stream.cast().unwrap(),
    /// )
    /// .unwrap();
    /// ```
    #[inline]
    pub fn new_from_stream(
        video_settings: VideoSettingsBuilder,
        audio_settings: AudioSettingsBuilder,
        container_settings: ContainerSettingsBuilder,
        stream: IRandomAccessStream,
    ) -> Result<Self, VideoEncoderError> {
        let media_encoding_profile = MediaEncodingProfile::new()?;

        let (video_encoding_properties_cfg, is_video_disabled) = video_settings.build()?;
        media_encoding_profile.SetVideo(&video_encoding_properties_cfg)?;
        let (audio_encoding_properties_cfg, is_audio_disabled) = audio_settings.build()?;
        media_encoding_profile.SetAudio(&audio_encoding_properties_cfg)?;
        let container_encoding_properties = container_settings.build()?;
        media_encoding_profile.SetContainer(&container_encoding_properties)?;

        let target_width = video_encoding_properties_cfg.Width()?;
        let target_height = video_encoding_properties_cfg.Height()?;

        let video_encoding_properties = VideoEncodingProperties::CreateUncompressed(
            &MediaEncodingSubtypes::Bgra8()?,
            video_encoding_properties_cfg.Width()?,
            video_encoding_properties_cfg.Height()?,
        )?;
        let video_stream_descriptor = VideoStreamDescriptor::Create(&video_encoding_properties)?;

        let audio_desc_props = AudioEncodingProperties::CreatePcm(
            audio_encoding_properties_cfg.SampleRate()?,
            audio_encoding_properties_cfg.ChannelCount()?,
            audio_encoding_properties_cfg.BitsPerSample()?,
        )?;
        let audio_stream_descriptor = AudioStreamDescriptor::Create(&audio_desc_props)?;

        // Monotonic audio timing parameters
        let audio_sr = audio_desc_props.SampleRate()?;
        let audio_ch = audio_desc_props.ChannelCount()?;
        let audio_bps = audio_desc_props.BitsPerSample()?;
        let audio_block_align = (audio_bps / 8) * audio_ch;

        let media_stream_source =
            MediaStreamSource::CreateFromDescriptors(&video_stream_descriptor, &audio_stream_descriptor)?;
        // CHANGED: use 30ms buffer (was 0)
        media_stream_source.SetBufferTime(Duration::from_millis(30).into())?;

        let starting = media_stream_source.Starting(&TypedEventHandler::<
            MediaStreamSource,
            MediaStreamSourceStartingEventArgs,
        >::new(move |_, stream_start| {
            let stream_start =
                stream_start.as_ref().expect("MediaStreamSource Starting parameter was None. This should not happen.");
            stream_start.Request()?.SetActualStartPosition(TimeSpan { Duration: 0 })?;
            Ok(())
        }))?;

        let (frame_sender, frame_receiver_raw) = encoder_channel::<Option<(VideoEncoderSource, TimeSpan)>>();
        let (audio_sender, audio_receiver_raw) = encoder_channel::<Option<(AudioEncoderSource, TimeSpan)>>();

        let frame_receiver = Arc::new(Mutex::new(frame_receiver_raw));
        let audio_receiver = Arc::new(Mutex::new(audio_receiver_raw));

        let sample_requested = Self::attach_sample_requested_handlers(
            &media_stream_source,
            is_video_disabled,
            is_audio_disabled,
            frame_receiver,
            audio_receiver,
            audio_block_align,
            audio_sr,
        )?;

        let media_transcoder = MediaTranscoder::new()?;
        media_transcoder.SetHardwareAccelerationEnabled(true)?;

        let transcode = media_transcoder
            .PrepareMediaStreamSourceTranscodeAsync(&media_stream_source, &stream, &media_encoding_profile)?
            .join()?;

        let (transcode_thread, pending_transcode) = spawn_transcoder(transcode, media_transcoder)?;

        Ok(Self {
            first_timestamp: None,
            frame_sender,
            audio_sender,
            sample_requested,
            media_stream_source,
            starting,
            transcode_thread: Some(transcode_thread),
            _pending_transcode: pending_transcode,
            is_video_disabled,
            is_audio_disabled,
            audio_sample_rate: audio_sr,
            audio_block_align,
            audio_samples_sent: 0,
            target_width,
            target_height,
        })
    }

    fn copy_surface(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        source: &ID3D11Texture2D,
        source_size: (u32, u32),
        target_size: (u32, u32),
        format: ColorFormat,
    ) -> Result<OwnedSurface, VideoEncoderError> {
        let cache = Self::create_owned_surface(device, target_size.0, target_size.1, format)?;

        if let Some(rtv) = &cache.render_target_view {
            let clear_color = [0.0f32, 0.0, 0.0, 1.0];
            unsafe {
                context.ClearRenderTargetView(&rtv.0, &clear_color);
            }
        }

        let copy_width = target_size.0.min(source_size.0);
        let copy_height = target_size.1.min(source_size.1);

        if copy_width > 0 && copy_height > 0 {
            let source_box = D3D11_BOX { left: 0, top: 0, front: 0, right: copy_width, bottom: copy_height, back: 1 };
            unsafe {
                context.CopySubresourceRegion(&cache.texture.0, 0, 0, 0, 0, source, 0, Some(&source_box));
            }
        }

        unsafe {
            context.Flush();
        }

        Ok(cache)
    }

    fn copy_frame_surface(&self, frame: &Frame) -> Result<SendDirectX<IDirect3DSurface>, VideoEncoderError> {
        // The capture pool may reuse its texture immediately after the callback returns.
        Ok(Self::copy_surface(
            frame.device(),
            frame.device_context(),
            frame.as_raw_texture(),
            (frame.width(), frame.height()),
            (self.target_width, self.target_height),
            frame.color_format(),
        )?
        .surface)
    }

    fn copy_surface_region(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        source: &ID3D11Texture2D,
        source_size: (u32, u32),
        area: (u32, u32, u32, u32),
        format: ColorFormat,
    ) -> Result<OwnedSurface, VideoEncoderError> {
        let (x, y, width, height) = area;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { source.GetDesc(&mut desc) };
        if width == 0
            || height == 0
            || x.checked_add(width).is_none_or(|right| right > source_size.0.min(desc.Width))
            || y.checked_add(height).is_none_or(|bottom| bottom > source_size.1.min(desc.Height))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Capture region exceeds the source texture.",
            )
            .into());
        }
        let owned = Self::create_owned_surface(device, width, height, format)?;
        let area = D3D11_BOX { left: x, top: y, front: 0, right: x + width, bottom: y + height, back: 1 };
        unsafe {
            context.CopySubresourceRegion(&owned.texture.0, 0, 0, 0, 0, source, 0, Some(&area));
            context.Flush();
        }
        Ok(owned)
    }

    /// Copies only the selected source pixels into an owned, encoder-sized GPU surface.
    pub fn send_frame_region(
        &mut self,
        frame: &Frame,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<(), VideoEncoderError> {
        if self.is_video_disabled {
            return Err(VideoEncoderError::VideoDisabled);
        }
        if (width, height) != (self.target_width, self.target_height) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Capture region must match encoder dimensions.",
            )
            .into());
        }
        let owned = Self::copy_surface_region(
            frame.device(),
            frame.device_context(),
            frame.as_raw_texture(),
            (frame.width(), frame.height()),
            (x, y, width, height),
            frame.color_format(),
        )?;
        let captured = frame.timestamp()?;
        let origin = *self.first_timestamp.get_or_insert(captured);
        let timestamp = TimeSpan { Duration: captured.Duration - origin.Duration };
        send_encoder_payload(
            &self.frame_sender,
            Some((VideoEncoderSource::DirectX(owned.surface), timestamp)),
            &mut self.transcode_thread,
        )
    }

    /// Sends a video frame (DirectX), waiting when two payloads are already queued.
    #[inline]
    pub fn send_frame(&mut self, frame: &Frame) -> Result<(), VideoEncoderError> {
        if self.is_video_disabled {
            return Err(VideoEncoderError::VideoDisabled);
        }

        let timestamp = match self.first_timestamp {
            Some(t0) => TimeSpan { Duration: frame.timestamp()?.Duration - t0.Duration },
            None => {
                let ts = frame.timestamp()?;
                self.first_timestamp = Some(ts);
                TimeSpan { Duration: 0 }
            }
        };

        let surface = self.copy_frame_surface(frame)?;

        send_encoder_payload(
            &self.frame_sender,
            Some((VideoEncoderSource::DirectX(surface), timestamp)),
            &mut self.transcode_thread,
        )
    }

    /// Sends owned video and audio payloads, waiting for capacity in each queue.
    /// Audio timestamp is derived from total samples sent so far (monotonic).
    #[inline]
    pub fn send_frame_with_audio(&mut self, frame: &mut Frame, audio_buffer: &[u8]) -> Result<(), VideoEncoderError> {
        if self.is_video_disabled {
            return Err(VideoEncoderError::VideoDisabled);
        }
        if self.is_audio_disabled {
            return Err(VideoEncoderError::AudioDisabled);
        }

        // Video timestamp based on capture timestamps (as before)
        let video_ts = match self.first_timestamp {
            Some(t0) => TimeSpan { Duration: frame.timestamp()?.Duration - t0.Duration },
            None => {
                let ts = frame.timestamp()?;
                self.first_timestamp = Some(ts);
                TimeSpan { Duration: 0 }
            }
        };

        let surface = self.copy_frame_surface(frame)?;

        send_encoder_payload(
            &self.frame_sender,
            Some((VideoEncoderSource::DirectX(surface), video_ts)),
            &mut self.transcode_thread,
        )?;

        // Audio timestamp from running sample count
        let frames_in_buf = (audio_buffer.len() as u32) / self.audio_block_align;
        let audio_ts_ticks = ((self.audio_samples_sent as i128) * 10_000_000i128) / (self.audio_sample_rate as i128);
        let audio_ts = TimeSpan { Duration: audio_ts_ticks as i64 };

        send_encoder_payload(
            &self.audio_sender,
            Some((AudioEncoderSource::Buffer(audio_buffer.to_vec()), audio_ts)),
            &mut self.transcode_thread,
        )?;

        // Advance counter after stamping
        self.audio_samples_sent = self.audio_samples_sent.saturating_add(frames_in_buf as u64);

        Ok(())
    }

    /// Sends a raw frame buffer, waiting when two payloads are already queued.
    /// Windows expects BGRA and bottom-to-top layout for this path.
    #[inline]
    pub fn send_frame_buffer(&mut self, buffer: &[u8], timestamp: i64) -> Result<(), VideoEncoderError> {
        if self.is_video_disabled {
            return Err(VideoEncoderError::VideoDisabled);
        }

        let frame_timestamp = timestamp;
        let timestamp = match self.first_timestamp {
            Some(t0) => TimeSpan { Duration: frame_timestamp - t0.Duration },
            None => {
                self.first_timestamp = Some(TimeSpan { Duration: frame_timestamp });
                TimeSpan { Duration: 0 }
            }
        };

        send_encoder_payload(
            &self.frame_sender,
            Some((VideoEncoderSource::Buffer(buffer.to_vec()), timestamp)),
            &mut self.transcode_thread,
        )
    }

    /// Sends an audio buffer, waiting when two payloads are already queued.
    /// NOTE: The provided `timestamp` is ignored; we use a monotonic audio clock.
    #[inline]
    pub fn send_audio_buffer(
        &mut self,
        buffer: &[u8],
        _timestamp: i64, // ignored to guarantee monotonic audio timing
    ) -> Result<(), VideoEncoderError> {
        if self.is_audio_disabled {
            return Err(VideoEncoderError::AudioDisabled);
        }

        let frames_in_buf = (buffer.len() as u32) / self.audio_block_align;
        let audio_ts_ticks = ((self.audio_samples_sent as i128) * 10_000_000i128) / (self.audio_sample_rate as i128);
        let timestamp = TimeSpan { Duration: audio_ts_ticks as i64 };

        send_encoder_payload(
            &self.audio_sender,
            Some((AudioEncoderSource::Buffer(buffer.to_vec()), timestamp)),
            &mut self.transcode_thread,
        )?;

        self.audio_samples_sent = self.audio_samples_sent.saturating_add(frames_in_buf as u64);

        Ok(())
    }

    /// Finishes the encoding and performs any necessary cleanup.
    #[inline]
    pub fn finish(mut self) -> Result<(), VideoEncoderError> {
        close_encoder_channel(&mut self.frame_sender);
        close_encoder_channel(&mut self.audio_sender);

        if let Some(transcode_thread) = self.transcode_thread.take() {
            join_transcoder(transcode_thread)?;
        }

        self.media_stream_source.RemoveStarting(self.starting)?;
        self.media_stream_source.RemoveSampleRequested(self.sample_requested)?;

        Ok(())
    }
}

impl Drop for VideoEncoder {
    #[inline]
    fn drop(&mut self) {
        close_encoder_channel(&mut self.frame_sender);
        close_encoder_channel(&mut self.audio_sender);

        if let Some(transcode_thread) = self.transcode_thread.take() {
            let _ = join_transcoder(transcode_thread);
        }

        let _ = self.media_stream_source.RemoveStarting(self.starting);
        let _ = self.media_stream_source.RemoveSampleRequested(self.sample_requested);
    }
}

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for VideoEncoder {}

#[cfg(test)]
mod owned_surface_tests {
    use super::*;

    #[test]
    #[ignore = "Synthetic WARP copy timing only; no capture item or encoder is opened."]
    fn benchmark_full_and_selected_gpu_copy() {
        use windows::Win32::{
            Foundation::HMODULE,
            Graphics::{
                Direct3D::D3D_DRIVER_TYPE_WARP,
                Direct3D11::{
                    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
                    D3D11_SDK_VERSION, D3D11_USAGE_STAGING, D3D11CreateDevice,
                },
            },
        };
        let _winrt = crate::winrt::WinRT::new().unwrap();
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .unwrap()
        };
        let (device, context) = (device.unwrap(), context.unwrap());
        let source = VideoEncoder::create_owned_surface(&device, 1920, 1080, ColorFormat::Bgra8).unwrap();
        unsafe { context.ClearRenderTargetView(&source.render_target_view.as_ref().unwrap().0, &[0.0, 0.0, 1.0, 1.0]) };
        for pass in 0..3 {
            for selected in [false, true] {
                let (width, height) = if selected { (640, 360) } else { (1920, 1080) };
                let started = std::time::Instant::now();
                for _ in 0..60 {
                    let owned = if selected {
                        VideoEncoder::copy_surface_region(
                            &device,
                            &context,
                            &source.texture.0,
                            (1920, 1080),
                            (101, 57, width, height),
                            ColorFormat::Bgra8,
                        )
                    } else {
                        VideoEncoder::copy_surface(
                            &device,
                            &context,
                            &source.texture.0,
                            (1920, 1080),
                            (width, height),
                            ColorFormat::Bgra8,
                        )
                    }
                    .unwrap();
                    let mut desc = D3D11_TEXTURE2D_DESC::default();
                    unsafe { owned.texture.0.GetDesc(&mut desc) };
                    desc.Usage = D3D11_USAGE_STAGING;
                    desc.BindFlags = 0;
                    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                    desc.MiscFlags = 0;
                    let mut staging = None;
                    unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)).unwrap() };
                    let staging = staging.unwrap();
                    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                    unsafe {
                        context.CopyResource(&staging, &owned.texture.0);
                        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).unwrap();
                        assert_eq!(std::slice::from_raw_parts(mapped.pData.cast::<u8>(), 4), &[255, 0, 0, 255]);
                        context.Unmap(&staging, 0);
                    }
                }
                eprintln!(
                    "WARP pass={pass} selected={selected} {width}x{height} frames=60 copy+staging_ms={:.3}",
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
    }

    #[test]
    fn selected_gpu_copies_exclude_outside_pixels_and_survive_source_reuse() {
        use windows::Win32::{
            Foundation::HMODULE,
            Graphics::{
                Direct3D::D3D_DRIVER_TYPE_WARP,
                Direct3D11::{
                    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
                    D3D11_SDK_VERSION, D3D11_USAGE_STAGING, D3D11CreateDevice,
                },
            },
        };
        let _winrt = crate::winrt::WinRT::new().unwrap();
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .unwrap()
        };
        let (device, context) = (device.unwrap(), context.unwrap());
        let source = VideoEncoder::create_owned_surface(&device, 8, 6, ColorFormat::Bgra8).unwrap();
        let mut pixels = [255, 0, 255, 255].repeat(8 * 6);
        for y in 1..5 {
            for x in 3..7 {
                let offset = (y * 8 + x) * 4;
                pixels[offset..offset + 4].copy_from_slice(&[x as u8, y as u8, 200, 255]);
            }
        }
        unsafe { context.UpdateSubresource(&source.texture.0, 0, None, pixels.as_ptr().cast(), 8 * 4, 0) };
        let first = VideoEncoder::copy_surface_region(
            &device,
            &context,
            &source.texture.0,
            (8, 6),
            (3, 1, 4, 4),
            ColorFormat::Bgra8,
        )
        .unwrap();
        unsafe { context.ClearRenderTargetView(&source.render_target_view.as_ref().unwrap().0, &[0.0, 1.0, 0.0, 1.0]) };
        let second = VideoEncoder::copy_surface_region(
            &device,
            &context,
            &source.texture.0,
            (8, 6),
            (3, 1, 4, 4),
            ColorFormat::Bgra8,
        )
        .unwrap();
        let (sender, receiver) = encoder_channel();
        sender.send(first).unwrap();
        sender.send(second).unwrap();
        for index in 0..2 {
            let owned = receiver.recv().unwrap();
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { owned.texture.0.GetDesc(&mut desc) };
            assert_eq!((desc.Width, desc.Height), (4, 4));
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)).unwrap() };
            let staging = staging.unwrap();
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe {
                context.CopyResource(&staging, &owned.texture.0);
                context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).unwrap();
            }
            for y in 0..4 {
                for x in 0..4 {
                    let actual = unsafe {
                        std::slice::from_raw_parts(
                            (mapped.pData as *const u8).add(y * mapped.RowPitch as usize + x * 4),
                            4,
                        )
                    };
                    let expected = if index == 0 { [x as u8 + 3, y as u8 + 1, 200, 255] } else { [0, 255, 0, 255] };
                    assert_eq!(actual, expected, "queued crop {index}, pixel {x},{y}");
                }
            }
            unsafe { context.Unmap(&staging, 0) };
        }
        for area in [(7, 1, 2, 2), (3, 5, 2, 2), (0, 0, 0, 2), (u32::MAX, 0, 2, 2), (0, u32::MAX, 2, 2)] {
            assert!(
                VideoEncoder::copy_surface_region(
                    &device,
                    &context,
                    &source.texture.0,
                    (100, 100),
                    area,
                    ColorFormat::Bgra8
                )
                .is_err()
            );
        }
        assert!(
            VideoEncoder::copy_surface_region(
                &device,
                &context,
                &source.texture.0,
                (6, 6),
                (3, 1, 4, 4),
                ColorFormat::Bgra8
            )
            .is_err()
        );
    }
    #[test]
    fn queued_gpu_copies_preserve_source_pixels_and_resize_padding() {
        use windows::Win32::{
            Foundation::HMODULE,
            Graphics::{
                Direct3D::D3D_DRIVER_TYPE_WARP,
                Direct3D11::{
                    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
                    D3D11_SDK_VERSION, D3D11_USAGE_STAGING, D3D11CreateDevice,
                },
            },
        };
        let _winrt = crate::winrt::WinRT::new().unwrap();
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .unwrap();
        }
        let device = device.unwrap();
        let context = context.unwrap();
        let source = VideoEncoder::create_owned_surface(&device, 4, 3, ColorFormat::Bgra8).unwrap();
        for (width, height) in [(4, 3), (6, 5), (2, 2)] {
            unsafe {
                context.ClearRenderTargetView(&source.render_target_view.as_ref().unwrap().0, &[0.0, 0.0, 1.0, 1.0]);
            }
            let first = VideoEncoder::copy_surface(
                &device,
                &context,
                &source.texture.0,
                (4, 3),
                (width, height),
                ColorFormat::Bgra8,
            )
            .unwrap();
            unsafe {
                context.ClearRenderTargetView(&source.render_target_view.as_ref().unwrap().0, &[1.0, 0.0, 0.0, 1.0]);
            }
            let second = VideoEncoder::copy_surface(
                &device,
                &context,
                &source.texture.0,
                (4, 3),
                (width, height),
                ColorFormat::Bgra8,
            )
            .unwrap();
            let (sender, receiver) = encoder_channel();
            sender.send(first.surface).unwrap();
            sender.send(second.surface).unwrap();
            for (index, copy) in [receiver.recv().unwrap(), receiver.recv().unwrap()].into_iter().enumerate() {
                use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
                let access = copy.0.cast::<IDirect3DDxgiInterfaceAccess>().unwrap();
                let texture = unsafe { access.GetInterface::<ID3D11Texture2D>().unwrap() };
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                unsafe {
                    texture.GetDesc(&mut desc);
                }
                desc.Usage = D3D11_USAGE_STAGING;
                desc.BindFlags = 0;
                desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                desc.MiscFlags = 0;
                let mut staging = None;
                unsafe {
                    device.CreateTexture2D(&desc, None, Some(&mut staging)).unwrap();
                }
                let staging = staging.unwrap();
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                unsafe {
                    context.CopyResource(&staging, &texture);
                    context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).unwrap();
                }
                for y in 0..height {
                    for x in 0..width {
                        let actual = unsafe {
                            std::slice::from_raw_parts(
                                (mapped.pData as *const u8).add((y * mapped.RowPitch + x * 4) as usize),
                                4,
                            )
                        };
                        let expected = if x >= 4 || y >= 3 {
                            [0, 0, 0, 255]
                        } else if index == 0 {
                            [255, 0, 0, 255]
                        } else {
                            [0, 0, 255, 255]
                        };
                        assert_eq!(actual, expected, "copy {index}, {width}x{height}, {x},{y}");
                    }
                }
                unsafe {
                    context.Unmap(&staging, 0);
                }
            }
        }
    }
}
