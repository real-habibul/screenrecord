use super::*;
use screenrecord_core::{SubtitleCue, SubtitleRange, SubtitleWord, VideoClip};

fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(bytes).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

fn ass_frame(ffmpeg: &Path, directory: &Path, width: u32, height: u32, frame: u32) -> Vec<u8> {
    decode(&run(media_command(ffmpeg).current_dir(directory).args([
        "-filter_threads",
        "1",
        "-f",
        "lavfi",
        "-i",
        &format!("color=c=black@0.0:s={width}x{height}:r=60:d=2,format=rgba"),
        "-vf",
        &format!("ass=f=subtitles.ass:alpha=1:shaping=complex,select=eq(n\\,{frame}),format=rgba"),
        "-frames:v",
        "1",
        "-f",
        "image2pipe",
        "-c:v",
        "png",
        "-",
    ])))
    .2
}

fn word(text: &str, start: f64, end: f64) -> SubtitleWord {
    SubtitleWord {
        text: text.into(),
        start,
        end,
    }
}

#[test]
fn word_highlight_rasters_keep_unicode_layout_and_match_exact_export_frames() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.edits.trim_start = 0.0;
    project.edits.trim_end = 2.0;
    project.edits.subtitle_style.highlight_words = true;
    project.edits.subtitle_style.font_size = 96.0;
    let words = vec![
        word("café", 0.1, 0.3),
        word("中文屏幕字幕文字测试示例", 0.35, 0.55),
        word("{\\pos(0,0)}", 0.6, 0.8),
        word("العربية", 1.3, 1.6),
    ];
    project.edits.subtitles = vec![SubtitleCue {
        id: "words".into(),
        start: 0.1,
        end: 1.8,
        text: words
            .iter()
            .map(|w| w.text.trim())
            .collect::<Vec<_>>()
            .join(" "),
        words,
        source_ranges: vec![
            SubtitleRange {
                start: 0.1,
                end: 1.0,
            },
            SubtitleRange {
                start: 1.3,
                end: 1.8,
            },
        ],
    }];
    let cancel = AtomicBool::new(false);
    project.validate().unwrap();
    let began = Instant::now();
    let mut rasters = Vec::new();
    for index in [None, Some(0), Some(1), Some(2), Some(3)] {
        rasters
            .push(decode(&subtitle_preview(&project, "words", index, &ffmpeg, &cancel).unwrap()).2);
    }
    for (index, raster) in rasters.iter().enumerate().skip(1) {
        assert!(
            raster
                .as_chunks::<4>()
                .0
                .iter()
                .zip(rasters[0].as_chunks::<4>().0)
                .all(|(a, b)| a[3] == b[3]),
            "Highlight {index} moved glyphs or their background"
        );
        assert_ne!(raster, &rasters[0]);
        assert!(
            raster[..1920 * 4 * 500]
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 0),
            "Literal tag moved highlighted text"
        );
    }
    eprintln!("Five 1080p subtitle appearances (base + four words): {:.4}s; identical alpha/layout for all variants", began.elapsed().as_secs_f64());
    let began = Instant::now();
    let batch = subtitle_previews(
        &project,
        "words",
        &[None, Some(0), Some(1), Some(2), Some(3)],
        &ffmpeg,
        &cancel,
    )
    .unwrap();
    let elapsed = began.elapsed().as_secs_f64();
    assert_eq!(batch.len(), 5);
    for (index, png) in batch.iter().enumerate() {
        assert!(
            decode(png).2 == rasters[index],
            "Five-state batch changed appearance {index}"
        );
    }
    eprintln!("Five-appearance batch: {elapsed:.4}s, {:.2} appearances/s; exact single-raster RGBA equality", 5.0 / elapsed);
    let geometry = math::layout(&project, "1080p").unwrap();
    subtitles::write_chunk(&project, &geometry, directory.path(), 0.0, 120, 60).unwrap();
    for (frame, index) in [(6, 1), (18, 0), (21, 2), (36, 3), (48, 0), (78, 4), (95, 4)] {
        assert!(
            ass_frame(&ffmpeg, directory.path(), 1920, 1080, frame) == rasters[index],
            "Preview and export differed at frame {frame}"
        );
    }
    assert!(
        ass_frame(&ffmpeg, directory.path(), 1920, 1080, 66)
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 0),
        "Caption appeared outside sourceRanges coverage"
    );
    assert!(
        ass_frame(&ffmpeg, directory.path(), 1920, 1080, 96)
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 0),
        "Projected timed cue remained past its last word"
    );
    subtitles::write_chunk(
        &project,
        &geometry,
        directory.path(),
        0.23333333333333334,
        30,
        60,
    )
    .unwrap();
    assert!(
        ass_frame(&ffmpeg, directory.path(), 1920, 1080, 4) == rasters[0],
        "Word end was shifted after a nonzero chunk start"
    );
    assert!(
        ass_frame(&ffmpeg, directory.path(), 1920, 1080, 7) == rasters[2],
        "Word start was shifted after a nonzero chunk start"
    );
    assert!(subtitle_preview(&project, "words", Some(4), &ffmpeg, &cancel).is_err());
    assert!(subtitle_preview(&project, "words", Some(0), &ffmpeg, &AtomicBool::new(true)).is_err());
}

#[test]
fn eight_word_preview_batch_matches_singles_and_rejects_invalid_or_cancelled_jobs() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.edits.subtitle_style.highlight_words = true;
    project.edits.subtitle_style.font_size = 72.0;
    let words: Vec<_> = [
        "one",
        "two",
        "three",
        "café",
        "中文",
        "{literal}",
        "العربية",
        "eight",
    ]
    .iter()
    .enumerate()
    .map(|(index, text)| word(text, index as f64 * 0.2, (index + 1) as f64 * 0.2))
    .collect();
    project.edits.subtitles = vec![SubtitleCue {
        id: "batch".into(),
        start: 0.0,
        end: 1.8,
        text: words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        words,
        source_ranges: vec![],
    }];
    let indices: Vec<_> = (0..8).map(Some).collect();
    let cancel = AtomicBool::new(false);
    let began = Instant::now();
    let batch = subtitle_previews(&project, "batch", &indices, &ffmpeg, &cancel).unwrap();
    let elapsed = began.elapsed().as_secs_f64();
    assert_eq!(batch.len(), 8);
    let compressed: usize = batch.iter().map(Vec::len).sum();
    for (index, png) in batch.iter().enumerate() {
        let single = subtitle_preview(&project, "batch", Some(index), &ffmpeg, &cancel).unwrap();
        assert!(
            decode(png).2 == decode(&single).2,
            "Eight-state batch changed appearance {index}"
        );
    }
    eprintln!("Eight-word batch: {elapsed:.4}s, {:.2} appearances/s, {compressed} compressed bytes; all eight states match singles",8.0/elapsed);
    assert!(subtitle_previews(&project, "batch", &[], &ffmpeg, &cancel).is_err());
    assert!(subtitle_previews(&project, "batch", &[None; 9], &ffmpeg, &cancel).is_err());
    assert!(subtitle_previews(&project, "batch", &[Some(8)], &ffmpeg, &cancel).is_err());
    assert!(
        subtitle_previews(&project, "batch", &indices, &ffmpeg, &AtomicBool::new(true)).is_err()
    );
    let cancelled = AtomicBool::new(false);
    let began = Instant::now();
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            cancelled.store(true, Ordering::Relaxed);
        });
        subtitle_previews(&project, "batch", &indices, &ffmpeg, &cancelled)
    });
    assert!(result.is_err());
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "Cancelled raster process was not released promptly"
    );
}

fn render(ffmpeg: &Path, project: &Project, name: &str) -> ExportOptions {
    let mut output = options(Path::new(&project.directory), name);
    output.fps = 30;
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    export_inner(
        project,
        &output,
        ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        7,
    )
    .unwrap();
    output
}

fn rgb_frame(ffmpeg: &Path, path: &str, time: f64) -> Vec<u8> {
    run(media_command(ffmpeg).args([
        "-ss",
        &time.to_string(),
        "-i",
        path,
        "-frames:v",
        "1",
        "-pix_fmt",
        "rgb24",
        "-f",
        "rawvideo",
        "-",
    ]))
}

#[test]
fn projected_word_export_removes_deleted_text_and_source_range_gaps_remain_empty() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let original = std::fs::read(&project.screen_path).unwrap();
    project.edits.trim_start = 0.1;
    project.edits.trim_end = 2.0;
    project.edits.background = "#000000".into();
    project.edits.camera.layout = "fullscreen".into();
    project.edits.camera.enabled = false;
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits.subtitle_style.highlight_words = true;
    project.edits.subtitle_style.font_size = 64.0;
    project.edits.video_clips = vec![
        VideoClip {
            id: "first".into(),
            start: 0.1,
            end: 0.6,
        },
        VideoClip {
            id: "last".into(),
            start: 1.4,
            end: 2.0,
        },
    ];
    let words = vec![
        word("TRIMMED", 0.0, 0.2),
        word("Kept", 0.25, 0.5),
        word("SECRET", 0.8, 1.1),
        word("words", 1.5, 1.8),
    ];
    project.edits.subtitles = vec![SubtitleCue {
        id: "safe".into(),
        start: 0.0,
        end: 2.0,
        text: words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        words,
        source_ranges: vec![],
    }];
    project.validate().unwrap();
    let safe = screenrecord_core::projected_subtitle(
        &project.edits,
        project.duration,
        &project.edits.subtitles[0],
    )
    .unwrap();
    assert_eq!(safe.text, "Kept words");
    let geometry = math::layout(&project, "1080p").unwrap();
    subtitles::write_chunk(&project, &geometry, directory.path(), 0.1, 30, 60).unwrap();
    let ass = std::fs::read_to_string(directory.path().join("subtitles.ass")).unwrap();
    assert!(!ass.contains("SECRET") && !ass.contains("TRIMMED"));
    let output = render(&ffmpeg, &project, "safe-words.mp4");
    let mut expected = project.clone();
    expected.edits.subtitles = vec![safe];
    let raster = decode(
        &subtitle_preview(&expected, "safe", Some(0), &ffmpeg, &AtomicBool::new(false)).unwrap(),
    )
    .2;
    let actual = rgb_frame(&ffmpeg, &output.output_path, 0.2);
    let mut compared = 0usize;
    let error: u64 = actual
        .as_chunks::<3>()
        .0
        .iter()
        .zip(raster.as_chunks::<4>().0)
        .filter(|(_, rgba)| rgba[3] > 0)
        .map(|(rgb, rgba)| {
            compared += 3;
            (0..3)
                .map(|channel| {
                    (rgb[channel] as i32 - (rgba[channel] as u32 * rgba[3] as u32 / 255) as i32)
                        .unsigned_abs() as u64
                })
                .sum::<u64>()
        })
        .sum();
    let mae = error as f64 / compared as f64;
    assert!(
        compared > 10_000 && mae < 3.0,
        "Encoded caption disagrees with projected preview raster: caption-region MAE {mae}"
    );
    eprintln!("H264 caption-region RGB MAE {mae:.5}; removed/trimmed words absent from ASS");
    project.edits.video_clips.clear();
    project.edits.trim_start = 0.0;
    project.edits.subtitles = vec![SubtitleCue {
        id: "imported".into(),
        start: 0.1,
        end: 1.7,
        text: "Imported caption".into(),
        words: vec![],
        source_ranges: vec![
            SubtitleRange {
                start: 0.1,
                end: 0.4,
            },
            SubtitleRange {
                start: 1.4,
                end: 1.7,
            },
        ],
    }];
    let output = render(&ffmpeg, &project, "imported-ranges.mp4");
    assert!(
        rgb_frame(&ffmpeg, &output.output_path, 0.8)
            .iter()
            .all(|pixel| *pixel < 8),
        "An imported SRT cue leaked across its source coverage gap or revealed the screen"
    );
    for time in [0.2, 1.5] {
        assert!(rgb_frame(&ffmpeg, &output.output_path, time)
            .as_chunks::<3>()
            .0
            .iter()
            .any(|pixel| pixel.iter().all(|value| *value > 220)));
    }
    assert_eq!(std::fs::read(&project.screen_path).unwrap(), original);
}
