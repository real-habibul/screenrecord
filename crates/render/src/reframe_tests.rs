use super::*;
use screenrecord_core::{ReframeKeyframe, VideoClip};

fn render(ffmpeg: &Path, project: &Project, name: &str, chunk_frames: u64) -> ExportOptions {
    let mut output = options(Path::new(&project.directory), name);
    output.fps = 30;
    let callback: process::Progress = Arc::new(Mutex::new(Box::new(|_| {})));
    export_inner(
        project,
        &output,
        ffmpeg,
        &Arc::new(AtomicBool::new(false)),
        &callback,
        chunk_frames,
    )
    .unwrap();
    output
}

fn uncompressed_reference(ffmpeg: &Path, project: &Project, options: &ExportOptions) -> Vec<u8> {
    let directory = Path::new(&project.directory).join("uncompressed-geometry");
    fs::create_dir(&directory).unwrap();
    let cancel = AtomicBool::new(false);
    let graph = graph::RenderGraph::prepare(project, options, ffmpeg, &directory, &cancel).unwrap();
    let mut result = Vec::new();
    for chunk in graph.chunks(900).unwrap() {
        let command = graph
            .video_command(
                project,
                options,
                ffmpeg,
                process::Encoder::SoftwareMf,
                chunk,
                &cancel,
            )
            .unwrap();
        let path = directory.join("filters.txt");
        let filters = fs::read_to_string(&path).unwrap();
        let (composition, _) = filters
            .rsplit_once("format=rgba,scale=in_range=pc:out_range=tv:out_color_matrix=bt709")
            .expect("Explicit final output color conversion");
        let filters = format!("{composition}scale=96:54,format=rgb24[outv]");
        fs::write(path, filters).unwrap();
        let mut raw = Command::new(ffmpeg);
        raw.current_dir(&directory)
            .args(
                command
                    .get_args()
                    .take_while(|arg| *arg != std::ffi::OsStr::new("-an")),
            )
            .args([
                "-an",
                "-frames:v",
                &chunk.frames.to_string(),
                "-fps_mode",
                "passthrough",
                "-c:v",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "-",
            ]);
        result.extend(run(&mut raw));
    }
    result
}

#[test]
fn moving_reframe_uses_source_time_and_combined_scale_across_cuts_and_chunks() {
    let Some(ffmpeg) = engine() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut project = fixture(&ffmpeg, directory.path());
    let screen = directory.path().join("coordinate-gradient.mkv");
    run(media_command(&ffmpeg)
        .args([
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=320x180:r=30:d=2.8,format=gbrp,geq=r='X*255/320':g='Y*255/180':b=0",
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "gbrp",
        ])
        .arg(&screen));
    project.screen_path = screen.to_string_lossy().into_owned();
    let originals = [
        &project.screen_path,
        project.microphone_path.as_ref().unwrap(),
        project.system_audio_path.as_ref().unwrap(),
    ]
    .map(|path| (path.clone(), fs::read(path).unwrap()));
    project.edits.aspect_ratio = "9:16".into();
    project.edits.fit_mode = "fill".into();
    project.edits.padding = 0;
    project.edits.corner_radius = 0;
    project.edits.camera.enabled = false;
    project.edits.show_cursor = false;
    project.edits.zooms = vec![ZoomSegment {
        id: "cursor-follow-must-not-own-center".into(),
        start: 0.0,
        end: 2.5,
        scale: 1.5,
        x: 0.95,
        y: 0.95,
        follow_cursor: true,
    }];
    project.cursor.iter_mut().for_each(|sample| {
        sample.x = 0.95;
        sample.y = 0.95;
    });
    project.edits.reframe_keyframes = [(0.0, 0.3, 0.4, 1.25), (2.0, 0.7, 0.6, 1.75)]
        .into_iter()
        .enumerate()
        .map(|(index, (time, x, y, scale))| ReframeKeyframe {
            id: format!("key-{index}"),
            time,
            x,
            y,
            scale,
        })
        .collect();
    project.edits.video_clips = [(0.417, 0.917), (1.217, 1.717)]
        .into_iter()
        .enumerate()
        .map(|(index, (start, end))| VideoClip {
            id: format!("kept-{index}"),
            start,
            end,
        })
        .collect();
    let whole = render(&ffmpeg, &project, "moving-whole.mp4", 900);
    let chunked = render(&ffmpeg, &project, "moving-chunked.mp4", 7);
    let pixels = video_pixels(&ffmpeg, &whole.output_path);
    let chunk_pixels = video_pixels(&ffmpeg, &chunked.output_path);
    let reference = uncompressed_reference(&ffmpeg, &project, &whole);
    assert_eq!(pixels.len(), 30 * 96 * 54 * 3);
    assert_eq!(pixels.len(), chunk_pixels.len());
    assert_eq!(pixels.len(), reference.len());
    for (name, decoded) in [("whole", &pixels), ("chunked", &chunk_pixels)] {
        let error = decoded
            .iter()
            .zip(&reference)
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / decoded.len() as f64;
        let maximum = decoded
            .iter()
            .zip(&reference)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        let maximum_frame_error = decoded
            .as_chunks::<{ 96 * 54 * 3 }>()
            .0
            .iter()
            .zip(reference.as_chunks::<{ 96 * 54 * 3 }>().0)
            .map(|(actual, expected)| {
                actual
                    .iter()
                    .zip(expected)
                    .map(|(a, b)| a.abs_diff(*b) as f64)
                    .sum::<f64>()
                    / actual.len() as f64
            })
            .fold(0.0_f64, f64::max);
        eprintln!("Reframe {name} H264 vs RGB composition: MAE {error:.3}, maxframeMAE {maximum_frame_error:.3}, maxchannelerror {maximum}");
        assert!(
            maximum_frame_error < 2.0,
            "{name} encoded frame changed geometry or exceeded chroma/codec mean-error budget"
        );
        assert!(
            maximum <= 8,
            "{name} exceeded the8-level chroma/codec maximum-error budget: {maximum}"
        );
    }
    let difference = pixels
        .iter()
        .zip(&chunk_pixels)
        .map(|(a, b)| a.abs_diff(*b) as f64)
        .sum::<f64>()
        / pixels.len() as f64;
    assert!(
        difference < 2.0,
        "Chunking changed reframing: MAE={difference}"
    );
    {
        let name = "RGB composition before YUV420";
        let rendered = &reference;
        let mut maximum_error = 0.0_f64;
        for (index, frame) in rendered.as_chunks::<{ 96 * 54 * 3 }>().0.iter().enumerate() {
            let time = if index < 15 {
                0.417 + index as f64 / 30.0
            } else {
                1.217 + (index - 15) as f64 / 30.0
            };
            // RGB encodes original source coordinates, independent of the render viewport helper.
            let center_x = (0.3 + 0.2 * time) * 320.0;
            let center_y = (0.4 + 0.1 * time) * 180.0;
            let scale = (1.25 + 0.25 * time) * 1.5;
            let view_width = ((80.0 / scale).round() / 2.0).floor() * 2.0;
            let view_height = ((144.0 / scale).round() / 2.0).floor() * 2.0;
            for (x, y) in [(24, 13), (48, 27), (72, 40)] {
                let actual = &frame[(y * 96 + x) * 3..][..3];
                let source_x = center_x + ((x as f64 + 0.5) / 96.0 - 0.5) * view_width;
                let source_y = center_y + ((y as f64 + 0.5) / 54.0 - 0.5) * view_height;
                let expected = [source_x * 255.0 / 320.0, source_y * 255.0 / 180.0, 0.0];
                for (actual, expected) in actual.iter().zip(expected) {
                    maximum_error = maximum_error.max((*actual as f64 - expected).abs());
                }
                assert!(
                    actual.iter().zip(expected).all(|(a, e)| (*a as f64 - e).abs() <= 5.0),
                    "{name} frame {index} source {time:.6} at ({x},{y}): {actual:?} != {expected:?}"
                );
            }
        }
        eprintln!("Reframe {name}: 30 frames checked, maximum RGB error {maximum_error:.3}");
    }
    eprintln!("Reframe chunk comparison MAE: {difference:.3}");
    assert_eq!(
        audio_samples(&ffmpeg, &whole.output_path),
        audio_samples(&ffmpeg, &chunked.output_path)
    );
    for (path, original) in originals {
        assert_eq!(fs::read(path).unwrap(), original);
    }
}
