use screenrecord_transcribe::{local, TranscriptionInput};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() < 2 {
        return Err("Usage: local_smoke install|status <absolute-runtime-directory> OR local_smoke transcribe <directory> <audio> <ffmpeg> <duration> <json-output>".into());
    }
    let root = PathBuf::from(&arguments[1]);
    let cancel = Arc::new(AtomicBool::new(false));
    match arguments[0].to_string_lossy().as_ref() {
        "install" => {
            let result = local::install(root, cancel, |value| {
                eprintln!("Setup {:.0}%", value * 100.0)
            })
            .await?;
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        }
        "status" => println!(
            "{}",
            serde_json::to_string_pretty(&local::status(&root)?).unwrap()
        ),
        "transcribe" if arguments.len() == 6 => {
            let input = TranscriptionInput {
                audio_path: PathBuf::from(&arguments[2]),
                ffmpeg_path: PathBuf::from(&arguments[3]),
                start: 0.0,
                end: arguments[4]
                    .to_string_lossy()
                    .parse()
                    .map_err(|_| "Invalid duration")?,
                ranges: vec![],
                language: "en".into(),
                api_key: String::new(),
            };
            let result = local::transcribe(input, root, cancel, |value| {
                eprintln!("Transcription {:.0}%", value * 100.0)
            })
            .await?;
            std::fs::write(
                PathBuf::from(&arguments[5]),
                serde_json::to_vec_pretty(&result).unwrap(),
            )
            .map_err(|error| error.to_string())?;
            println!("{} local cues written", result.len());
        }
        _ => return Err("Unsupported local smoke command.".into()),
    }
    Ok(())
}
