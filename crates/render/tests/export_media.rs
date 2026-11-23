use screenrecord_core::{
    CameraSettings, CropSettings, CursorSample, EditSettings, ExportOptions, Project, ZoomSegment,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

fn ffmpeg() -> Option<PathBuf> {
    std::env::var_os("SCREENRECORD_FFMPEG")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .map(|p| {
                    p.join(if cfg!(windows) {
                        "ffmpeg.exe"
                    } else {
                        "ffmpeg"
                    })
                })
                .find(|p| p.is_file())
        })
        .and_then(|path| path.canonicalize().ok())
}

fn command(ffmpeg: &Path) -> Command {
    let mut command = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.args(["-hide_banner", "-v", "error", "-nostdin", "-y"]);
    command
}

fn run(command: &mut Command) {
    let result = command.output().expect("FFmpeg fixture process");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn fixture(ffmpeg: &Path, directory: &Path) -> Project {
    let screen = directory.join("screen video.mp4");
    let camera = directory.join("camera.mp4");
    let audio = directory.join("audio.wav");
    let encoder = if cfg!(windows) {
        vec!["-c:v", "h264_mf", "-hw_encoding", "0"]
    } else {
        vec!["-c:v", "libx264", "-preset", "ultrafast"]
    };
    run(command(ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=2",
        ])
        .args(&encoder)
        .args(["-pix_fmt", "yuv420p"])
        .arg(&screen));
    run(command(ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:size=160x120:rate=30:duration=2",
        ])
        .args(&encoder)
        .args(["-pix_fmt", "yuv420p"])
        .arg(&camera));
    run(command(ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&audio));
    let cursor = (0..120)
        .map(|i| CursorSample {
            buttons: 0,
            time: i as f64 / 60.0,
            x: 0.2 + 0.6 * i as f64 / 120.0,
            y: 0.45,
            visible: true,
            pressed: i == 30 || i == 60,
        })
        .collect();
    let mut edits = EditSettings::new(2.0);
    edits.trim_start = 0.2;
    edits.trim_end = 1.8;
    edits.background = "#121212".into();
    edits.padding = 60;
    edits.crop = Some(CropSettings {
        x: 0.1,
        y: 0.1,
        width: 0.8,
        height: 0.8,
    });
    edits.camera = CameraSettings {
        layout: "bubble".into(),
        enabled: true,
        x: 0.7,
        y: 0.7,
        width: 0.2,
        shape: "circle".into(),
        mirror: true,
    };
    edits.zooms.push(ZoomSegment {
        id: "test-zoom".into(),
        start: 0.3,
        end: 1.6,
        scale: 1.8,
        x: 0.5,
        y: 0.5,
        follow_cursor: true,
    });
    Project {
        version: screenrecord_core::PROJECT_VERSION,
        id: "9ae9d6a5-c147-4d09-9b85-e203e385004d".into(),
        name: "Generated media integration test".into(),
        created_at: "2026-11-11T00:00:00Z".into(),
        directory: directory.to_string_lossy().into_owned(),
        screen_path: screen.to_string_lossy().into_owned(),
        camera_path: Some(camera.to_string_lossy().into_owned()),
        camera_offset_seconds: 0.0,
        camera_intervals: None,
        microphone_path: Some(audio.to_string_lossy().into_owned()),
        system_audio_path: Some(audio.to_string_lossy().into_owned()),
        width: 320,
        height: 240,
        fps: 30,
        duration: 2.0,
        cursor,
        edits,
        status: "ready".into(),
    }
}

#[test]
fn exports_real_h264_with_camera_audio_and_trim() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("SKIPPED: FFmpeg is not installed.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let project = fixture(&ffmpeg, directory.path());
    let output = directory.path().join("result.mp4");
    let options = ExportOptions {
        preset: screenrecord_core::ExportPreset::Standard,
        output_path: output.to_string_lossy().into_owned(),
        resolution: "1080p".into(),
        fps: 30,
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    screenrecord_render::export(
        &project,
        &options,
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        move |event| observed.lock().unwrap().push(event),
    )
    .unwrap();
    let info = screenrecord_render::probe_media(&ffmpeg, &output).unwrap();
    let video = info.video().unwrap();
    assert_eq!(video.codec_name.as_deref(), Some("h264"));
    assert_eq!((video.width, video.height), (Some(1440), Some(1080)));
    assert!(info.has_audio());
    assert!((info.duration().unwrap() - 1.6).abs() < 0.15);
    assert_eq!(events.lock().unwrap().last().unwrap().state, "completed");
    let pixel = command(&ffmpeg)
        .args(["-ss", "0.8", "-i"])
        .arg(&output)
        .args([
            "-vf",
            "format=rgb24,crop=1:1:1152:900:exact=1",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        pixel.status.success(),
        "{}",
        String::from_utf8_lossy(&pixel.stderr)
    );
    assert!(
        pixel.stdout.len() >= 3 && pixel.stdout[2] > 180 && pixel.stdout[0] < 60,
        "Webcam blue pixel missing: {:?}",
        pixel.stdout
    );
    let original_length = std::fs::metadata(&output).unwrap().len();
    assert!(screenrecord_render::export(
        &project,
        &options,
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {}
    )
    .is_err());
    assert_eq!(std::fs::metadata(&output).unwrap().len(), original_length);
    assert!(!std::fs::read_dir(directory.path())
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".screenrecord-export-")));
}

#[test]
fn cancellation_never_publishes_partial_output() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("SKIPPED: FFmpeg is not installed.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let project = fixture(&ffmpeg, directory.path());
    let output = directory.path().join("cancelled.mp4");
    let cancel = Arc::new(AtomicBool::new(false));
    let requested = cancel.clone();
    let result = screenrecord_render::export(
        &project,
        &ExportOptions {
            preset: screenrecord_core::ExportPreset::Standard,
            output_path: output.to_string_lossy().into_owned(),
            resolution: "1080p".into(),
            fps: 30,
        },
        &ffmpeg,
        cancel,
        move |event| {
            if event.state == "running" && event.progress > 0.05 {
                requested.store(true, Ordering::Relaxed);
            }
        },
    );
    assert!(result.is_err());
    assert!(!output.exists());
    assert!(!std::fs::read_dir(directory.path())
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".screenrecord-export-")));
}

#[test]
fn dynamic_zoom_changes_source_region_and_cursor_is_redrawn() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("SKIPPED: FFmpeg is not installed.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let encoder = if cfg!(windows) {
        vec!["-c:v", "h264_mf", "-hw_encoding", "0"]
    } else {
        vec!["-c:v", "libx264"]
    };
    run(command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x240:r=30:d=2,drawbox=x=160:y=0:w=160:h=240:color=blue:t=fill",
        ])
        .args(encoder)
        .arg(&project.screen_path));
    project.camera_path = None;
    project.microphone_path = None;
    project.system_audio_path = None;
    project.edits = EditSettings::new(2.0);
    project.edits.padding = 0;
    project.edits.corner_radius = 0;
    project.edits.cursor_smoothing = 0.0;
    project.edits.cursor_size = 2.0;
    project.cursor = vec![CursorSample {
        buttons: 0,
        time: 0.0,
        x: 0.75,
        y: 0.5,
        pressed: false,
        visible: true,
    }];
    project.edits.zooms = vec![ZoomSegment {
        id: "right".into(),
        start: 0.2,
        end: 1.8,
        scale: 2.0,
        x: 0.9,
        y: 0.5,
        follow_cursor: false,
    }];
    let output = directory.path().join("motion.mp4");
    screenrecord_render::export(
        &project,
        &ExportOptions {
            preset: screenrecord_core::ExportPreset::Standard,
            output_path: output.to_string_lossy().into_owned(),
            resolution: "1080p".into(),
            fps: 60,
        },
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let pixel = |time: &str, x: u32, y: u32| {
        let result = command(&ffmpeg)
            .args(["-ss", time, "-i"])
            .arg(&output)
            .args([
                "-vf",
                &format!("format=rgb24,crop=1:1:{x}:{y}:exact=1"),
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-",
            ])
            .output()
            .unwrap();
        assert!(result.status.success());
        result.stdout
    };
    let before = pixel("0.0", 100, 500);
    let after = pixel("1.0", 100, 500);
    assert!(
        before[0] > 180 && before[2] < 60,
        "Initial red region missing: {before:?}"
    );
    assert!(
        after[2] > 180 && after[0] < 60,
        "Zoom failed to pan to blue region: {after:?}"
    );
    let cursor = pixel("1.0", 728, 555);
    assert!(
        cursor[0] > 180 && cursor[1] > 180 && cursor[2] > 180,
        "Redrawn pointer missing: {cursor:?}"
    );
}

#[test]
fn portrait_and_square_exports_have_exact_dimensions() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("SKIPPED: FFmpeg is not installed.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    project.edits.trim_end = 0.7;
    for (aspect, width, height) in [("9:16", 1080, 1920), ("1:1", 1080, 1080)] {
        project.edits.aspect_ratio = aspect.into();
        let output = directory.path().join(format!("{width}-{height}.mp4"));
        screenrecord_render::export(
            &project,
            &ExportOptions {
                preset: screenrecord_core::ExportPreset::Standard,
                output_path: output.to_string_lossy().into_owned(),
                resolution: "1080p".into(),
                fps: 30,
            },
            &ffmpeg,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        let info = screenrecord_render::probe_media(&ffmpeg, &output).unwrap();
        assert_eq!(
            (info.video().unwrap().width, info.video().unwrap().height),
            (Some(width), Some(height))
        );
        assert!((info.duration().unwrap() - 0.5).abs() < 0.1);
    }
}

#[test]
fn output_created_during_export_is_never_overwritten() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("SKIPPED: FFmpeg is not installed.");
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let project = fixture(&ffmpeg, directory.path());
    let output = directory.path().join("already-created.mp4");
    let competing_path = output.clone();
    let created = Arc::new(AtomicBool::new(false));
    let competing_created = created.clone();
    let result = screenrecord_render::export(
        &project,
        &ExportOptions {
            preset: screenrecord_core::ExportPreset::Standard,
            output_path: output.to_string_lossy().into_owned(),
            resolution: "1080p".into(),
            fps: 30,
        },
        &ffmpeg,
        Arc::new(AtomicBool::new(false)),
        move |event| {
            if event.state == "running"
                && event.progress > 0.0
                && !competing_created.swap(true, Ordering::SeqCst)
            {
                std::fs::write(&competing_path, b"existing file belongs to the user").unwrap();
            }
        },
    );
    assert!(created.load(Ordering::SeqCst));
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(&output).unwrap(),
        b"existing file belongs to the user"
    );
}
