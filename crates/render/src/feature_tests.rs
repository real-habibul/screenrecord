use super::*;
use screenrecord_core::{MusicClip, Redaction, SubtitleCue};

fn music(path: &Path) -> MusicClip {
    MusicClip {
        id: "music-1".into(),
        name: "Generated audio".into(),
        path: path.to_string_lossy().into_owned(),
        source_duration: 3.0,
        start: 0.4,
        source_start: 0.25,
        duration: 2.0,
        volume: 0.5,
        fade_in: 0.5,
        fade_out: 0.5,
    }
}

fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(bytes).read_info().unwrap();
    let mut output = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut output).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    output.truncate(info.buffer_size());
    (info.width, info.height, output)
}

#[test]
fn music_offsets_partial_fades_overlap_and_limiter_match_pcm_contract() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.edits.trim_start = 0.65;
    project.edits.trim_end = 2.2;
    project.microphone_path = None;
    project.system_audio_path = None;
    let source = directory.path().join("music constant.wav");
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.2:sample_rate=48000:duration=3",
            "-c:a",
            "pcm_f32le",
        ])
        .arg(&source));
    project.edits.music = vec![music(&source)];
    let mut second = music(&source);
    second.id = "music-2".into();
    second.start = 1.8;
    second.duration = 0.4;
    second.volume = 0.25;
    second.fade_in = 0.0;
    second.fade_out = 0.0;
    project.edits.music.push(second);
    project.validate().unwrap();
    let tracks = audio::prepare(&project, &ffmpeg, &AtomicBool::new(false)).unwrap();
    let mut command = media_command(&ffmpeg);
    command.args(["-f", "lavfi", "-i", "color=s=16x16:r=1:d=2"]);
    let filters = audio::append_inputs(&mut command, &tracks, 1.55);
    command.args([
        "-filter_complex",
        &filters.join(";"),
        "-map",
        "[outa]",
        "-c:a",
        "pcm_f32le",
        "-f",
        "f32le",
        "-",
    ]);
    let bytes = run(&mut command);
    let samples: Vec<_> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|sample| f32::from_le_bytes(sample[..4].try_into().unwrap()) as f64)
        .collect();
    assert_eq!(samples.len(), 74_400);
    let gain = |clip: &MusicClip, time: f64| {
        let t = (time * 48_000.0).round() / 48_000.0 - clip.start;
        if t < 0.0 || t >= clip.duration {
            return 0.0;
        }
        let start = if clip.fade_in > 0.0 {
            (t / clip.fade_in).min(1.0)
        } else {
            1.0
        };
        let end = if clip.fade_out > 0.0 {
            ((clip.duration - t) / clip.fade_out).min(1.0)
        } else {
            1.0
        };
        clip.volume * start.min(end)
    };
    for t in [0.0, 0.1, 0.25, 0.8, 1.15, 1.3, 1.5] {
        let expected = 0.2
            * project
                .edits
                .music
                .iter()
                .map(|clip| gain(clip, t + 0.65))
                .sum::<f64>();
        let actual = samples[(t * 48_000.0_f64).round() as usize];
        assert!(
            (actual - expected).abs() < 0.00005,
            "t={t}: expected {expected}, got {actual}"
        );
    }
    let original_clips = project.edits.music.clone();
    project.edits.music = (0..3)
        .map(|index| {
            let mut clip = music(&source);
            clip.id = format!("loud-{index}");
            clip.volume = 2.0;
            clip.fade_in = 0.0;
            clip.fade_out = 0.0;
            clip
        })
        .collect();
    let tracks = audio::prepare(&project, &ffmpeg, &AtomicBool::new(false)).unwrap();
    let mut command = media_command(&ffmpeg);
    command.args(["-f", "lavfi", "-i", "color=s=16x16:r=1:d=2"]);
    let filters = audio::append_inputs(&mut command, &tracks, 1.55);
    let loud = run(command.args([
        "-filter_complex",
        &filters.join(";"),
        "-map",
        "[outa]",
        "-c:a",
        "pcm_f32le",
        "-f",
        "f32le",
        "-",
    ]));
    let peak = loud
        .as_chunks::<4>()
        .0
        .iter()
        .map(|sample| f32::from_le_bytes(*sample).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        (peak - 0.95).abs() < 0.00001,
        "Combined music exceeded limiter headroom: {peak}"
    );
    project.edits.music = original_clips;
    project.edits.music[0].source_duration = 100.0;
    project.edits.music[0].source_start = 2.0;
    assert!(audio::prepare(&project, &ffmpeg, &AtomicBool::new(false))
        .unwrap_err()
        .contains("source trim"));
}

#[test]
fn subtitle_preview_is_transparent_literal_unicode_and_matches_layout() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.edits.subtitles = vec![SubtitleCue {
        words: vec![],
        source_ranges: vec![],
        id: "cue".into(),
        start: 0.3,
        end: 1.2,
        text: "{\\pos(0,0)} C:\\new\\N 100%\nBahasa Indonesia café 中文 العربية".into(),
    }];
    project.edits.subtitle_style.font_size = 48.0;
    let png = subtitle_preview(&project, "cue", None, &ffmpeg, &AtomicBool::new(false)).unwrap();
    let (width, height, pixels) = decode_png(&png);
    assert_eq!((width, height), (1920, 1080));
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] > 0)
            .count()
            > 10_000
    );
    assert!(
        pixels[..width as usize * 4 * 500]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 0),
        "Literal ASS text moved the subtitle out of its bottom position"
    );
    assert!(pixels
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[0] > 220 && pixel[3] == 255));
    project.edits.aspect_ratio = "9:16".into();
    project.edits.subtitle_style.position = "top".into();
    let (width, height, pixels) = decode_png(
        &subtitle_preview(&project, "cue", None, &ffmpeg, &AtomicBool::new(false)).unwrap(),
    );
    assert_eq!((width, height), (1080, 1920));
    assert!(pixels[width as usize * 4 * 1000..]
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| pixel[3] == 0));
    assert!(subtitle_preview(&project, "cue", None, &ffmpeg, &AtomicBool::new(true)).is_err());
}

#[test]
fn privacy_filters_cover_exact_source_pixels_only_during_active_frames() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.width = 64;
    project.height = 48;
    project.edits.trim_start = 0.0;
    project.edits.redactions = vec![
        Redaction {
            tracking: None,
            id: "blur".into(),
            start: 0.1,
            end: 0.2,
            x: 0.125,
            y: 0.125,
            width: 0.5,
            height: 0.5,
            mode: "blur".into(),
            strength: 8.0,
        },
        Redaction {
            tracking: None,
            id: "solid".into(),
            start: 0.2,
            end: 0.3,
            x: 0.125,
            y: 0.125,
            width: 0.5,
            height: 0.5,
            mode: "solid".into(),
            strength: 1.0,
        },
    ];
    let filter = format!(
        "[0:v]format=rgba[effectsource];{}format=rgb24[out]",
        effects::filters(
            &project,
            0.0,
            10,
            30,
            directory.path(),
            &AtomicBool::new(false),
            &effects::patch_sizes(&project)
        )
        .unwrap()
    );
    let bytes = run(media_command(&ffmpeg).args([
        "-f",
        "lavfi",
        "-i",
        "nullsrc=s=64x48:r=30:d=1,geq=lum='mod(X+Y,2)*255':cb=128:cr=128",
        "-filter_complex",
        &filter,
        "-map",
        "[out]",
        "-frames:v",
        "10",
        "-pix_fmt",
        "rgb24",
        "-f",
        "rawvideo",
        "-",
    ]));
    let frames: Vec<_> = bytes.as_chunks::<{ 64 * 48 * 3 }>().0.iter().collect();
    assert_eq!(frames.len(), 10);
    let roi = |frame: &[u8]| {
        (6..30)
            .flat_map(|y| (8..40).map(move |x| frame[(y * 64 + x) * 3] as f64))
            .collect::<Vec<_>>()
    };
    let variance = |values: &[f64]| {
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / values.len() as f64
    };
    let original = variance(&roi(frames[0]));
    assert!(original > 10_000.0);
    for index in [0, 1, 2, 9] {
        assert_eq!(
            frames[index], frames[0],
            "Effect leaked outside [start,end) at frame{index}"
        );
    }
    for index in [3, 4, 5] {
        assert!(
            variance(&roi(frames[index])) < original * 0.01,
            "Blur did not cover frame{index}"
        );
    }
    for index in [6, 7, 8] {
        assert!(
            roi(frames[index]).iter().all(|value| *value == 0.0),
            "Solid mask did not cover frame{index}"
        );
    }
    for frame in &frames {
        for y in 0..48 {
            for x in 0..64 {
                if !(8..40).contains(&x) || !(6..30).contains(&y) {
                    let pixel = (y * 64 + x) * 3;
                    assert_eq!(&frame[pixel..pixel + 3], &frames[0][pixel..pixel + 3]);
                }
            }
        }
    }
}

#[test]
fn editor_tracks_survive_chunk_boundaries_trim_crop_zoom_and_full_decode() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let source = directory.path().join("music.wav");
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1600:sample_rate=44100:duration=3",
            "-c:a",
            "pcm_f32le",
        ])
        .arg(&source));
    project.edits.music = vec![music(&source)];
    project.edits.subtitles = vec![
        SubtitleCue {
            words: vec![],
            source_ranges: vec![],
            id: "cue-1".into(),
            start: 0.22,
            end: 0.8,
            text: "Start {literal} \\N".into(),
        },
        SubtitleCue {
            words: vec![],
            source_ranges: vec![],
            id: "cue-2".into(),
            start: 0.801,
            end: 2.3,
            text: "Across chunks\nBahasa Indonesia".into(),
        },
    ];
    project.edits.redactions = vec![
        Redaction {
            tracking: None,
            id: "blur".into(),
            start: 0.43,
            end: 1.91,
            x: 0.2,
            y: 0.2,
            width: 0.3,
            height: 0.3,
            mode: "blur".into(),
            strength: 8.0,
        },
        Redaction {
            tracking: None,
            id: "cover".into(),
            start: 0.8,
            end: 2.3,
            x: 0.65,
            y: 0.3,
            width: 0.2,
            height: 0.3,
            mode: "solid".into(),
            strength: 1.0,
        },
    ];
    project.validate().unwrap();
    let before: Vec<_> = [&project.screen_path, &source.to_string_lossy().into_owned()]
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect();
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    let normal = options(directory.path(), "normal.mp4");
    let chunked = options(directory.path(), "chunked.mp4");
    let cancel = Arc::new(AtomicBool::new(false));
    export_inner(
        &project,
        &normal,
        &ffmpeg,
        &cancel,
        &callback,
        graph::MAX_CHUNK_FRAMES,
    )
    .unwrap();
    export_inner(&project, &chunked, &ffmpeg, &cancel, &callback, 12).unwrap();
    let a = video_pixels(&ffmpeg, &normal.output_path);
    let b = video_pixels(&ffmpeg, &chunked.output_path);
    assert_eq!(a.len(), b.len());
    let maximum = a
        .as_chunks::<{ 96 * 54 * 3 }>()
        .0
        .iter()
        .zip(b.as_chunks::<{ 96 * 54 * 3 }>().0.iter())
        .map(|(a, b)| {
            a.iter()
                .zip(b)
                .map(|(a, b)| (*a as f64 - *b as f64).abs())
                .sum::<f64>()
                / a.len() as f64
        })
        .fold(0.0, f64::max);
    assert!(
        maximum < 1.5,
        "Chunk boundary mean pixel difference {maximum}"
    );
    assert_eq!(
        audio_samples(&ffmpeg, &normal.output_path),
        audio_samples(&ffmpeg, &chunked.output_path)
    );
    assert_eq!(fs::read(&project.screen_path).unwrap(), before[0]);
    assert_eq!(fs::read(&source).unwrap(), before[1]);
    run(media_command(&ffmpeg).args(["-i", &chunked.output_path, "-f", "null", "-"]));
    println!("Music/subtitles/privacy: trimmed crop+zoom+webcam 143 frames, forced12-frame chunks match normal max mean RGB difference {maximum:.5}; decoded audio identical; originals unchanged; full decode passed.");
}
