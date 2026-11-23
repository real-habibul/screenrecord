use super::*;
use screenrecord_core::VideoClip;
use std::io::Write;

fn fixture_engine() -> PathBuf {
    engine().unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/ffmpeg.exe")
    })
}

fn rising_edges(active: impl Iterator<Item = bool>, rate: f64) -> Vec<f64> {
    let mut previous = false;
    active
        .enumerate()
        .filter_map(|(index, current)| {
            let rising = current && !previous;
            previous = current;
            rising.then_some(index as f64 / rate)
        })
        .collect()
}

fn decode_markers(
    ffmpeg: &Path,
    mp4: &Path,
    start: f64,
    length: Option<f64>,
) -> (u64, Vec<f64>, Vec<f64>) {
    let mut video = media_command(ffmpeg);
    video.args(["-ss", &start.to_string(), "-i"]).arg(mp4);
    if let Some(length) = length {
        video.args(["-t", &length.to_string()]);
    }
    let pixels = run(video.args([
        "-an",
        "-vf",
        "scale=1:1,format=gray",
        "-fps_mode",
        "passthrough",
        "-f",
        "rawvideo",
        "-",
    ]));
    let frames = pixels.len() as u64;
    let flashes = rising_edges(pixels.into_iter().map(|gray| gray > 150), 30.0);
    (frames, flashes, decode_tones(ffmpeg, mp4, start, length))
}

fn decode_tones(ffmpeg: &Path, path: &Path, start: f64, length: Option<f64>) -> Vec<f64> {
    let mut audio = media_command(ffmpeg);
    audio.arg("-i").arg(path).arg("-vn");
    if let Some(length) = length {
        // Count samples from the beginning so AAC packet preroll cannot shift the measured onset.
        audio.args([
            "-af",
            &format!(
                "atrim=start_sample={}:end_sample={}",
                (start * 48000.0).round() as u64,
                ((start + length) * 48000.0).round() as u64
            ),
        ]);
    }
    let bytes = run(audio.args(["-ac", "1", "-ar", "48000", "-f", "f32le", "-"]));
    rising_edges(
        bytes.chunks(48 * 4).map(|millisecond| {
            millisecond
                .as_chunks::<4>()
                .0
                .iter()
                .any(|sample| f32::from_le_bytes(*sample).abs() > 0.18)
        }),
        1000.0,
    )
}

#[test]
#[ignore = "Accelerated 8-hour FFmpeg fixture; uses about 250 MB of temporary media."]
fn eight_hour_source_preserves_flash_and_tone_sync_across_fractional_cuts() {
    const PERIOD: f64 = 1.001;
    const DURATION: f64 = 28_800.0;
    let ffmpeg = fixture_engine();
    let directory = tempfile::tempdir().unwrap();
    let motif = directory.path().join("motif.mp4");
    let source = directory.path().join("eight-hours.mp4");
    let start = Instant::now();
    let encoder = process::software_encoder(
        &ffmpeg,
        process::VideoSpec {
            width: 160,
            height: 90,
            fps: 30,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    run(media_command(&ffmpeg)
        .args(["-f", "lavfi", "-i", "color=black:s=160x90:r=30000/1001:d=10.01,drawbox=c=white:t=fill:enable='between(mod(n,30),12,17)'",
               "-f", "lavfi", "-i", "aevalsrc='if(between(mod(t,1.001),0.4004,0.6006),0.55*sin(2*PI*997*t),0)':s=44100:d=10.01"])
        .args(encoder.args())
        .args(["-b:v", "140k", "-g", "30", "-bf", "0", "-video_track_timescale", "30000", "-pix_fmt", "yuv420p", "-c:a", "alac", "-sample_fmt", "s16p"])
        .arg(&motif));
    run(media_command(&ffmpeg)
        .args(["-stream_loop", "2877", "-i"])
        .arg(&motif)
        .args(["-t", "28800", "-c", "copy", "-movflags", "+faststart"])
        .arg(&source));
    let generation = start.elapsed();
    let info = probe_media(&ffmpeg, &source).unwrap();
    assert_eq!(info.video().unwrap().nb_frames.as_deref(), Some("863137"));
    assert_eq!(
        info.video().unwrap().avg_frame_rate.as_deref(),
        Some("30000/1001")
    );
    assert!((info.duration().unwrap() - DURATION).abs() < 0.15);

    let mut project = fixture(&ffmpeg, directory.path());
    project.screen_path = source.to_string_lossy().into_owned();
    project.system_audio_path = Some(project.screen_path.clone());
    project.camera_path = None;
    project.microphone_path = None;
    project.width = 160;
    project.height = 90;
    project.duration = DURATION;
    project.cursor.clear();
    project.edits = EditSettings::new(DURATION);
    project.edits.camera.enabled = false;
    project.edits.padding = 0;
    project.edits.corner_radius = 0;
    let mut expected = Vec::new();
    let mut output_time = 0.0;
    for (cycle, from, to) in [
        (0, 0.071, 0.940),
        (3596, 0.073, 0.938),
        (14385, 0.059, 0.941),
        (28737, 0.067, 33.959),
    ] {
        let base = f64::from(cycle) * PERIOD;
        project.edits.video_clips.push(VideoClip {
            id: format!("clip-{cycle}"),
            start: base + from,
            end: base + to,
        });
        for marker in 0..34 {
            let source_time = f64::from(marker) * PERIOD + 0.4004;
            if source_time >= from && source_time < to {
                expected.push(output_time + source_time - from);
            }
        }
        output_time += to - from;
    }
    let mut output = options(directory.path(), "long-source-cuts.mp4");
    output.fps = 30;
    let start = Instant::now();
    export(
        &project,
        &output,
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let export_wall = start.elapsed();
    let mp4 = Path::new(&output.output_path);
    let info = probe_media(&ffmpeg, mp4).unwrap();
    let frames = (output_time * 30.0_f64).ceil() as u64;
    assert_eq!(
        info.video()
            .unwrap()
            .nb_frames
            .as_deref()
            .unwrap()
            .parse::<u64>()
            .unwrap(),
        frames
    );
    let (decoded_frames, flashes, tones) = decode_markers(&ffmpeg, mp4, 0.0, None);
    assert_eq!(decoded_frames, frames);
    assert_eq!(
        flashes.len(),
        expected.len(),
        "Unexpected video pulses: {flashes:?}"
    );
    assert_eq!(
        tones.len(),
        expected.len(),
        "Unexpected audio pulses: {tones:?}"
    );
    let mut max_video = 0.0_f64;
    let mut max_audio = 0.0_f64;
    let mut max_av = 0.0_f64;
    for ((expected, flash), tone) in expected.iter().zip(&flashes).zip(&tones) {
        max_video = max_video.max((flash - expected).abs());
        max_audio = max_audio.max((tone - expected).abs());
        max_av = max_av.max((flash - tone).abs());
    }
    eprintln!("8h source: bytes={}, generation={generation:?}, retained={output_time:.6}s, export={export_wall:?}, frames={frames}, pulses={}, max_flash_error={max_video:.6}s, max_tone_error={max_audio:.6}s, max_av={max_av:.6}s, last_flash={:.6}s, last_tone={:.6}s, output_bytes={}",
        source.metadata().unwrap().len(), expected.len(), flashes.last().unwrap(), tones.last().unwrap(), mp4.metadata().unwrap().len());
    assert!(
        max_video <= 1.0 / 30.0 + 0.001,
        "Flash timing drift: {max_video}"
    );
    assert!(max_audio <= 0.003, "Tone timing drift: {max_audio}");
    assert!(max_av <= 1.0 / 30.0 + 0.003, "A/V timing drift: {max_av}");
    verify_hour_mux(&ffmpeg, &mut project, &output);
}

fn verify_hour_mux(ffmpeg: &Path, project: &mut Project, output: &ExportOptions) {
    let stage = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    project.edits.video_clips = (0..120)
        .map(|i| {
            let start = f64::from(i) * 31.031;
            VideoClip {
                id: format!("repeat-{i}"),
                start,
                end: start + 30.0,
            }
        })
        .collect();
    let graph =
        graph::RenderGraph::prepare(project, output, ffmpeg, stage.path(), &cancel).unwrap();
    let chunks = graph.chunks(graph::MAX_CHUNK_FRAMES).unwrap();
    let audio = graph.audio_chunks();
    assert_eq!(chunks.len(), 120);
    assert_eq!(audio.len(), 120);
    assert!(chunks.iter().all(|chunk| chunk.frames == 900));
    assert!(audio.iter().all(|chunk| chunk.samples == 1_440_000));
    let encoder = process::choose_encoder(
        ffmpeg,
        process::VideoSpec {
            width: graph.layout.width,
            height: graph.layout.height,
            fps: output.fps,
        },
        &cancel,
    )
    .unwrap();
    run(&mut graph
        .video_command(project, output, ffmpeg, encoder, chunks[0], &cancel)
        .unwrap());
    run(&mut graph.audio_command(ffmpeg, audio[0]));
    // The source flash and tone repeat every 1.001s. Each retained span starts 31 full cycles later.
    // Reuse equivalent encoded chunks to exercise hour-long mux timestamps with bounded fixture storage.
    let video_chunk = stage.path().join("video-00000.mp4");
    let audio_chunk = stage.path().join("audio-00000.wav");
    for index in 1..120 {
        fs::hard_link(
            &video_chunk,
            stage.path().join(format!("video-{index:05}.mp4")),
        )
        .unwrap();
        fs::hard_link(
            &audio_chunk,
            stage.path().join(format!("audio-{index:05}.wav")),
        )
        .unwrap();
    }
    let start = Instant::now();
    run(&mut graph.mux_command(project, output, ffmpeg, &chunks).unwrap());
    let mux_wall = start.elapsed();
    let path = stage.path().join("encoded.mp4");
    let info = probe_media(ffmpeg, &path).unwrap();
    validate_output(&info, &graph.layout, 108_000, 30, 3600.0, true).unwrap();
    let reference_tones = decode_tones(ffmpeg, &audio_chunk, 0.0, None);
    assert_eq!(reference_tones.len(), 30);
    let mut max_tone_error = 0.0_f64;
    let mut max_mux_error = 0.0_f64;
    let mut max_av = 0.0_f64;
    for seek in [0u32, 1799, 3598] {
        let (frames, flashes, tones) = decode_markers(ffmpeg, &path, f64::from(seek), Some(2.0));
        assert_eq!((frames, flashes.len(), tones.len()), (60, 2, 2));
        let window_start = f64::from(seek % 30);
        for (offset, (flash, tone)) in flashes.iter().zip(&tones).enumerate() {
            let block_second = (seek + offset as u32) % 30;
            let block_start = if block_second < seek % 30 { 30.0 } else { 0.0 };
            let expected = block_start + f64::from(block_second) * 1.001 + 0.4004 - window_start;
            let reference = block_start + reference_tones[block_second as usize] - window_start;
            max_tone_error = max_tone_error.max((tone - expected).abs());
            max_mux_error = max_mux_error.max((tone - reference).abs());
            max_av = max_av.max((flash - tone).abs());
        }
        eprintln!("1h output seek={seek}s flashes={flashes:?}s tones={tones:?}s");
    }
    eprintln!("1h production mux: 120 chunks, 108000 frames, mux_wall={mux_wall:?}, encoded_video_chunk_bytes={}, pcm_chunk_bytes={}, mp4_bytes={}, max_tone_error={max_tone_error:.6}s, max_mux_error={max_mux_error:.6}s, max_av={max_av:.6}s",
        video_chunk.metadata().unwrap().len(), audio_chunk.metadata().unwrap().len(), path.metadata().unwrap().len());
    assert!(max_tone_error < 0.003);
    assert!(max_mux_error < 0.003);
    assert!(max_av <= 1.0 / 30.0 + 0.003);
}

#[test]
#[ignore = "Requires a disposable 1 MiB quota volume in SCREENRECORD_TEST_QUOTA_ROOT."]
fn full_export_refuses_full_disk_and_retries_after_space_is_freed() {
    let ffmpeg = fixture_engine();
    let quota =
        PathBuf::from(std::env::var_os("SCREENRECORD_TEST_QUOTA_ROOT").expect("quota root"));
    let directory = tempfile::tempdir_in(&quota).unwrap();
    let source = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, source.path());
    project.edits = EditSettings::new(project.duration);
    project.edits.trim_end = 0.5;
    project.edits.camera.enabled = false;
    project.camera_path = None;
    project.microphone_path = None;
    let originals: Vec<_> = [
        project.screen_path.as_str(),
        project.system_audio_path.as_deref().unwrap(),
    ]
    .into_iter()
    .map(|path| (path, fs::read(path).unwrap()))
    .collect();
    let mut output = options(directory.path(), "retry.mp4");
    output.fps = 30;
    let mut filler = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    let error = (0..512)
        .find_map(|_| filler.as_file_mut().write_all(&[0x75; 4096]).err())
        .expect("The test requires an isolated quota volume of at most 2 MiB.");
    #[cfg(windows)]
    let quota_exhausted = {
        let free = process::available_bytes(&quota).expect("Query the quota filesystem");
        eprintln!("Free bytes at the failed write: {free}; Windows error: {error}");
        free == 0 && [Some(31), Some(112)].contains(&error.raw_os_error())
    };
    #[cfg(not(windows))]
    let quota_exhausted = error.kind() == std::io::ErrorKind::StorageFull;
    assert!(quota_exhausted, "{error}");
    let used = filler.as_file().metadata().unwrap().len();
    filler
        .as_file_mut()
        .set_len(used.checked_sub(64 * 1024).unwrap())
        .unwrap();
    filler.as_file().sync_all().unwrap();
    let states = Arc::new(Mutex::new(Vec::new()));
    let observed = states.clone();
    let failure = export(
        &project,
        &output,
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        move |p| observed.lock().unwrap().push(p.state),
    )
    .unwrap_err();
    assert!(failure.contains("disk space"), "{failure}");
    assert!(
        states
            .lock()
            .unwrap()
            .last()
            .is_some_and(|state| state == "failed"),
        "{failure}"
    );
    assert!(!Path::new(&output.output_path).exists());
    assert_eq!(
        fs::read_dir(directory.path()).unwrap().count(),
        1,
        "Staging files left after failed export"
    );
    drop(filler);
    export(
        &project,
        &output,
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let path = Path::new(&output.output_path);
    let info = probe_media(&ffmpeg, path).unwrap();
    assert_eq!(info.video().unwrap().nb_frames.as_deref(), Some("15"));
    assert!(info.has_audio());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    for (path, before) in originals {
        assert_eq!(fs::read(path).unwrap(), before)
    }
    eprintln!(
        "Disk-full error={error} export_diagnostic={failure} retry_bytes={}",
        path.metadata().unwrap().len()
    );
}
