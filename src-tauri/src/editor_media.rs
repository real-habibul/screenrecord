use screenrecord_core::{MusicClip, Project};
use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn import_music(
    project: &Project,
    source: &Path,
    start: f64,
    ffmpeg: &Path,
) -> Result<MusicClip, String> {
    if !start.is_finite() || start < 0.0 || start + 0.1 > project.duration + 1e-9 {
        return Err("Choose an import position with at least 0.1 seconds remaining.".into());
    }
    if project.edits.music.len() >= 32 {
        return Err("A project can contain at most 32 music clips.".into());
    }
    let source = dunce::canonicalize(source).map_err(|_| "The selected music file is missing.")?;
    let extension = source
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["mp3", "wav", "m4a", "aac", "flac", "ogg", "opus"].contains(&extension.as_str()) {
        return Err("Choose MP3, WAV, M4A, AAC, FLAC, OGG, or Opus audio.".into());
    }
    let metadata = source.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 512 * 1024 * 1024 {
        return Err("Music files must be nonempty and at most 512 MiB.".into());
    }
    let info = screenrecord_render::probe_media(ffmpeg, &source)?;
    let duration = info
        .duration()
        .filter(|d| (0.1..=3600.0).contains(d))
        .ok_or("Music must have a valid duration between 0.1 seconds and one hour.")?;
    if !info.has_audio() {
        return Err("The selected file has no audio track.".into());
    }
    let temporary =
        tempfile::NamedTempFile::new_in(&project.directory).map_err(|e| e.to_string())?;
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(&source)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            "2",
            "-ar",
            "48000",
            "-c:a",
            "pcm_f32le",
            "-t",
            "3600",
            "-f",
            "wav",
        ]);
    if info
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"))
        .and_then(|stream| stream.channels)
        == Some(1)
    {
        command.args(["-af", "pan=stereo|c0=c0|c1=c0"]);
    }
    command.arg(temporary.path());
    run(&mut command, Duration::from_secs(120))?;
    let normalized = screenrecord_render::probe_media(ffmpeg, temporary.path())?;
    let source_duration = normalized
        .duration()
        .filter(|d| d.is_finite() && *d >= 0.1)
        .ok_or("Imported music did not produce playable audio.")?;
    if !normalized.has_audio() || (duration - source_duration).abs() > 0.25 {
        return Err(
            "Music conversion ended before the source finished. The source is unchanged.".into(),
        );
    }
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let destination = Path::new(&project.directory).join(format!("music-{id}.wav"));
    temporary
        .persist_noclobber(&destination)
        .map_err(|e| e.to_string())?;
    let name: String = source
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    Ok(MusicClip {
        id,
        name,
        path: destination.to_string_lossy().into_owned(),
        source_duration,
        start,
        source_start: 0.0,
        duration: source_duration.min(project.duration - start),
        volume: 0.25,
        fade_in: 0.0,
        fade_out: 0.0,
    })
}

fn run(command: &mut Command, timeout: Duration) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "Cannot start the audio conversion engine.".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or("Audio converter did not provide a diagnostic pipe.")?;
    let reader = thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buffer) {
            if n == 0 {
                break;
            }
        }
    });
    let deadline = Instant::now() + timeout;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(
                        "The music file could not be decoded. Choose a different audio file."
                            .into(),
                    )
                }
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(30)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(
                    "Audio import exceeded the two-minute limit. Choose a shorter file.".into(),
                );
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("The audio converter stopped unexpectedly.".into());
            }
        }
    };
    let _ = reader.join();
    result
}

pub fn export_srt(project: &Project, output: &Path) -> Result<(), String> {
    if !output.is_absolute()
        || output
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("srt"))
            != Some(true)
    {
        return Err("Choose an absolute SRT output path.".into());
    }
    if output.exists() {
        return Err("The destination already exists. Choose a new filename.".into());
    }
    let text = screenrecord_core::subtitles_srt(&project.edits, project.duration)?;
    if text.is_empty() {
        return Err("No subtitle cues intersect the selected trim.".into());
    }
    let parent = output
        .parent()
        .filter(|p| p.is_dir())
        .ok_or("The destination folder is missing.")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    use std::io::Write;
    temporary
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary
        .persist_noclobber(output)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn preview_directory(project: &Project) -> Result<std::path::PathBuf, String> {
    let project_directory = dunce::canonicalize(&project.directory).map_err(|e| e.to_string())?;
    let directory = project_directory.join("editor-previews");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let canonical = dunce::canonicalize(&directory).map_err(|e| e.to_string())?;
    if canonical != directory || !canonical.starts_with(project_directory) {
        return Err("Subtitle preview cache must remain inside the project folder.".into());
    }
    Ok(canonical)
}

pub(crate) fn import_srt(
    project: &screenrecord_core::Project,
    path: &std::path::Path,
) -> Result<Vec<screenrecord_core::SubtitleCue>, String> {
    use std::io::Read;
    const LIMIT: u64 = 2 * 1024 * 1024;
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("srt"))
    {
        return Err("Choose a local .srt subtitle file.".into());
    }
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Cannot open the SRT file: {e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Err("SRT files must be regular files of at most 2 MiB.".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let after = file.metadata().map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT
        || metadata.len() != after.len()
        || metadata.modified().ok() != after.modified().ok()
    {
        return Err(
            "The SRT file changed while being read or exceeds 2 MiB. Retry the import.".into(),
        );
    }
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !(bytes.len() - 2).is_multiple_of(2) {
            return Err("The SRT file contains incomplete UTF-16 text.".into());
        }
        let little = bytes.starts_with(&[0xff, 0xfe]);
        let units: Vec<_> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|unit| {
                if little {
                    u16::from_le_bytes([unit[0], unit[1]])
                } else {
                    u16::from_be_bytes([unit[0], unit[1]])
                }
            })
            .collect();
        String::from_utf16(&units)
            .map_err(|_| "Save the SRT file as valid UTF-8 or UTF-16 text.".to_string())?
    } else {
        String::from_utf8(bytes)
            .map_err(|_| "Save the SRT file as valid UTF-8 or UTF-16 text.".to_string())?
    };
    screenrecord_core::subtitles_from_srt(&project.edits, project.duration, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_import_reads_utf16_preserves_input_and_does_not_modify_existing_captions_on_failure() {
        let directory = tempfile::tempdir().unwrap();
        let project = project(directory.path());
        let path = directory.path().join("captions.srt");
        let text = "1\r\n00:00:00,100 --> 00:00:00,500\r\nHalo 世界\r\n";
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        fs::write(&path, &bytes).unwrap();
        let cues = import_srt(&project, &path).unwrap();
        assert_eq!(cues[0].text, "Halo 世界");
        assert!(cues[0].words.is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let before = serde_json::to_string(&project.edits).unwrap();
        fs::write(&path, [0xff, 0xfe, 0x20]).unwrap();
        assert!(import_srt(&project, &path).is_err());
        assert_eq!(serde_json::to_string(&project.edits).unwrap(), before);
    }
    use screenrecord_core::{EditSettings, SubtitleCue, PROJECT_VERSION};

    fn project(directory: &Path) -> Project {
        Project {
            version: PROJECT_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: "Editor media fixture".into(),
            created_at: "2026-11-11T00:00:00Z".into(),
            directory: directory.to_string_lossy().into_owned(),
            screen_path: directory.join("screen.mp4").to_string_lossy().into_owned(),
            microphone_path: None,
            system_audio_path: None,
            camera_path: None,
            camera_offset_seconds: 0.0,
            camera_intervals: None,
            width: 1920,
            height: 1080,
            fps: 30,
            duration: 3.0,
            cursor: vec![],
            edits: EditSettings::new(3.0),
            status: "ready".into(),
        }
    }

    #[test]
    fn srt_export_is_trimmed_and_never_overwrites() {
        let directory = tempfile::tempdir().unwrap();
        let mut project = project(directory.path());
        project.edits.trim_start = 1.0;
        project.edits.subtitles.push(SubtitleCue {
            id: "cue".into(),
            start: 0.5,
            end: 2.5,
            text: "Halo dunia".into(),
            words: vec![],
            source_ranges: vec![],
        });
        let output = directory.path().join("captions.srt");
        export_srt(&project, &output).unwrap();
        let bytes = fs::read(&output).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("00:00:00,000 --> 00:00:01,500"));
        assert!(export_srt(&project, &output).is_err());
        assert_eq!(fs::read(output).unwrap(), bytes);
        assert!(export_srt(&project, Path::new("relative.srt")).is_err());
    }

    #[test]
    fn imported_music_is_normalized_and_original_is_preserved() {
        let ffmpeg = std::env::var_os("SCREENRECORD_FFMPEG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/ffmpeg.exe"));
        assert!(
            ffmpeg.is_file(),
            "Set SCREENRECORD_FFMPEG for the music import media test"
        );
        let source_directory = tempfile::tempdir().unwrap();
        let source = source_directory.path().join("song.wav");
        run(
            Command::new(&ffmpeg)
                .args([
                    "-hide_banner",
                    "-nostdin",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "aevalsrc=0.2:s=44100:d=1.2",
                    "-c:a",
                    "pcm_f32le",
                ])
                .arg(&source),
            Duration::from_secs(20),
        )
        .unwrap();
        let original = fs::read(&source).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut project = project(directory.path());
        let clip = import_music(&project, &source, 2.0, &ffmpeg).unwrap();
        assert!((clip.duration - 1.0).abs() < 1e-6);
        assert!((clip.source_duration - 1.2).abs() < 0.01);
        assert!(Path::new(&clip.path).starts_with(directory.path()));
        let info = screenrecord_render::probe_media(&ffmpeg, Path::new(&clip.path)).unwrap();
        assert!(info
            .streams
            .iter()
            .any(|s| s.codec_name.as_deref() == Some("pcm_f32le")
                && s.sample_rate.as_deref() == Some("48000")));
        let pcm = directory.path().join("normalized.f32");
        run(
            Command::new(&ffmpeg)
                .args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
                .arg(&clip.path)
                .args(["-c:a", "pcm_f32le", "-f", "f32le"])
                .arg(&pcm),
            Duration::from_secs(20),
        )
        .unwrap();
        let pcm = fs::read(pcm).unwrap();
        assert_eq!(pcm.len() / 8, 57_600);
        let maximum_error = pcm
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| (f32::from_le_bytes(*bytes) - 0.2).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            maximum_error < 1e-5,
            "Mono-to-stereo import changed gain: {maximum_error}"
        );
        project.edits.music.push(clip);
        project.edits.validate(project.duration).unwrap();
        assert_eq!(fs::read(&source).unwrap(), original);
        assert!(import_music(&project, &source, 2.95, &ffmpeg).is_err());
    }
}
