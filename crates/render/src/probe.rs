use crate::process;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
    time::Duration,
};

#[derive(Debug, Clone, Deserialize)]
pub struct MediaInfo {
    pub streams: Vec<MediaStream>,
    pub format: Option<MediaFormat>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct MediaStream {
    pub codec_type: Option<String>,
    pub codec_name: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sample_rate: Option<String>,
    pub channels: Option<u32>,
    pub duration: Option<String>,
    pub start_time: Option<String>,
    pub time_base: Option<String>,
    pub pix_fmt: Option<String>,
    pub profile: Option<String>,
    pub level: Option<i32>,
    pub extradata_hash: Option<String>,
    pub avg_frame_rate: Option<String>,
    pub nb_frames: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct MediaFormat {
    pub duration: Option<String>,
}

impl MediaInfo {
    pub fn duration(&self) -> Option<f64> {
        self.format
            .as_ref()
            .and_then(|format| format.duration.as_deref())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
    }
    pub fn video(&self) -> Option<&MediaStream> {
        self.streams
            .iter()
            .find(|stream| stream.codec_type.as_deref() == Some("video"))
    }
    pub fn has_audio(&self) -> bool {
        self.streams
            .iter()
            .any(|stream| stream.codec_type.as_deref() == Some("audio"))
    }
}

pub fn ffprobe_path(ffmpeg: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    let sibling = ffmpeg.with_file_name(name);
    if sibling.is_file() {
        sibling
    } else {
        PathBuf::from(name)
    }
}

pub fn probe_media(ffmpeg: &Path, media: &Path) -> Result<MediaInfo, String> {
    probe_with_cancel(ffmpeg, media, &AtomicBool::new(false))
}

pub(crate) fn probe_with_cancel(
    ffmpeg: &Path,
    media: &Path,
    cancel: &AtomicBool,
) -> Result<MediaInfo, String> {
    if !media.is_file() {
        return Err(format!("Media file is missing: {}", media.display()));
    }
    let mut command = Command::new(ffprobe_path(ffmpeg));
    command.args(["-v", "error", "-protocol_whitelist", "file,pipe", "-show_data_hash", "sha256", "-show_entries", "stream=codec_type,codec_name,width,height,sample_rate,channels,duration,start_time,time_base,pix_fmt,profile,level,extradata_hash,avg_frame_rate,nb_frames:format=duration", "-of", "json"])
        .arg(process::media_path(media));
    let (success, output, error) = process::capture(&mut command, Duration::from_secs(20), cancel)?;
    if !success {
        return Err(format!("Cannot inspect media: {}", error.trim()));
    }
    serde_json::from_slice(&output)
        .map_err(|error| format!("Cannot read media information: {error}"))
}
