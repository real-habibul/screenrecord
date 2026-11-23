use super::*;
#[path = "assembly_tests.rs"]
mod assembly_media;
#[path = "background_tests.rs"]
mod background_media;
#[path = "camera_layout_tests.rs"]
mod camera_layouts;
#[path = "click_tests.rs"]
mod click_media;
#[path = "compact_tests.rs"]
mod compact_media;
#[path = "cut_tests.rs"]
mod cuts;
#[path = "ducking_tests.rs"]
mod ducking_media;
#[path = "feature_tests.rs"]
mod features;
#[path = "fill_tests.rs"]
mod fill;
#[path = "reframe_tests.rs"]
mod reframing;
#[path = "reliability_tests.rs"]
mod reliability;
#[path = "source_gain_tests.rs"]
mod source_gain;
#[path = "subtitle_word_tests.rs"]
mod subtitle_words;
#[path = "tracking_tests.rs"]
mod tracking;
#[path = "volume_tests.rs"]
mod volume_automation;
use screenrecord_core::{CameraSettings, CropSettings, CursorSample, EditSettings, ZoomSegment};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

fn engine() -> Option<PathBuf> {
    std::env::var_os("SCREENRECORD_FFMPEG")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn run(command: &mut Command) -> Vec<u8> {
    process::hidden(command);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn media_command(ffmpeg: &Path) -> Command {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-v", "error", "-nostdin", "-y"]);
    command
}

fn fixture(ffmpeg: &Path, directory: &Path) -> Project {
    let screen = directory.join("screen.mp4");
    let camera = directory.join("camera.mp4");
    let microphone = directory.join("microphone.wav");
    let system = directory.join("system.wav");
    let software = process::software_encoder(
        ffmpeg,
        process::VideoSpec {
            width: 320,
            height: 180,
            fps: 30,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    run(media_command(ffmpeg).args(["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=2.8,drawbox=x=0:y=130:w=320:h=50:color=white:t=fill:enable='eq(mod(n,2),0)'"]).args(software.args()).arg(&screen));
    run(media_command(ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x120:rate=24:duration=2.8",
        ])
        .args(software.args())
        .arg(&camera));
    for (path, frequency) in [(&microphone, 997), (&system, 431)] {
        run(media_command(ffmpeg)
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency={frequency}:sample_rate=48000:duration=2.8"),
                "-c:a",
                "pcm_f32le",
            ])
            .arg(path));
    }
    let mut edits = EditSettings::new(2.8);
    edits.trim_start = 0.21;
    edits.trim_end = 2.58;
    edits.microphone_volume = 0.6;
    edits.system_volume = 0.5;
    edits.crop = Some(CropSettings {
        x: 0.1,
        y: 0.1,
        width: 0.8,
        height: 0.8,
    });
    edits.camera = CameraSettings {
        layout: "bubble".into(),
        enabled: true,
        x: 0.65,
        y: 0.5,
        width: 0.22,
        shape: "circle".into(),
        mirror: true,
    };
    edits.zooms = vec![ZoomSegment {
        id: "moving-zoom".into(),
        start: 0.2,
        end: 2.5,
        scale: 1.65,
        x: 0.5,
        y: 0.5,
        follow_cursor: true,
    }];
    Project {
        version: screenrecord_core::PROJECT_VERSION,
        id: "26bf72de-b0ae-4a96-999a-2dacaa468daf".into(),
        name: "Chunk continuity".into(),
        created_at: "2026-11-11T00:00:00Z".into(),
        directory: directory.to_string_lossy().into_owned(),
        screen_path: screen.to_string_lossy().into_owned(),
        microphone_path: Some(microphone.to_string_lossy().into_owned()),
        system_audio_path: Some(system.to_string_lossy().into_owned()),
        camera_path: Some(camera.to_string_lossy().into_owned()),
        camera_offset_seconds: 0.0,
        camera_intervals: None,
        width: 320,
        height: 180,
        fps: 30,
        duration: 2.8,
        cursor: (0..336)
            .map(|index| CursorSample {
                buttons: 0,
                time: index as f64 / 120.0,
                x: 0.3 + 0.4 * index as f64 / 336.0,
                y: 0.5,
                pressed: false,
                visible: true,
            })
            .collect(),
        edits,
        status: "ready".into(),
    }
}

fn options(directory: &Path, name: &str) -> ExportOptions {
    ExportOptions {
        preset: screenrecord_core::ExportPreset::Standard,
        output_path: directory.join(name).to_string_lossy().into_owned(),
        resolution: "1080p".into(),
        fps: 60,
    }
}

fn video_pixels(ffmpeg: &Path, path: &str) -> Vec<u8> {
    run(media_command(ffmpeg).args([
        "-i",
        path,
        "-map",
        "0:v:0",
        "-vf",
        "scale=96:54",
        "-pix_fmt",
        "rgb24",
        "-fps_mode",
        "passthrough",
        "-f",
        "rawvideo",
        "-",
    ]))
}

fn audio_samples(ffmpeg: &Path, path: &str) -> Vec<u8> {
    run(media_command(ffmpeg).args([
        "-i",
        path,
        "-map",
        "0:a:0",
        "-c:a",
        "pcm_f32le",
        "-f",
        "f32le",
        "-",
    ]))
}

#[test]
fn output_contract_rejects_video_truncation_hidden_by_audio_duration() {
    let mut info: MediaInfo = serde_json::from_value(serde_json::json!({
        "streams": [
            {"codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080, "avg_frame_rate": "60/1", "nb_frames": "2"},
            {"codec_type": "audio", "codec_name": "aac", "sample_rate": "48000", "channels": 2, "duration": "2.37", "start_time": "0"}
        ],
        "format": {"duration": "2.383333"}
    })).unwrap();
    let layout = math::Layout {
        width: 1920,
        height: 1080,
        frame_width: 1920,
        frame_height: 1080,
        frame_x: 0,
        frame_y: 0,
        reference_scale: 1.0,
        screen_visible: true,
        camera_pane: None,
    };
    assert!(validate_output(&info, &layout, 143, 60, 2.37, true).is_err());
    info.streams[0].nb_frames = Some("143".into());
    assert!(validate_output(&info, &layout, 143, 60, 2.37, true).is_ok());
    info.streams[0].avg_frame_rate = Some("30/1".into());
    assert!(validate_output(&info, &layout, 143, 60, 2.37, true).is_err());
    info.streams[0].avg_frame_rate = Some("60/1".into());
    info.streams.pop();
    assert!(validate_output(&info, &layout, 143, 60, 2.37, true).is_err());
    assert_eq!(graph::output_frame_count(0.4 - 0.1, 30), 9);
    assert_eq!(graph::output_frame_count(2.37, 60), 143);
}

#[test]
fn output_contract_rejects_unexpected_or_misaligned_audio() {
    let info: MediaInfo=serde_json::from_value(serde_json::json!({
        "streams":[
            {"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"avg_frame_rate":"30/1","nb_frames":"60"},
            {"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2,"duration":"2.0","start_time":"0"}
        ],"format":{"duration":"2.0"}
    })).unwrap();
    let layout = math::Layout {
        width: 1920,
        height: 1080,
        frame_width: 1920,
        frame_height: 1080,
        frame_x: 0,
        frame_y: 0,
        reference_scale: 1.0,
        screen_visible: true,
        camera_pane: None,
    };
    assert!(validate_output(&info, &layout, 60, 30, 2.0, true).is_ok());
    assert!(validate_output(&info, &layout, 60, 30, 2.0, false).is_err());
    for case in 0..6 {
        let mut changed = info.clone();
        match case {
            0 => changed.streams[1].channels = Some(1),
            1 => changed.streams[1].sample_rate = Some("44100".into()),
            2 => changed.streams[1].start_time = Some("0.25".into()),
            3 => changed.streams[1].duration = Some("NaN".into()),
            4 => changed.streams[1].duration = None,
            _ => changed.streams.push(changed.streams[1].clone()),
        }
        assert!(
            validate_output(&changed, &layout, 60, 30, 2.0, true).is_err(),
            "accepted audio contract case {case}"
        );
    }
}

#[test]
fn output_contract_rejects_real_audio_truncation_hidden_by_complete_video() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for output validation media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let project = fixture(&ffmpeg, directory.path());
    let output = directory.path().join("short-audio.mp4");
    run(media_command(&ffmpeg)
        .arg("-i")
        .arg(&project.screen_path)
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=997:sample_rate=48000:duration=0.1",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-ac",
            "2",
            "-t",
            "2",
        ])
        .arg(&output));
    let info = probe_media(&ffmpeg, &output).unwrap();
    let layout = math::Layout {
        width: 320,
        height: 180,
        frame_width: 320,
        frame_height: 180,
        frame_x: 0,
        frame_y: 0,
        reference_scale: 1.0,
        screen_visible: true,
        camera_pane: None,
    };
    assert_eq!(info.video().unwrap().nb_frames.as_deref(), Some("60"));
    let audio = info
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"))
        .unwrap();
    let audio_duration = audio.duration.as_ref().unwrap().parse::<f64>().unwrap();
    assert!(audio_duration < 0.2);
    assert!((info.duration().unwrap() - 2.0).abs() < 0.01);
    assert!(
        validate_output(&info, &layout, 60, 30, 2.0, true).is_err(),
        "Full video hid a truncated {audio_duration}s AAC track"
    );
}

#[test]
fn chunk_boundaries_preserve_video_cursor_webcam_and_continuous_audio() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for chunk media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let project = fixture(&ffmpeg, directory.path());
    let single = options(directory.path(), "single.mp4");
    let chunked = options(directory.path(), "chunked.mp4");
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    export_inner(
        &project,
        &single,
        &ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        graph::MAX_CHUNK_FRAMES,
    )
    .unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    let callback: process::Progress =
        Arc::new(Mutex::new(Box::new(move |event: ExportProgress| {
            observed.lock().unwrap().push(event.progress)
        })));
    export_inner(
        &project,
        &chunked,
        &ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        12,
    )
    .unwrap();
    let a = video_pixels(&ffmpeg, &single.output_path);
    let b = video_pixels(&ffmpeg, &chunked.output_path);
    assert_eq!(
        a.len(),
        b.len(),
        "A chunk boundary duplicated or dropped video frames."
    );
    let frame_bytes = 96 * 54 * 3;
    assert_eq!(a.len() / frame_bytes, 143);
    let mut largest_frame_error = 0.0_f64;
    for (index, (a, b)) in a
        .chunks_exact(frame_bytes)
        .zip(b.chunks_exact(frame_bytes))
        .enumerate()
    {
        let error = a
            .iter()
            .zip(b)
            .map(|(a, b)| (*a as f64 - *b as f64).abs())
            .sum::<f64>()
            / frame_bytes as f64;
        largest_frame_error = largest_frame_error.max(error);
        assert!(
            error < 3.0,
            "Frame {index} shifted at a chunk boundary; mean RGB error {error}"
        );
    }
    let decoded_audio = audio_samples(&ffmpeg, &single.output_path);
    assert_eq!(
        decoded_audio,
        audio_samples(&ffmpeg, &chunked.output_path),
        "Audio must be encoded once, independently of video chunking."
    );
    let reference_audio = run(media_command(&ffmpeg).args([
        "-ss",
        "0.21",
        "-i",
        project.microphone_path.as_ref().unwrap(),
        "-ss",
        "0.21",
        "-i",
        project.system_audio_path.as_ref().unwrap(),
        "-filter_complex",
        "[0:a]volume=0.6[m];[1:a]volume=0.5[s];[m][s]amix=inputs=2:normalize=0,atrim=duration=2.37",
        "-c:a",
        "pcm_f32le",
        "-f",
        "f32le",
        "-",
    ]));
    let reference_samples = reference_audio
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes) as f64);
    let output_samples = decoded_audio
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes) as f64);
    let (reference_power, output_power) = reference_samples
        .zip(output_samples)
        .fold((0.0, 0.0), |(a, b), (x, y)| (a + x * x, b + y * y));
    let gain = (output_power / reference_power).sqrt();
    assert!(
        (gain - 1.0).abs() < 0.01,
        "Unexpected limiter makeup gain: {gain}"
    );
    let events = events.lock().unwrap();
    assert!(events.windows(2).all(|pair| pair[1] >= pair[0]));
    let probe = Command::new(probe::ffprobe_path(&ffmpeg))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_frames",
            "-show_entries",
            "frame=key_frame,best_effort_timestamp_time",
            "-of",
            "json",
            &chunked.output_path,
        ])
        .output()
        .unwrap();
    assert!(probe.status.success());
    let frames: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    for (index, frame) in frames["frames"].as_array().unwrap().iter().enumerate() {
        let time = frame["best_effort_timestamp_time"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!((time - index as f64 / 60.0).abs() < 0.0001);
        if index % 12 == 0 {
            assert_eq!(frame["key_frame"], 1);
        }
    }
    eprintln!("chunk continuity: 143 frames, 12 chunks, exact decoded AAC equality, maximum mean RGB frame error={largest_frame_error:.4}");
}

#[test]
fn webcam_offset_selects_the_correct_source_time_and_hides_at_eof() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for chunk media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let camera = directory.path().join("offset-camera.mp4");
    let encoder = process::software_encoder(
        &ffmpeg,
        process::VideoSpec {
            width: 160,
            height: 120,
            fps: 30,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=160x120:r=30:d=1",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=160x120:r=30:d=1.8",
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0[v]",
            "-map",
            "[v]",
        ])
        .args(encoder.args())
        .arg(&camera));
    project.camera_path = Some(camera.to_string_lossy().into_owned());
    project.camera_offset_seconds = 1.6;
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits.trim_start = 0.5;
    project.edits.trim_end = 2.5;
    project.edits.crop = None;
    project.edits.zooms.clear();
    project.edits.show_cursor = false;
    project.edits.padding = 180;
    project.edits.background = "#00ff00".into();
    project.edits.camera = CameraSettings {
        layout: "bubble".into(),
        enabled: true,
        x: 0.8,
        y: 0.8,
        width: 0.2,
        shape: "rectangle".into(),
        mirror: false,
    };
    let options = options(directory.path(), "offset.mp4");
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    export_inner(
        &project,
        &options,
        &ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        12,
    )
    .unwrap();
    let pixel = |time: &str| {
        run(media_command(&ffmpeg).args([
            "-ss",
            time,
            "-i",
            &options.output_path,
            "-vf",
            "format=rgb24,crop=1:1:1850:1000:exact=1",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-",
        ]))
    };
    let blue = pixel("0.1");
    assert!(
        blue.len() >= 3 && blue[2] > 180 && blue[0] < 60,
        "Camera offset was ignored: {blue:?}"
    );
    let background = pixel("1.2");
    assert!(
        background.len() >= 3 && background[1] > 180 && background[2] < 60,
        "Ended camera was repeated: {background:?}"
    );
}

#[test]
fn sparse_camera_frames_are_held_until_the_next_real_frame_across_chunks() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for chunk media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let camera = directory.path().join("sparse-camera.mp4");
    run(media_command(&ffmpeg).args([
        "-f", "lavfi", "-i", "color=c=red:s=160x120:r=30:d=2.8,drawbox=color=blue:t=fill:enable='gte(n,75)',select='eq(n,0)+eq(n,75)+eq(n,83)'",
        "-fps_mode", "vfr", "-c:v", "mpeg4", "-q:v", "2",
    ]).arg(&camera));
    project.camera_path = Some(camera.to_string_lossy().into_owned());
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits.trim_start = 1.7;
    project.edits.trim_end = 2.7;
    project.edits.crop = None;
    project.edits.zooms.clear();
    project.edits.camera = CameraSettings {
        layout: "bubble".into(),
        enabled: true,
        x: 0.7,
        y: 0.7,
        width: 0.2,
        shape: "rectangle".into(),
        mirror: false,
    };
    let options = options(directory.path(), "sparse.mp4");
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    export_inner(
        &project,
        &options,
        &ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        12,
    )
    .unwrap();
    let pixel = |time: &str| {
        run(media_command(&ffmpeg).args([
            "-ss",
            time,
            "-i",
            &options.output_path,
            "-vf",
            "format=rgb24,crop=1:1:1536:900:exact=1",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-",
        ]))
    };
    let red = pixel("0.6");
    assert!(
        red.len() >= 3 && red[0] > 180 && red[2] < 60,
        "Sparse input advanced before its source timestamp: {red:?}"
    );
    let blue = pixel("0.9");
    assert!(
        blue.len() >= 3 && blue[2] > 180 && blue[0] < 60,
        "Sparse input failed to advance at its source timestamp: {blue:?}"
    );
}

#[test]
fn software_encoder_chunks_mux_with_the_requested_frame_and_audio_contract() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for chunk media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let project = fixture(&ffmpeg, directory.path());
    let options = options(directory.path(), "unused-publication.mp4");
    let cancel = Arc::new(AtomicBool::new(false));
    let graph = graph::RenderGraph::prepare(&project, &options, &ffmpeg, directory.path(), &cancel)
        .unwrap();
    let encoder = process::software_encoder(
        &ffmpeg,
        process::VideoSpec {
            width: graph.layout.width,
            height: graph.layout.height,
            fps: options.fps,
        },
        &cancel,
    )
    .unwrap();
    assert!(encoder.is_software());
    let chunks = graph.chunks(72).unwrap();
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    for chunk in &chunks {
        process::encode(
            &mut graph
                .video_command(&project, &options, &ffmpeg, encoder, *chunk, &cancel)
                .unwrap(),
            &cancel,
            &callback,
            chunk.frames as f64 / options.fps as f64,
            &options.output_path,
            0.0..0.95,
        )
        .unwrap();
    }
    for chunk in graph.audio_chunks() {
        process::encode(
            &mut graph.audio_command(&ffmpeg, chunk),
            &cancel,
            &callback,
            chunk.samples as f64 / 48_000.0,
            &options.output_path,
            0.8..0.95,
        )
        .unwrap();
    }
    process::encode(
        &mut graph
            .mux_command(&project, &options, &ffmpeg, &chunks)
            .unwrap(),
        &cancel,
        &callback,
        graph.duration,
        &options.output_path,
        0.95..0.995,
    )
    .unwrap();
    let output = directory.path().join("encoded.mp4");
    let info = probe_media(&ffmpeg, &output).unwrap();
    validate_output(
        &info,
        &graph.layout,
        graph.frame_count,
        options.fps,
        graph.duration,
        true,
    )
    .unwrap();
    run(media_command(&ffmpeg)
        .arg("-i")
        .arg(output)
        .args(["-f", "null", "-"]));
}

#[test]
fn cancellation_after_a_completed_chunk_removes_all_intermediate_files() {
    let Some(ffmpeg) = engine() else {
        eprintln!("SKIPPED: set SCREENRECORD_FFMPEG for chunk media tests.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.edits.fit_mode = "fill".into();
    project.edits.aspect_ratio = "9:16".into();
    project.edits.background_mode = "videoBlur".into();
    project.edits.reframe_keyframes = (0..2800)
        .map(|index| screenrecord_core::ReframeKeyframe {
            id: format!("reframe-{index}"),
            time: index as f64 / 1000.0,
            x: index as f64 / 3000.0,
            y: 0.5,
            scale: 1.0 + index as f64 / 3000.0,
        })
        .collect();
    let options = options(directory.path(), "cancelled.mp4");
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let callback: process::Progress =
        Arc::new(Mutex::new(Box::new(move |event: ExportProgress| {
            if event.progress > 0.12 {
                trigger.store(true, Ordering::Release);
            }
        })));
    assert!(
        export_inner(&project, &options, &ffmpeg, &cancel, &callback, 12)
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(!Path::new(&options.output_path).exists());
    assert!(!fs::read_dir(directory.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".screenrecord-export-")));
}

#[test]
#[ignore = "Measured FFmpeg scheduling benchmark; run explicitly with SCREENRECORD_FFMPEG."]
fn measure_bounded_motion_scheduler_scaling() {
    let ffmpeg = engine().expect("SCREENRECORD_FFMPEG");
    let directory = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    for frames in [600_u64, 2400, 9600, 38400] {
        let mut times = Vec::new();
        for chunk_limit in [frames, graph::MAX_CHUNK_FRAMES] {
            let started = Instant::now();
            let mut count = 0;
            for first in (0..frames).step_by(chunk_limit as usize) {
                let count_frames = chunk_limit.min(frames - first);
                let mut text = String::new();
                for frame in 0..count_frames {
                    let timestamp = (frame as f64 / 60.0 * 1_000_000.0).floor() / 1_000_000.0;
                    text.push_str(&format!(
                        "{timestamp:.6} crop@view x {};\n",
                        (first + frame) % 64
                    ));
                }
                fs::write(directory.path().join("motion.txt"), text).unwrap();
                let mut command = media_command(&ffmpeg);
                command.current_dir(directory.path()).args([
                    "-f",
                    "lavfi",
                    "-i",
                    &format!(
                        "color=c=red:s=128x128:r=60:d={:.9}",
                        count_frames as f64 / 60.0
                    ),
                    "-vf",
                    "sendcmd=f=motion.txt,crop@view=64:64:0:0",
                    "-frames:v",
                    &count_frames.to_string(),
                    "-f",
                    "null",
                    "-",
                ]);
                let (success, _, errors) =
                    process::capture(&mut command, Duration::from_secs(120), &cancel).unwrap();
                assert!(success, "{errors}");
                count += 1;
            }
            times.push((started.elapsed().as_secs_f64(), count));
        }
        eprintln!("motion scheduling: frames={frames}, legacy_seconds={:.4}, bounded_seconds={:.4}, bounded_chunks={}", times[0].0, times[1].0, times[1].1);
    }
}

#[test]
#[ignore = "Full 1080p export scaling measurement; run explicitly with SCREENRECORD_FFMPEG."]
fn measure_full_export_scaling_and_motion_bounds() {
    let ffmpeg = engine().expect("SCREENRECORD_FFMPEG");
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let screen = directory.path().join("long-screen.mp4");
    let software = process::software_encoder(
        &ffmpeg,
        process::VideoSpec {
            width: 320,
            height: 180,
            fps: 30,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=60",
        ])
        .args(software.args())
        .arg(&screen));
    project.screen_path = screen.to_string_lossy().into_owned();
    project.duration = 60.0;
    project.microphone_path = None;
    project.system_audio_path = None;
    project.camera_path = None;
    project.edits = EditSettings::new(60.0);
    project.edits.zooms = vec![ZoomSegment {
        id: "benchmark-zoom".into(),
        start: 0.0,
        end: 60.0,
        scale: 1.65,
        x: 0.5,
        y: 0.5,
        follow_cursor: true,
    }];
    project.cursor = (0..7200)
        .map(|index| CursorSample {
            buttons: 0,
            time: index as f64 / 120.0,
            x: 0.5 + 0.25 * (index as f64 / 60.0).sin(),
            y: 0.5,
            pressed: false,
            visible: true,
        })
        .collect();
    for duration in [3.0, 15.0, 60.0] {
        project.edits.trim_end = duration;
        let options = options(directory.path(), &format!("benchmark-{duration}.mp4"));
        let maximum = Arc::new(Mutex::new((0usize, 0usize)));
        let observed = maximum.clone();
        let parent = directory.path().to_owned();
        let callback: process::Progress = Arc::new(Mutex::new(Box::new(move |_| {
            for entry in fs::read_dir(&parent).unwrap().flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".screenrecord-export-")
                {
                    if let Ok(script) = fs::read_to_string(entry.path().join("motion.txt")) {
                        let mut maximum = observed.lock().unwrap();
                        maximum.0 = maximum.0.max(script.lines().count());
                        maximum.1 = maximum.1.max(script.len());
                    }
                }
            }
        })));
        let start = Instant::now();
        export_inner(
            &project,
            &options,
            &ffmpeg,
            &Arc::new(AtomicBool::new(false)),
            &callback,
            graph::MAX_CHUNK_FRAMES,
        )
        .unwrap();
        let seconds = start.elapsed().as_secs_f64();
        let maximum = maximum.lock().unwrap();
        assert!(maximum.0 <= graph::MAX_CHUNK_FRAMES as usize);
        assert!(maximum.1 <= 256 * 1024);
        let info = probe_media(&ffmpeg, Path::new(&options.output_path)).unwrap();
        assert!((info.duration().unwrap() - duration).abs() < 1.0 / 60.0);
        run(media_command(&ffmpeg).args(["-i", &options.output_path, "-f", "null", "-"]));
        eprintln!("full export: duration={duration:.0}s, wall={seconds:.4}s, dimensions=1920x1080, fps=60, max_intervals={}, max_motion_bytes={}", maximum.0, maximum.1);
    }
}
