use crate::process;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug)]
pub(crate) enum RecoveryError {
    Unusable(String),
    Retry(String),
}

impl From<String> for RecoveryError {
    fn from(message: String) -> Self {
        Self::Retry(message)
    }
}

impl From<&str> for RecoveryError {
    fn from(message: &str) -> Self {
        Self::Retry(message.to_owned())
    }
}

impl From<RecoveryError> for String {
    fn from(error: RecoveryError) -> Self {
        match error {
            RecoveryError::Unusable(message) | RecoveryError::Retry(message) => message,
        }
    }
}

impl RecoveryError {
    pub(crate) fn input_io(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::NotFound {
            Self::Unusable(error.to_string())
        } else {
            Self::Retry(error.to_string())
        }
    }
}

pub(crate) fn usable<T>(result: Result<T, RecoveryError>) -> Result<Option<T>, RecoveryError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(RecoveryError::Unusable(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Debug)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub start: f64,
}

pub fn probe_video(ffmpeg: &Path, path: &Path) -> Result<VideoInfo, RecoveryError> {
    let input = fs::File::open(path).map_err(RecoveryError::input_io)?;
    probe_open_video(ffmpeg, path, &input)
}

fn probe_open_video(
    ffmpeg: &Path,
    path: &Path,
    input: &fs::File,
) -> Result<VideoInfo, RecoveryError> {
    let metadata = input.metadata().map_err(RecoveryError::input_io)?;
    if !metadata.is_file() || metadata.len() < 128 {
        return Err(RecoveryError::Unusable(
            "Video file is empty or truncated.".into(),
        ));
    }
    let bytes = process::read_output(
        &ffmpeg.with_file_name("ffprobe.exe"),
        &[
            "-v".into(),
            "error".into(),
            "-select_streams".into(),
            "v:0".into(),
            "-show_entries".into(),
            "stream=width,height,duration,start_time:format=duration".into(),
            "-of".into(),
            "json".into(),
            path.to_string_lossy().into_owned(),
        ],
        Duration::from_secs(20),
    )?;
    let parsed: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let stream = parsed["streams"]
        .as_array()
        .and_then(|streams| streams.first())
        .ok_or_else(|| RecoveryError::Unusable("Video stream is missing.".into()))?;
    let number = |value: &serde_json::Value| {
        value
            .as_str()
            .and_then(|text| text.parse::<f64>().ok())
            .or_else(|| value.as_f64())
    };
    let info = VideoInfo {
        width: stream["width"]
            .as_u64()
            .and_then(|value| value.try_into().ok())
            .ok_or_else(|| RecoveryError::Unusable("Video width is missing.".into()))?,
        height: stream["height"]
            .as_u64()
            .and_then(|value| value.try_into().ok())
            .ok_or_else(|| RecoveryError::Unusable("Video height is missing.".into()))?,
        duration: number(&stream["duration"])
            .or_else(|| number(&parsed["format"]["duration"]))
            .ok_or_else(|| RecoveryError::Unusable("Video duration is missing.".into()))?,
        start: number(&stream["start_time"]).unwrap_or(0.0),
    };
    if !(2..=16384).contains(&info.width)
        || !(2..=16384).contains(&info.height)
        || !info.duration.is_finite()
        || info.duration <= 0.0
        || !info.start.is_finite()
    {
        return Err(RecoveryError::Unusable(
            "Video contains invalid timing or dimensions.".into(),
        ));
    }
    Ok(info)
}

pub fn validate_video(ffmpeg: &Path, path: &Path, log: &Path) -> Result<VideoInfo, RecoveryError> {
    let input = fs::File::open(path).map_err(RecoveryError::input_io)?;
    let info = probe_open_video(ffmpeg, path, &input)?;
    process::run_logged_classified(
        ffmpeg,
        &[
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-xerror".into(),
            "-i".into(),
            path.to_string_lossy().into_owned(),
            "-map".into(),
            "0:v:0".into(),
            "-an".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ],
        log,
    )?;
    Ok(info)
}

pub fn preserve_invalid(path: &Path) -> Result<PathBuf, String> {
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or("Invalid media filename.")?;
    for index in 0..1000 {
        let preserved = path.with_file_name(format!("{stem}.invalid-{index:04}.mp4"));
        if !preserved.exists() {
            fs::rename(path, &preserved).map_err(|error| error.to_string())?;
            return Ok(preserved);
        }
    }
    Err("Too many preserved invalid media files in this project.".into())
}
