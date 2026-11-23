use screenrecord_capture::{list_devices, recover, Recorder};
use screenrecord_core::RecordingOptions;
use std::{path::PathBuf, thread, time::Duration};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ffmpeg = std::env::var_os("SCREENRECORD_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| "ffmpeg.exe".into());
    if args.first().map(String::as_str) == Some("recover") && args.len() == 2 {
        let result = recover(&PathBuf::from(&args[1]), &ffmpeg)?;
        println!(
            "{}",
            serde_json::json!({"screen":result.screen_path,"duration":result.duration,"cursorSamples":result.cursor.len(),"camera":result.camera_path})
        );
        return Ok(());
    }
    let devices = list_devices(&ffmpeg)?;
    if args.first().map(String::as_str) == Some("list") || args.is_empty() {
        println!("{}", serde_json::to_string_pretty(&devices).unwrap());
        return Ok(());
    }
    if args[0] != "record" || args.len() < 2 {
        return Err("Usage: capture-smoke list | record <empty-output-directory> [seconds] [--audio] [--camera]".into());
    }
    let duration: u64 = args
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(4)
        .clamp(1, 30);
    let kind = if args.iter().any(|arg| arg == "--window") {
        "window"
    } else {
        "monitor"
    };
    let source = devices
        .sources
        .iter()
        .find(|source| source.kind == kind)
        .ok_or("No capture source found.")?;
    let options = RecordingOptions {
        source_id: source.id.clone(),
        fps: if args.iter().any(|arg| arg == "--60") {
            60
        } else {
            30
        },
        microphone_id: if args.iter().any(|arg| arg == "--audio") {
            devices.microphones.last().map(|device| device.id.clone())
        } else {
            None
        },
        system_audio: args.iter().any(|arg| arg == "--audio"),
        system_audio_scope: Default::default(),
        region: None,
        camera_id: if args.iter().any(|arg| arg == "--camera") {
            devices.cameras.first().map(|device| device.id.clone())
        } else {
            None
        },
    };
    let recorder = Recorder::start(&PathBuf::from(&args[1]), &options, &ffmpeg)?;
    for _ in 0..duration {
        thread::sleep(Duration::from_secs(1));
        recorder.health()?;
    }
    let result = recorder.stop()?;
    println!(
        "{}",
        serde_json::json!({"screen":result.screen_path,"microphone":result.microphone_path,"systemAudio":result.system_audio_path,"camera":result.camera_path,"width":result.width,"height":result.height,"duration":result.duration,"cursorSamples":result.cursor.len()})
    );
    Ok(())
}
