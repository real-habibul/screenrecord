use std::{
    fs::File,
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};

const HEADER_LEN: usize = 92;

pub struct PcmWriter {
    file: BufWriter<File>,
    bytes: u64,
}

impl PcmWriter {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let mut file = BufWriter::with_capacity(64 * 1024, File::create(path)?);
        file.write_all(&header(0))?;
        Ok(Self { file, bytes: 0 })
    }

    pub fn write_frames(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if !bytes.len().is_multiple_of(8) {
            return Err(std::io::Error::other(
                "PCM packet ends inside a stereo frame.",
            ));
        }
        self.file.write_all(bytes)?;
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::other("Audio length exceeded the supported range."))?;
        Ok(())
    }

    pub fn truncate_frames(&mut self, frames: u64) -> std::io::Result<()> {
        let bytes = frames
            .checked_mul(8)
            .ok_or_else(|| std::io::Error::other("Audio length overflowed."))?;
        if bytes < self.bytes {
            self.file.flush()?;
            self.file.get_mut().set_len(HEADER_LEN as u64 + bytes)?;
            self.file.get_mut().seek(SeekFrom::End(0))?;
            self.bytes = bytes;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()?;
        let file = self.file.get_mut();
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header(self.bytes))?;
        file.seek(SeekFrom::End(0))?;
        file.sync_data()
    }

    pub fn finalize(mut self) -> std::io::Result<()> {
        self.flush()?;
        self.file.get_ref().sync_all()
    }
}

pub fn readable(path: &Path) -> std::io::Result<bool> {
    use std::io::ErrorKind;
    match inspect(path) {
        Ok(_) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::InvalidData | ErrorKind::UnexpectedEof
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn inspect(path: &Path) -> std::io::Result<u64> {
    let invalid = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid or incomplete captured WAV file.",
        )
    };
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut riff = [0u8; 12];
    file.read_exact(&mut riff)?;
    if (&riff[..4] != b"RIFF" && &riff[..4] != b"RF64") || &riff[8..12] != b"WAVE" {
        return Err(invalid());
    }
    let mut position = 12u64;
    let mut format = false;
    let mut rf64_length = None;
    for _ in 0..256 {
        if position + 8 > length {
            return Err(invalid());
        }
        file.seek(SeekFrom::Start(position))?;
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk)?;
        let size = u32::from_le_bytes(chunk[4..8].try_into().unwrap()) as u64;
        let start = position + 8;
        if &chunk[..4] == b"data" {
            let size = if size == u32::MAX as u64 {
                rf64_length.ok_or_else(invalid)?
            } else {
                size
            };
            if !format || size == 0 || size % 8 != 0 || size > length.saturating_sub(start) {
                return Err(invalid());
            }
            return Ok(size / 8);
        }
        if size > length.saturating_sub(start) {
            return Err(invalid());
        }
        if &chunk[..4] == b"ds64" && size >= 28 {
            let mut bytes = [0u8; 16];
            file.read_exact(&mut bytes)?;
            rf64_length = Some(u64::from_le_bytes(bytes[8..16].try_into().unwrap()));
        }
        if &chunk[..4] == b"fmt " && (16..=4096).contains(&size) {
            let mut bytes = vec![0u8; size as usize];
            file.read_exact(&mut bytes)?;
            let tag = u16::from_le_bytes(bytes[..2].try_into().unwrap());
            let float = tag == 3
                || (tag == 65534 && bytes.len() >= 40 && bytes[24..28] == 3u32.to_le_bytes());
            format = float
                && bytes[2..4] == 2u16.to_le_bytes()
                && bytes[4..8] == 48_000u32.to_le_bytes()
                && bytes[12..14] == 8u16.to_le_bytes()
                && bytes[14..16] == 32u16.to_le_bytes();
        }
        position = start
            .checked_add(size)
            .and_then(|value| value.checked_add(size % 2))
            .ok_or_else(invalid)?;
    }
    Err(invalid())
}

fn header(data_bytes: u64) -> [u8; HEADER_LEN] {
    let mut bytes = [0u8; HEADER_LEN];
    let file_size = data_bytes.saturating_add(HEADER_LEN as u64 - 8);
    let large = file_size > u32::MAX as u64;
    bytes[0..4].copy_from_slice(if large { b"RF64" } else { b"RIFF" });
    bytes[4..8].copy_from_slice(&(if large { u32::MAX } else { file_size as u32 }).to_le_bytes());
    bytes[8..12].copy_from_slice(b"WAVE");
    // Reserve ds64 space from the start, so crossing 4 GiB never moves existing PCM.
    bytes[12..16].copy_from_slice(if large { b"ds64" } else { b"JUNK" });
    bytes[16..20].copy_from_slice(&28u32.to_le_bytes());
    if large {
        bytes[20..28].copy_from_slice(&file_size.to_le_bytes());
        bytes[28..36].copy_from_slice(&data_bytes.to_le_bytes());
        bytes[36..44].copy_from_slice(&(data_bytes / 8).to_le_bytes());
    }
    bytes[48..52].copy_from_slice(b"fmt ");
    bytes[52..56].copy_from_slice(&16u32.to_le_bytes());
    bytes[56..58].copy_from_slice(&3u16.to_le_bytes());
    bytes[58..60].copy_from_slice(&2u16.to_le_bytes());
    bytes[60..64].copy_from_slice(&48_000u32.to_le_bytes());
    bytes[64..68].copy_from_slice(&384_000u32.to_le_bytes());
    bytes[68..70].copy_from_slice(&8u16.to_le_bytes());
    bytes[70..72].copy_from_slice(&32u16.to_le_bytes());
    bytes[72..76].copy_from_slice(b"fact");
    bytes[76..80].copy_from_slice(&4u32.to_le_bytes());
    bytes[80..84].copy_from_slice(&((data_bytes / 8).min(u32::MAX as u64) as u32).to_le_bytes());
    bytes[84..88].copy_from_slice(b"data");
    bytes[88..92]
        .copy_from_slice(&(if large { u32::MAX } else { data_bytes as u32 }).to_le_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    mod quota {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/quota.rs"
        ));
    }

    #[test]
    #[ignore = "requires the isolated 1 MiB volume created by scripts/test-disk-full.ps1"]
    fn disk_full_during_pcm_capture_keeps_checkpointed_samples() {
        let root = tempfile::tempdir_in(std::env::var_os("SCREENRECORD_TEST_QUOTA_ROOT").unwrap())
            .unwrap();
        let path = root.path().join("system.wav");
        let reserve = root.path().join("reserve");
        std::fs::write(&reserve, [0; 65536]).unwrap();
        let mut writer = PcmWriter::create(&path).unwrap();
        let frames_per_packet = 4096_u32;
        let mut packets = 0_u32;
        let error = loop {
            assert!(packets < 32, "Run only with the isolated 1 MiB quota");
            let sample = (packets as f32 + 1.0) / 32.0;
            let packet = sample.to_le_bytes().repeat(frames_per_packet as usize * 2);
            match writer.write_frames(&packet).and_then(|()| writer.flush()) {
                Ok(()) => packets += 1,
                Err(error) => break error,
            }
        };
        assert!(packets > 0);
        quota::assert_volume_full();
        assert!(
            matches!(error.raw_os_error(), Some(31 | 39 | 112)),
            "{error:?}"
        );
        let _ = writer.finalize();
        assert_eq!(
            inspect(&path).unwrap(),
            u64::from(packets * frames_per_packet)
        );
        let mut decoded = hound::WavReader::open(&path).unwrap();
        assert_eq!(decoded.duration(), packets * frames_per_packet);
        for (index, sample) in decoded.samples::<f32>().enumerate() {
            let packet = index as u32 / (frames_per_packet * 2);
            assert_eq!(sample.unwrap(), (packet as f32 + 1.0) / 32.0);
        }
        std::fs::remove_file(reserve).unwrap();
        let retry = root.path().join("next.wav");
        let mut next = PcmWriter::create(&retry).unwrap();
        next.write_frames(&[0; 32768]).unwrap();
        next.finalize().unwrap();
        assert_eq!(inspect(&retry).unwrap(), u64::from(frames_per_packet));
        eprintln!("Volume full during PCM capture after {packets} checkpointed packets. All samples decoded identically.");
    }

    #[test]
    fn recovery_rejects_empty_or_truncated_pcm_and_keeps_checkpointed_tail() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("track.wav");
        let mut writer = PcmWriter::create(&path).unwrap();
        writer.flush().unwrap();
        assert!(!readable(&path).unwrap());
        writer.write_frames(&[0u8; 800]).unwrap();
        writer.flush().unwrap();
        assert_eq!(inspect(&path).unwrap(), 100);
        writer.write_frames(&[0u8; 80]).unwrap();
        writer.file.flush().unwrap();
        assert_eq!(inspect(&path).unwrap(), 100);
        writer.truncate_frames(50).unwrap();
        writer.finalize().unwrap();
        assert_eq!(inspect(&path).unwrap(), 50);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(HEADER_LEN as u64 + 392)
            .unwrap();
        assert!(!readable(&path).unwrap());
    }

    #[test]
    fn ordinary_float_wave_is_readable_by_independent_decoder() {
        let mut bytes = header(16).to_vec();
        for sample in [0.25f32, -0.25, 0.75, -0.75] {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.duration(), 2);
        assert_eq!(
            reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            vec![0.25, -0.25, 0.75, -0.75]
        );
    }

    #[test]
    fn large_recording_uses_rf64_and_retains_full_sizes() {
        let length = 5u64 * 1024 * 1024 * 1024;
        let bytes = header(length);
        assert_eq!(&bytes[..4], b"RF64");
        assert_eq!(&bytes[12..16], b"ds64");
        assert_eq!(
            u64::from_le_bytes(bytes[28..36].try_into().unwrap()),
            length
        );
        assert_eq!(
            u64::from_le_bytes(bytes[36..44].try_into().unwrap()),
            length / 8
        );
        assert_eq!(
            u32::from_le_bytes(bytes[88..92].try_into().unwrap()),
            u32::MAX
        );
    }
}
