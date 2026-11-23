use crate::{
    cancelled, check_cancel, provider::MAX_AUDIO_BYTES, Chunk, TranscriptionInput, CANCELLED,
};
use std::{path::Path, process::Stdio, sync::atomic::AtomicBool, time::Duration};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    },
};

pub(crate) struct Job(usize);

impl Job {
    pub(crate) fn attach(child: &Child) -> Result<Self, String> {
        let raw = child
            .raw_handle()
            .ok_or("Audio extraction process has already exited.")?;
        unsafe {
            let handle = CreateJobObjectW(None, None)
                .map_err(|_| "Cannot secure the audio extraction process.".to_owned())?;
            let job = Self(handle.0 as usize);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
            .and_then(|_| AssignProcessToJobObject(handle, HANDLE(raw)))
            .map_err(|_| "Cannot secure the audio extraction process.".to_owned())?;
            Ok(job)
        }
    }
    pub(crate) fn terminate(&self) {
        unsafe {
            let _ = TerminateJobObject(HANDLE(self.0 as *mut _), 1);
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(HANDLE(self.0 as *mut _));
        }
    }
}

pub(crate) async fn extract(
    input: &TranscriptionInput,
    chunk: &Chunk,
    sample_rate: u32,
    output: &Path,
    cancel: &AtomicBool,
) -> Result<f64, String> {
    check_cancel(cancel)?;
    let mut command = Command::new(&input.ffmpeg_path);
    command
        .creation_flags(0x0800_0000)
        .kill_on_drop(true)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-n",
            "-copyts",
            "-protocol_whitelist",
            "file,pipe",
            "-ss",
        ])
        .arg(format!("{:.0}", chunk.start.floor()))
        .arg("-noaccurate_seek")
        .arg("-i")
        .arg(&input.audio_path)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-t"])
        .arg(format!("{:.9}", chunk.end - chunk.start))
        .arg("-af")
        .arg(format!(
            "asettb=1/{sample_rate},atrim=start_pts={}:end_pts={},asetpts=PTS-STARTPTS",
            sample_boundary(chunk.start, sample_rate),
            sample_boundary(chunk.end, sample_rate)
        ))
        .args(["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", "-f", "wav"])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    run(&mut command, cancel, Duration::from_secs(180)).await?;
    let length = tokio::fs::metadata(output)
        .await
        .map_err(|_| "Audio extraction did not create a usable file.".to_owned())?
        .len();
    if !(45..=MAX_AUDIO_BYTES).contains(&length) {
        return Err("Extracted subtitle audio is empty or exceeds 20 MiB.".into());
    }
    let mut header = Vec::new();
    tokio::fs::File::open(output)
        .await
        .map_err(|_| "Cannot inspect prepared audio.".to_owned())?
        .take(4096)
        .read_to_end(&mut header)
        .await
        .map_err(|_| "Cannot inspect prepared audio.".to_owned())?;
    pcm_duration(&header, length)
}

pub(crate) fn sample_boundary(seconds: f64, sample_rate: u32) -> u64 {
    let rate = f64::from(sample_rate);
    let sample = (seconds * rate).floor() as u64;
    sample + u64::from((sample as f64 / rate) < seconds)
}

pub(crate) async fn sample_rate(
    input: &TranscriptionInput,
    directory: &Path,
    cancel: &AtomicBool,
) -> Result<u32, String> {
    check_cancel(cancel)?;
    let output = directory.join("sample-rate.txt");
    let file = std::fs::File::create(&output)
        .map_err(|_| "Cannot inspect the recording audio format.".to_owned())?;
    let mut command = Command::new(input.ffmpeg_path.with_file_name("ffprobe.exe"));
    command
        .creation_flags(0x0800_0000)
        .kill_on_drop(true)
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=sample_rate",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(&input.audio_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .stderr(Stdio::null());
    run(&mut command, cancel, Duration::from_secs(30)).await?;
    let mut bytes = Vec::new();
    tokio::fs::File::open(&output)
        .await
        .map_err(|_| "Cannot inspect the recording audio format.".to_owned())?
        .take(65)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Cannot inspect the recording audio format.".to_owned())?;
    tokio::fs::remove_file(output)
        .await
        .map_err(|_| "Cannot remove temporary audio metadata.".to_owned())?;
    let rate = std::str::from_utf8(&bytes)
        .ok()
        .filter(|_| bytes.len() <= 64)
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|rate| (1..=768_000).contains(rate))
        .ok_or("The recording audio has no supported sample rate.")?;
    check_cancel(cancel)?;
    Ok(rate)
}

pub(crate) fn pcm_duration(bytes: &[u8], file_length: u64) -> Result<f64, String> {
    let invalid = || "The selected range contains no complete PCM audio.".to_owned();
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(invalid());
    }
    let mut at = 12;
    let mut format_ok = false;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let data = at + 8;
        if &bytes[at..at + 4] == b"data" {
            if !format_ok
                || size == 0
                || !size.is_multiple_of(2)
                || (data as u64).saturating_add(size as u64) > file_length
            {
                return Err(invalid());
            }
            return Ok(size as f64 / 32_000.0);
        }
        if data.saturating_add(size) > bytes.len() {
            return Err(invalid());
        }
        if &bytes[at..at + 4] == b"fmt " && size >= 16 {
            format_ok = bytes[data..data + 2] == 1u16.to_le_bytes()
                && bytes[data + 2..data + 4] == 1u16.to_le_bytes()
                && bytes[data + 4..data + 8] == 16_000u32.to_le_bytes()
                && bytes[data + 12..data + 14] == 2u16.to_le_bytes()
                && bytes[data + 14..data + 16] == 16u16.to_le_bytes();
        }
        at = data.saturating_add(size).saturating_add(size % 2);
    }
    Err(invalid())
}

pub(crate) async fn run(
    command: &mut Command,
    cancel: &AtomicBool,
    deadline: Duration,
) -> Result<(), String> {
    let mut child = command
        .spawn()
        .map_err(|_| "Cannot start local subtitle audio extraction.".to_owned())?;
    let job = match Job::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            if let Ok(Some(status)) = child.try_wait() {
                return if status.success() {
                    Ok(())
                } else {
                    Err("The selected recording has no readable audio stream.".into())
                };
            }
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            return Err(error);
        }
    };
    let result = tokio::select! {
        biased;
        _ = cancelled(cancel) => Err(CANCELLED.to_owned()),
        _ = tokio::time::sleep(deadline) => Err("Local subtitle audio extraction exceeded its deadline.".to_owned()),
        status = child.wait() => match status {
            Ok(status) if status.success() => Ok(()),
            _ => Err("The selected recording has no readable audio stream or audio extraction failed.".into()),
        },
    };
    if result.is_err() {
        job.terminate();
        let _ = child.start_kill();
        if !matches!(
            tokio::time::timeout(Duration::from_secs(2), child.wait()).await,
            Ok(Ok(_))
        ) {
            return Err("Audio extraction is still closing after termination. Try again after restarting the application.".into());
        }
    }
    result
}

#[cfg(test)]
pub(crate) async fn run_fixture(command: &mut Command, cancel: &AtomicBool) -> Result<(), String> {
    run(command, cancel, Duration::from_secs(3)).await
}
