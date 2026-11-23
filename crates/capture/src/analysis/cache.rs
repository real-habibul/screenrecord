use super::{recommended_gain, sample_at, AudioAnalysis, AudioAnalysisInput, WaveformBin};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        FileBasicInfo, FileIdInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
    },
};

const BLOCK_FRAMES: u64 = 4096;
const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIRECTORY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;
const MAGIC: &[u8; 8] = b"SRAUDIO1";

#[cfg(test)]
#[derive(Clone, Copy, Default, Serialize)]
struct ReadCounts {
    source_bytes: u64,
    cache_bytes: u64,
}
#[cfg(test)]
thread_local! { static READ_COUNTS: std::cell::Cell<ReadCounts> = const { std::cell::Cell::new(ReadCounts { source_bytes: 0, cache_bytes: 0 }) }; }
fn count_source_read(bytes: usize) {
    #[cfg(test)]
    READ_COUNTS.with(|counts| {
        let mut value = counts.get();
        value.source_bytes += bytes as u64;
        counts.set(value);
    });
    #[cfg(not(test))]
    let _ = bytes;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisCachePolicy {
    Reuse,
    Refresh,
}

/// Reuses source statistics, but always reads original PCM at retained-range boundaries.
pub fn analyze_cached(
    input: AudioAnalysisInput,
    cache_directory: &Path,
    policy: AnalysisCachePolicy,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(f64) + Send + 'static,
) -> Result<AudioAnalysis, String> {
    super::validate(&input)?;
    check_cancel(&cancel)?;
    let mut source = Source::open(&input.audio_path)?;
    let Some(format) = read_format(&mut source.file)? else {
        return super::analyze(input, cancel, progress);
    };
    let frames = format
        .frames
        .min(sample_at(input.source_duration, format.rate));
    if frames == 0 {
        return Err("The selected file contains no complete audio samples.".into());
    }
    let block_count = frames.div_ceil(BLOCK_FRAMES);
    if encoded_size(block_count, input.bins) > MAX_CACHE_BYTES {
        return super::analyze(input, cancel, progress);
    }
    let stamp = source.stamp().ok();
    let storage = CacheDirectory::open(cache_directory, &input.audio_path).ok();
    let header = Header {
        version: 1,
        stamp: stamp.clone(),
        format,
        frames,
        duration_bits: input.source_duration.to_bits(),
        bins: input.bins as u32,
        block_count,
    };
    progress(0.0);
    let mut hit = false;
    let index = if policy == AnalysisCachePolicy::Reuse && stamp.is_some() {
        storage
            .as_ref()
            .and_then(|storage| storage.load(&header, &cancel).ok())
    } else {
        None
    };
    check_cancel(&cancel)?;
    let index = match index {
        Some(index) => {
            hit = true;
            index
        }
        None => build(&mut source.file, &header, &cancel, &progress)?,
    };
    let result = query(&mut source.file, &header, &index, &input, &cancel)?;
    source.verify(stamp.as_ref())?;
    check_cancel(&cancel)?;
    if !hit && stamp.is_some() {
        if let Some(storage) = &storage {
            if let Err(error) = storage.save(&header, &index, &cancel) {
                check_cancel(&cancel)?;
                log::debug!("Audio waveform cache was not saved: {error}");
            }
        }
    }
    check_cancel(&cancel)?;
    source.verify(stamp.as_ref())?;
    progress(1.0);
    Ok(result)
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Audio analysis cancelled.".into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stamp {
    volume: u64,
    id: [u8; 16],
    length: u64,
    created: i64,
    written: i64,
    changed: i64,
    prefix: [u8; 32],
}

struct Source {
    file: File,
}
impl Source {
    fn open(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)
            .map_err(|error| {
                format!("Cannot read stable audio; close any program changing this file: {error}")
            })?;
        plain_file(&file)?;
        Ok(Self { file })
    }
    fn stamp(&mut self) -> Result<Stamp, String> {
        let mut basic = FILE_BASIC_INFO::default();
        let mut id = FILE_ID_INFO::default();
        unsafe {
            GetFileInformationByHandleEx(
                HANDLE(self.file.as_raw_handle()),
                FileBasicInfo,
                std::ptr::from_mut(&mut basic).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
            .map_err(|error| error.to_string())?;
            GetFileInformationByHandleEx(
                HANDLE(self.file.as_raw_handle()),
                FileIdInfo,
                std::ptr::from_mut(&mut id).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
            .map_err(|error| error.to_string())?;
        }
        let length = self.file.metadata().map_err(|e| e.to_string())?.len();
        let saved = self.file.stream_position().map_err(|e| e.to_string())?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        let mut prefix = vec![0; length.min(65_536) as usize];
        self.file
            .read_exact(&mut prefix)
            .map_err(|e| e.to_string())?;
        count_source_read(prefix.len());
        self.file
            .seek(SeekFrom::Start(saved))
            .map_err(|e| e.to_string())?;
        Ok(Stamp {
            volume: id.VolumeSerialNumber,
            id: id.FileId.Identifier,
            length,
            created: basic.CreationTime,
            written: basic.LastWriteTime,
            changed: basic.ChangeTime,
            prefix: Sha256::digest(prefix).into(),
        })
    }
    fn verify(&mut self, expected: Option<&Stamp>) -> Result<(), String> {
        if let Some(expected) = expected {
            if &self.stamp()? != expected {
                return Err("Audio changed during analysis. Analyze it again.".into());
            }
        }
        Ok(())
    }
}

fn plain_file(file: &File) -> Result<(), String> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err("Audio analysis does not follow file links or reparse points.".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Format {
    rate: u32,
    channels: u32,
    data: u64,
    frames: u64,
}
impl Format {
    fn frame_bytes(&self) -> u64 {
        u64::from(self.channels) * 4
    }
}

fn read_format(file: &mut File) -> Result<Option<Format>, String> {
    let invalid = || "The WAV file contains incomplete or invalid PCM data.".to_owned();
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let mut riff = [0; 12];
    if file.read_exact(&mut riff).is_err() {
        return Ok(None);
    }
    if (&riff[..4] != b"RIFF" && &riff[..4] != b"RF64") || &riff[8..] != b"WAVE" {
        return Ok(None);
    }
    let mut position = 12u64;
    let mut format = None;
    let mut rf64_length = None;
    for _ in 0..256 {
        if position.checked_add(8).is_none_or(|end| end > length) {
            return Err(invalid());
        }
        file.seek(SeekFrom::Start(position))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0; 8];
        file.read_exact(&mut chunk).map_err(|_| invalid())?;
        let size = u64::from(u32::from_le_bytes(chunk[4..].try_into().unwrap()));
        let start = position + 8;
        if &chunk[..4] == b"data" {
            let Some((rate, channels)) = format else {
                return Ok(None);
            };
            let size = if size == u64::from(u32::MAX) {
                rf64_length.ok_or_else(invalid)?
            } else {
                size
            };
            let frame_bytes = u64::from(channels) * 4;
            if size == 0 || size % frame_bytes != 0 || size > length.saturating_sub(start) {
                return Err(invalid());
            }
            return Ok(Some(Format {
                rate,
                channels,
                data: start,
                frames: size / frame_bytes,
            }));
        }
        if size > length.saturating_sub(start) {
            return Err(invalid());
        }
        if &chunk[..4] == b"ds64" && size >= 28 {
            let mut bytes = [0; 28];
            file.read_exact(&mut bytes).map_err(|_| invalid())?;
            rf64_length = Some(u64::from_le_bytes(bytes[8..16].try_into().unwrap()));
        }
        if &chunk[..4] == b"fmt " {
            if !(16..=4096).contains(&size) {
                return Ok(None);
            }
            let mut bytes = vec![0; size as usize];
            file.read_exact(&mut bytes).map_err(|_| invalid())?;
            let tag = u16::from_le_bytes(bytes[..2].try_into().unwrap());
            let float_guid = [3, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113];
            let float = tag == 3
                || (tag == 0xfffe
                    && bytes.len() >= 40
                    && bytes[18..20] == 32u16.to_le_bytes()
                    && bytes[24..40] == float_guid);
            if !float || bytes[14..16] != 32u16.to_le_bytes() {
                return Ok(None);
            }
            let channels = u32::from(u16::from_le_bytes(bytes[2..4].try_into().unwrap()));
            let rate = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
            if !(1..=8).contains(&channels)
                || !(1..=768_000).contains(&rate)
                || u32::from(u16::from_le_bytes(bytes[12..14].try_into().unwrap())) != channels * 4
            {
                return Err(invalid());
            }
            format = Some((rate, channels));
        }
        position = start
            .checked_add(size)
            .and_then(|p| p.checked_add(size % 2))
            .ok_or_else(invalid)?;
    }
    Err(invalid())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    stamp: Option<Stamp>,
    format: Format,
    frames: u64,
    duration_bits: u64,
    bins: u32,
    block_count: u64,
}

#[derive(Clone, Copy, Default)]
struct Block {
    min: f64,
    max: f64,
    squares: f64,
    frames: u64,
}
struct Index {
    bins: Vec<WaveformBin>,
    blocks: Vec<Block>,
}

#[derive(Default)]
struct Sum {
    value: f64,
    correction: f64,
}
impl Sum {
    fn add(&mut self, value: f64) {
        let corrected = value - self.correction;
        let next = self.value + corrected;
        self.correction = (next - self.value) - corrected;
        self.value = next;
    }
}

fn read_frames(
    file: &mut File,
    format: &Format,
    start: u64,
    count: u64,
    cancel: &AtomicBool,
    mut consume: impl FnMut(u64, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    file.seek(SeekFrom::Start(format.data + start * format.frame_bytes()))
        .map_err(|e| e.to_string())?;
    let mut buffer = [0; 65_536];
    let frame_bytes = format.frame_bytes() as usize;
    let capacity = buffer.len() / frame_bytes;
    let mut position = start;
    let end = start + count;
    while position < end {
        check_cancel(cancel)?;
        let frames = (end - position).min(capacity as u64) as usize;
        file.read_exact(&mut buffer[..frames * frame_bytes])
            .map_err(|_| "The audio file ended inside its PCM data.".to_owned())?;
        count_source_read(frames * frame_bytes);
        for frame in buffer[..frames * frame_bytes].chunks_exact(frame_bytes) {
            consume(position, frame)?;
            position += 1;
        }
    }
    Ok(())
}

fn build(
    file: &mut File,
    header: &Header,
    cancel: &AtomicBool,
    progress: &impl Fn(f64),
) -> Result<Index, String> {
    let mut bins = vec![WaveformBin::default(); header.bins as usize];
    let mut seen = vec![false; bins.len()];
    let mut blocks = Vec::with_capacity(header.block_count as usize);
    let mut block = Block::default();
    let mut squares = Sum::default();
    let duration = f64::from_bits(header.duration_bits);
    read_frames(
        file,
        &header.format,
        0,
        header.frames,
        cancel,
        |frame, bytes| {
            let bin = ((frame as f64 / f64::from(header.format.rate) / duration
                * f64::from(header.bins))
            .floor() as usize)
                .min(bins.len() - 1);
            for (channel, sample) in bytes.as_chunks::<4>().0.iter().enumerate() {
                let value = f64::from(f32::from_le_bytes(*sample));
                if !value.is_finite() {
                    return Err("Audio contains nonfinite samples.".into());
                }
                if !seen[bin] {
                    bins[bin] = WaveformBin {
                        min: value,
                        max: value,
                    };
                    seen[bin] = true;
                } else {
                    bins[bin].min = bins[bin].min.min(value);
                    bins[bin].max = bins[bin].max.max(value);
                }
                if block.frames == 0 && channel == 0 {
                    block.min = value;
                    block.max = value;
                } else {
                    block.min = block.min.min(value);
                    block.max = block.max.max(value);
                }
                squares.add(value * value);
            }
            block.frames += 1;
            if block.frames == BLOCK_FRAMES || frame + 1 == header.frames {
                block.squares = squares.value;
                blocks.push(block);
                block = Block::default();
                squares = Sum::default();
                if blocks.len().is_multiple_of(256) || frame + 1 == header.frames {
                    progress((frame + 1) as f64 / header.frames as f64 * 0.9);
                }
            }
            Ok(())
        },
    )?;
    Ok(Index { bins, blocks })
}

fn ranges(input: &AudioAnalysisInput, header: &Header) -> Vec<(u64, u64)> {
    let converted = if input.retained_ranges.is_empty() {
        vec![(0, header.frames)]
    } else {
        input
            .retained_ranges
            .iter()
            .map(|&(a, b)| {
                (
                    sample_at(a, header.format.rate).min(header.frames),
                    sample_at(b, header.format.rate).min(header.frames),
                )
            })
            .collect()
    };
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (start, end) in converted {
        if start >= end {
            continue;
        }
        if let Some(previous) = merged.last_mut().filter(|previous| start <= previous.1) {
            previous.1 = previous.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}

fn query(
    file: &mut File,
    header: &Header,
    index: &Index,
    input: &AudioAnalysisInput,
    cancel: &AtomicBool,
) -> Result<AudioAnalysis, String> {
    let mut peak = 0f64;
    let mut squares = Sum::default();
    let mut count = 0u64;
    for (mut start, end) in ranges(input, header) {
        while start < end {
            check_cancel(cancel)?;
            let block = index.blocks[(start / BLOCK_FRAMES) as usize];
            let block_start = start / BLOCK_FRAMES * BLOCK_FRAMES;
            let until = end.min(block_start + block.frames);
            if start == block_start && until == block_start + block.frames {
                peak = peak.max(block.min.abs()).max(block.max.abs());
                squares.add(block.squares);
            } else {
                read_frames(
                    file,
                    &header.format,
                    start,
                    until - start,
                    cancel,
                    |_, bytes| {
                        for sample in bytes.as_chunks::<4>().0 {
                            let value = f64::from(f32::from_le_bytes(*sample));
                            if !value.is_finite() {
                                return Err("Audio contains nonfinite samples.".into());
                            }
                            peak = peak.max(value.abs());
                            squares.add(value * value);
                        }
                        Ok(())
                    },
                )?;
            }
            count += until - start;
            start = until;
        }
    }
    let rms = if count == 0 {
        0.0
    } else {
        (squares.value / (count as f64 * f64::from(header.format.channels))).sqrt()
    };
    Ok(AudioAnalysis {
        source_duration: input.source_duration,
        bins: index.bins.clone(),
        peak,
        rms,
        analyzed_seconds: count as f64 / f64::from(header.format.rate),
        recommended_gain: recommended_gain(peak, rms),
    })
}

fn encoded_size(blocks: u64, bins: usize) -> u64 {
    4096 + blocks * 32 + bins as u64 * 16 + 32
}

struct CacheDirectory {
    path: PathBuf,
    key: String,
    _project: File,
    _directory: File,
}
impl CacheDirectory {
    fn open(path: &Path, source: &Path) -> Result<Self, String> {
        if !path.is_absolute()
            || path.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
            || path
                .file_name()
                .is_none_or(|name| name != ".audio-analysis")
        {
            return Err("Invalid audio cache directory.".into());
        }
        let parent = path
            .parent()
            .ok_or("Audio cache has no project directory.")?;
        let project = directory_handle(parent)?;
        let canonical_parent = parent.canonicalize().map_err(|e| e.to_string())?;
        let canonical_source = source.canonicalize().map_err(|e| e.to_string())?;
        let relative = canonical_source
            .strip_prefix(&canonical_parent)
            .map_err(|_| "Audio cache source is outside the project.")?;
        match fs::create_dir(path) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.to_string()),
        }
        let directory = directory_handle(path)?;
        if path.canonicalize().map_err(|e| e.to_string())?
            != canonical_parent.join(".audio-analysis")
        {
            return Err("Audio cache changed location.".into());
        }
        let key = format!(
            "waveform-{:x}.bin",
            Sha256::digest(relative.to_string_lossy().as_bytes())
        );
        Ok(Self {
            path: path.to_owned(),
            key,
            _project: project,
            _directory: directory,
        })
    }
    fn load(&self, expected: &Header, cancel: &AtomicBool) -> Result<Index, String> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(self.path.join(&self.key))
            .map_err(|e| e.to_string())?;
        plain_file(&file)?;
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        if !(44..=MAX_CACHE_BYTES).contains(&length) {
            return Err("Invalid audio cache size.".into());
        }
        let mut reader = BufReader::new(file);
        let mut hash = Sha256::new();
        let mut prefix = [0; 12];
        hashed_read(&mut reader, &mut hash, &mut prefix)?;
        if &prefix[..8] != MAGIC {
            return Err("Unsupported audio cache version.".into());
        }
        let header_len = u32::from_le_bytes(prefix[8..].try_into().unwrap()) as usize;
        if !(1..=4096).contains(&header_len) {
            return Err("Invalid audio cache header.".into());
        }
        let mut bytes = vec![0; header_len];
        hashed_read(&mut reader, &mut hash, &mut bytes)?;
        let header: Header =
            serde_json::from_slice(&bytes).map_err(|_| "Malformed audio cache header.")?;
        if &header != expected
            || length
                != 12
                    + header_len as u64
                    + u64::from(header.bins) * 16
                    + header.block_count * 32
                    + 32
        {
            return Err("Audio cache belongs to a different source revision.".into());
        }
        let mut bins = Vec::with_capacity(header.bins as usize);
        for _ in 0..header.bins {
            let mut data = [0; 16];
            hashed_read(&mut reader, &mut hash, &mut data)?;
            let min = f64::from_le_bytes(data[..8].try_into().unwrap());
            let max = f64::from_le_bytes(data[8..].try_into().unwrap());
            if !min.is_finite()
                || !max.is_finite()
                || min > max
                || min.abs() > f64::from(f32::MAX)
                || max.abs() > f64::from(f32::MAX)
            {
                return Err("Invalid waveform extrema.".into());
            }
            bins.push(WaveformBin { min, max });
        }
        let mut blocks = Vec::with_capacity(header.block_count as usize);
        for number in 0..header.block_count {
            if number.is_multiple_of(256) {
                check_cancel(cancel)?;
            }
            let mut data = [0; 32];
            hashed_read(&mut reader, &mut hash, &mut data)?;
            let block = Block {
                min: f64::from_le_bytes(data[..8].try_into().unwrap()),
                max: f64::from_le_bytes(data[8..16].try_into().unwrap()),
                squares: f64::from_le_bytes(data[16..24].try_into().unwrap()),
                frames: u64::from_le_bytes(data[24..].try_into().unwrap()),
            };
            let expected_frames = (header.frames - number * BLOCK_FRAMES).min(BLOCK_FRAMES);
            let peak = block.min.abs().max(block.max.abs());
            if block.frames != expected_frames
                || !block.min.is_finite()
                || !block.max.is_finite()
                || block.min > block.max
                || peak > f64::from(f32::MAX)
                || !block.squares.is_finite()
                || block.squares < peak * peak * (1.0 - 1e-12)
                || block.squares
                    > peak
                        * peak
                        * block.frames as f64
                        * f64::from(header.format.channels)
                        * (1.0 + 1e-12)
            {
                return Err("Invalid audio statistics block.".into());
            }
            blocks.push(block);
        }
        let mut checksum = [0; 32];
        reader
            .read_exact(&mut checksum)
            .map_err(|e| e.to_string())?;
        if hash.finalize().as_slice() != checksum {
            return Err("Audio cache checksum mismatch.".into());
        }
        Ok(Index { bins, blocks })
    }
    fn save(&self, header: &Header, index: &Index, cancel: &AtomicBool) -> Result<(), String> {
        check_cancel(cancel)?;
        let target = self.path.join(&self.key);
        if let Ok(metadata) = fs::symlink_metadata(&target) {
            if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                return Err("Audio cache target is not a regular file.".into());
            }
        }
        let mut count = 0;
        let mut bytes = 0u64;
        for (entry_number, entry) in fs::read_dir(&self.path)
            .map_err(|e| e.to_string())?
            .enumerate()
        {
            if entry_number >= MAX_ENTRIES {
                return Err("Audio cache directory entry budget reached.".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name() == self.key.as_str() {
                continue;
            }
            if entry.file_name().to_string_lossy().starts_with("waveform-") {
                let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
                if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                    return Err("Audio cache contains a nonregular entry.".into());
                }
                count += 1;
                bytes = bytes.saturating_add(metadata.len());
            }
            if count >= MAX_ENTRIES
                || bytes.saturating_add(encoded_size(header.block_count, header.bins as usize))
                    > MAX_DIRECTORY_BYTES
            {
                return Err("Audio cache storage budget reached.".into());
            }
        }
        let mut temporary =
            tempfile::NamedTempFile::new_in(&self.path).map_err(|e| e.to_string())?;
        {
            let mut writer = BufWriter::new(temporary.as_file_mut());
            let mut hash = Sha256::new();
            let header_bytes = serde_json::to_vec(header).map_err(|e| e.to_string())?;
            if header_bytes.len() > 4096 {
                return Err("Audio cache header exceeds its limit.".into());
            }
            hashed_write(&mut writer, &mut hash, MAGIC)?;
            hashed_write(
                &mut writer,
                &mut hash,
                &(header_bytes.len() as u32).to_le_bytes(),
            )?;
            hashed_write(&mut writer, &mut hash, &header_bytes)?;
            for bin in &index.bins {
                hashed_write(&mut writer, &mut hash, &bin.min.to_le_bytes())?;
                hashed_write(&mut writer, &mut hash, &bin.max.to_le_bytes())?;
            }
            for (number, block) in index.blocks.iter().enumerate() {
                if number.is_multiple_of(256) {
                    check_cancel(cancel)?;
                }
                hashed_write(&mut writer, &mut hash, &block.min.to_le_bytes())?;
                hashed_write(&mut writer, &mut hash, &block.max.to_le_bytes())?;
                hashed_write(&mut writer, &mut hash, &block.squares.to_le_bytes())?;
                hashed_write(&mut writer, &mut hash, &block.frames.to_le_bytes())?;
            }
            writer
                .write_all(&hash.finalize())
                .map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
        }
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        check_cancel(cancel)?;
        temporary.persist(target).map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn directory_handle(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err("Audio cache does not follow directory links or reparse points.".into());
    }
    Ok(file)
}
fn hashed_read(reader: &mut impl Read, hash: &mut Sha256, bytes: &mut [u8]) -> Result<(), String> {
    reader.read_exact(bytes).map_err(|e| e.to_string())?;
    #[cfg(test)]
    READ_COUNTS.with(|counts| {
        let mut value = counts.get();
        value.cache_bytes += bytes.len() as u64;
        counts.set(value);
    });
    hash.update(bytes);
    Ok(())
}

#[cfg(test)]
mod tests;
fn hashed_write(writer: &mut impl Write, hash: &mut Sha256, bytes: &[u8]) -> Result<(), String> {
    writer.write_all(bytes).map_err(|e| e.to_string())?;
    hash.update(bytes);
    Ok(())
}
