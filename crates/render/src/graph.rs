use crate::{
    assets, audio, background, clicks, ducking, effects, envelopes,
    math::{self, Layout},
    probe,
    process::{self, Encoder},
    subtitles, timeline,
};
use screenrecord_core::{
    CursorSample, ExportOptions, ExportPreset, KeptInterval, Project, WhatsappExportPlan,
};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) const MAX_CHUNK_FRAMES: u64 = 900;
const MAX_CHUNKS: u64 = 2048;
const OUTPUT_COLOR_FILTER: &str = "format=rgba,scale=in_range=pc:out_range=tv:out_color_matrix=bt709,format=yuv420p,setparams=range=limited:color_primaries=bt709:color_trc=bt709:colorspace=bt709";

#[derive(Clone, Copy, Debug)]
pub(crate) struct VideoChunk {
    pub index: usize,
    pub first_frame: u64,
    pub frames: u64,
    pub source_start: f64,
    source_end: f64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AudioChunk {
    pub index: usize,
    pub first_sample: u64,
    pub samples: u64,
    source_start: f64,
    source_end: f64,
}

pub(crate) struct RenderGraph {
    inputs: Vec<Input>,
    pub(crate) duration: f64,
    pub(crate) layout: Layout,
    pub(crate) frame_count: u64,
    audio: Vec<audio::AudioTrack>,
    ducking: Option<screenrecord_core::DuckingSettings>,
    envelopes: Option<envelopes::Envelopes>,
    boundaries: Vec<u64>,
    cursor: Vec<CursorSample>,
    video_filters: String,
    video_filters_without_camera: String,
    directory: PathBuf,
    intervals: Vec<KeptInterval>,
    fps: u32,
    camera_start: f64,
    camera_duration: Option<f64>,
    mask_patches: Vec<(u32, u32)>,
    whatsapp: Option<WhatsappExportPlan>,
    clicks: clicks::Clicks,
}
struct Input {
    path: PathBuf,
    image: bool,
    offset_seconds: f64,
}

impl RenderGraph {
    pub(crate) fn prepare(
        project: &Project,
        options: &ExportOptions,
        ffmpeg: &Path,
        directory: &Path,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let whatsapp = (options.preset == ExportPreset::Whatsapp)
            .then(|| screenrecord_core::whatsapp_export_plan(project))
            .transpose()?;
        let layout = match whatsapp {
            Some(plan) => math::layout_dimensions(project, plan.width, plan.height)?,
            None => math::layout(project, &options.resolution)?,
        };
        let intervals = timeline::merged(project);
        let duration = intervals
            .last()
            .map(timeline::end)
            .ok_or("No video remains inside the selected trim.")?;
        if duration > screenrecord_core::MAX_RECORDING_RECOVERY_SECONDS {
            return Err("An export can contain eight hours plus the recording recovery allowance. Split longer recordings into separate exports.".into());
        }
        let screen = probe::probe_with_cancel(ffmpeg, Path::new(&project.screen_path), cancel)?;
        let video = screen
            .video()
            .ok_or("The screen recording has no video stream.")?;
        if video.width != Some(project.width) || video.height != Some(project.height) {
            return Err("The screen recording dimensions do not match the saved project.".into());
        }
        if screen
            .duration()
            .is_some_and(|value| value + 0.15 < project.edits.trim_end)
        {
            return Err("The selected trim ends after the screen recording. Reopen the recovered project and shorten its trim.".into());
        }
        let mut graph = Self {
            inputs: vec![Input {
                path: PathBuf::from(&project.screen_path),
                image: false,
                offset_seconds: 0.0,
            }],
            duration,
            layout,
            frame_count: output_frame_count(duration, options.fps),
            audio: audio::prepare(project, ffmpeg, cancel)?,
            ducking: None,
            envelopes: None,
            boundaries: intervals
                .iter()
                .flat_map(|interval| {
                    project
                        .edits
                        .redactions
                        .iter()
                        .flat_map(|region| [region.start, region.end])
                        .filter(|time| *time > interval.start && *time < interval.end)
                        .map(|time| {
                            timeline::unit_at(
                                interval.output_start + time - interval.start,
                                options.fps,
                            )
                        })
                })
                .collect(),
            cursor: math::smooth_cursor(&project.cursor, project.edits.cursor_smoothing),
            video_filters: String::new(),
            video_filters_without_camera: String::new(),
            directory: directory.into(),
            intervals,
            fps: options.fps,
            camera_start: 0.0,
            camera_duration: None,
            mask_patches: effects::patch_sizes(project),
            whatsapp,
            clicks: clicks::Clicks::new(project),
        };
        if audio::needs_ducking(
            &graph.audio,
            &project.edits.audio_processing.ducking,
            &graph.intervals,
        ) {
            graph.ducking = Some(project.edits.audio_processing.ducking.clone());
        }
        let keys = &project.edits.audio_processing.volume_keyframes;
        graph.envelopes = envelopes::Envelopes::new(
            keys,
            audio::active_envelopes(&graph.audio, keys, &graph.intervals),
        );
        let camera = if project.edits.camera.enabled {
            if let Some(path) = &project.camera_path {
                let info = probe::probe_with_cancel(ffmpeg, Path::new(path), cancel)?;
                let video = info
                    .video()
                    .ok_or("The webcam recording has no video stream.")?;
                let width = video
                    .width
                    .filter(|v| *v > 0)
                    .ok_or("Cannot read webcam width.")?;
                let height = video
                    .height
                    .filter(|v| *v > 0)
                    .ok_or("Cannot read webcam height.")?;
                graph.camera_start = video
                    .start_time
                    .as_deref()
                    .and_then(|time| time.parse::<f64>().ok())
                    .filter(|time| time.is_finite())
                    .unwrap_or(0.0);
                graph.camera_duration = video
                    .duration
                    .as_deref()
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite())
                    .map(|duration| duration + graph.camera_start)
                    .or_else(|| info.duration());
                Some((
                    graph.add_media(path, project.camera_offset_seconds),
                    width,
                    height,
                ))
            } else {
                None
            }
        } else {
            None
        };
        let blurred_background = background::enabled(project, &layout);
        let mut filters = if blurred_background {
            Vec::new()
        } else {
            vec![format!(
                "color=c=0x{}:s={}x{}:r={}:d=__DURATION__,format=rgba[canvas]",
                &project.edits.background[1..],
                layout.width,
                layout.height,
                options.fps
            )]
        };
        let mut result = "canvas";
        if layout.screen_visible {
            let initial = math::viewport(project, &layout, &graph.cursor, project.edits.trim_start);
            let branch = if blurred_background {
                "split=2[viewinput][backgroundinput];\n[viewinput]"
            } else {
                ""
            };
            filters.push(format!(
            "[0:v]trim=end=__SOURCE_END__,tpad=stop_mode=clone:stop_duration=__DURATION__,settb=AVTB,setpts=PTS-__SOURCE_START__/TB,fps={}:start_time=0:round=up,trim=end_frame=__FRAME_COUNT__,format=rgba[effectsource];\n__SCREEN_EFFECTS__{branch}sendcmd=f=motion.txt,crop@view=w={}:h={}:x={}:y={}:exact=1,scale={}:{}:flags=lanczos,setsar=1,format=rgba[screen]",
            options.fps, initial.width, initial.height, initial.x, initial.y, layout.frame_width, layout.frame_height
        ));
            if blurred_background {
                filters.push(background::filters(project, &layout, &initial));
            }
            let mut screen_label = "screen";
            if !graph.clicks.is_empty() {
                filters.push("[screen]__CLICKS__[clicked]".into());
                screen_label = "clicked";
            }
            if project.edits.show_cursor && !graph.cursor.is_empty() {
                let pointer_height = (28.0 * layout.reference_scale * project.edits.cursor_size)
                    .round()
                    .clamp(8.0, 256.0) as u32;
                assets::pointer(&directory.join("pointer.png"), pointer_height)?;
                let index = graph.add_image("pointer.png");
                filters.push(format!("[{index}:v]format=rgba[pointer]"));
                filters.push(format!("[{screen_label}][pointer]overlay@pointer=x=-1000:y=-1000:format=auto:eval=frame:shortest=1[pointed]"));
                screen_label = "pointed";
            }
            assets::mask(
                &directory.join("screen-mask.png"),
                layout.frame_width,
                layout.frame_height,
                project.edits.corner_radius as f64 * layout.reference_scale,
                false,
            )?;
            let mask = graph.add_image("screen-mask.png");
            filters.push(format!("[{mask}:v]format=gray[screenmask]"));
            filters.push(format!("[{screen_label}][screenmask]alphamerge[rounded]"));
            filters.push(format!(
                "[canvas][rounded]overlay=x={}:y={}:shortest=1:format=auto[screened]",
                layout.frame_x, layout.frame_y
            ));
            result = "screened";
        }
        graph.video_filters_without_camera = format!(
            "{};\n[{result}]__SUBTITLES__{OUTPUT_COLOR_FILTER}[outv]",
            filters.join(";\n")
        );
        if let Some((index, source_width, source_height)) = camera {
            let camera = &project.edits.camera;
            let math::FrameRect {
                x,
                y,
                width,
                height,
            } = math::camera_rect(project, &layout, source_width, source_height);
            let pane_layout = layout.camera_pane.is_some();
            let circle = !pane_layout && camera.shape == "circle";
            let mut camera_filters = format!(
                "[{index}:v]trim=end=__CAMERA_SOURCE_END__,__CAMERA_PAD__settb=AVTB,setpts=PTS-__CAMERA_SOURCE_START__/TB,fps={}:start_time=0:round=up,trim=end_frame=__FRAME_COUNT__,",
                options.fps
            );
            if pane_layout {
                let crop = math::camera_crop(
                    source_width,
                    source_height,
                    &math::FrameRect {
                        x,
                        y,
                        width,
                        height,
                    },
                );
                camera_filters.push_str(&format!(
                    "format=rgba,crop=w={}:h={}:x={}:y={}:exact=1,",
                    crop.width, crop.height, crop.x, crop.y
                ));
            } else if circle {
                camera_filters.push_str("crop=w='min(iw,ih)':h='min(iw,ih)',");
            }
            if camera.mirror {
                camera_filters.push_str("hflip,");
            }
            camera_filters.push_str(&format!(
                "scale={width}:{height}:flags=lanczos,setsar=1,format=rgba[webcam]"
            ));
            filters.push(camera_filters);
            if circle || pane_layout {
                assets::mask(
                    &directory.join("camera-mask.png"),
                    width,
                    height,
                    if circle {
                        width as f64 / 2.0
                    } else {
                        project.edits.corner_radius as f64 * layout.reference_scale
                    },
                    circle,
                )?;
                let index = graph.add_image("camera-mask.png");
                filters.push(format!("[{index}:v]format=gray[cameramask]"));
                filters.push("[webcam][cameramask]alphamerge=shortest=1[shapedcamera]".into());
            }
            let camera_label = if circle || pane_layout {
                "shapedcamera"
            } else {
                "webcam"
            };
            filters.push(format!("[{result}][{camera_label}]overlay=x={x}:y={y}:eof_action=pass:repeatlast=0:format=auto:enable='__CAMERA_VISIBLE__'[composed]"));
            result = "composed";
        }
        filters.push(format!(
            "[{result}]__SUBTITLES__{OUTPUT_COLOR_FILTER}[outv]"
        ));
        graph.video_filters = filters.join(";\n");
        Ok(graph)
    }

    fn add_media(&mut self, path: &str, offset_seconds: f64) -> usize {
        let index = self.inputs.len();
        self.inputs.push(Input {
            path: path.into(),
            image: false,
            offset_seconds,
        });
        index
    }
    fn add_image(&mut self, name: &str) -> usize {
        let index = self.inputs.len();
        self.inputs.push(Input {
            path: name.into(),
            image: true,
            offset_seconds: 0.0,
        });
        index
    }

    pub(crate) fn chunks(&self, chunk_frames: u64) -> Result<Vec<VideoChunk>, String> {
        if chunk_frames == 0 || chunk_frames > MAX_CHUNK_FRAMES {
            return Err("Invalid video chunk frame limit.".into());
        }
        let mut boundaries: Vec<_> = (0..self.frame_count)
            .step_by(chunk_frames as usize)
            .collect();
        boundaries.extend(
            self.boundaries
                .iter()
                .copied()
                .filter(|frame| *frame > 0 && *frame < self.frame_count),
        );
        boundaries.extend(
            self.intervals
                .iter()
                .map(|interval| timeline::unit_at(interval.output_start, self.fps)),
        );
        boundaries.push(self.frame_count);
        boundaries.sort_unstable();
        boundaries.dedup();
        if boundaries.len() as u64 - 1 > MAX_CHUNKS {
            return Err("This export exceeds the video chunk limit. Choose a shorter trim.".into());
        }
        boundaries
            .windows(2)
            .enumerate()
            .map(|(index, pair)| {
                let interval = self
                    .intervals
                    .iter()
                    .find(|interval| {
                        pair[0] >= timeline::unit_at(interval.output_start, self.fps)
                            && pair[0] < timeline::unit_at(timeline::end(interval), self.fps)
                    })
                    .ok_or("The edit timeline contains an unmapped output frame.")?;
                Ok(VideoChunk {
                    index,
                    first_frame: pair[0],
                    frames: pair[1] - pair[0],
                    source_start: interval.start + pair[0] as f64 / self.fps as f64
                        - interval.output_start,
                    source_end: interval.end,
                })
            })
            .collect()
    }

    pub(crate) fn has_audio(&self) -> bool {
        !self.audio.is_empty()
    }

    pub(crate) fn video_bitrate(&self) -> Option<u32> {
        self.whatsapp.map(|plan| plan.video_bitrate)
    }

    pub(crate) fn video_command(
        &self,
        project: &Project,
        options: &ExportOptions,
        ffmpeg: &Path,
        encoder: Encoder,
        chunk: VideoChunk,
        cancel: &AtomicBool,
    ) -> Result<Command, String> {
        self.video_command_with_rate(
            project,
            options,
            ffmpeg,
            (encoder, self.video_bitrate()),
            chunk,
            cancel,
        )
    }

    pub(crate) fn video_command_with_rate(
        &self,
        project: &Project,
        options: &ExportOptions,
        ffmpeg: &Path,
        encoding: (Encoder, Option<u32>),
        chunk: VideoChunk,
        cancel: &AtomicBool,
    ) -> Result<Command, String> {
        let (encoder, bitrate) = encoding;
        let source_start = chunk.source_start;
        let duration = chunk.frames as f64 / options.fps as f64;
        write_commands(
            project,
            options,
            &self.layout,
            &self.cursor,
            &self.directory,
            chunk,
            cancel,
        )?;
        let captions = subtitles::write_chunk(
            project,
            &self.layout,
            &self.directory,
            source_start,
            chunk.frames,
            options.fps,
        )?;
        let interval = self
            .intervals
            .iter()
            .find(|interval| {
                chunk.first_frame >= timeline::unit_at(interval.output_start, options.fps)
                    && chunk.first_frame < timeline::unit_at(timeline::end(interval), options.fps)
            })
            .ok_or("The click timeline contains an unmapped output frame.")?;
        let clicks = self.clicks.write_chunk(
            project,
            &self.layout,
            &self.cursor,
            &clicks::ClickWindow {
                interval: *interval,
                source_start: chunk.source_start,
                frames: chunk.frames,
                fps: options.fps,
            },
            &self.directory,
            cancel,
        )?;
        let visibility = camera_visibility(project, source_start, duration, options.fps);
        let camera_ended = self.camera_duration.is_some_and(|end| {
            source_start + project.camera_offset_seconds + screenrecord_core::TIME_EPSILON >= end
        });
        let camera_not_started =
            source_start + duration + project.camera_offset_seconds <= self.camera_start;
        let video_filters = if camera_ended || camera_not_started || visibility == "0" {
            &self.video_filters_without_camera
        } else {
            &self.video_filters
        };
        let mut visibility = format!(
            "({visibility})*gte(n,{})",
            effects::frame_at(
                self.camera_start - project.camera_offset_seconds,
                source_start,
                options.fps
            )
        );
        if let Some(end) = self.camera_duration {
            visibility.push_str(&format!(
                "*lt(n,{})",
                effects::frame_at(
                    end - project.camera_offset_seconds,
                    source_start,
                    options.fps
                )
            ));
        }
        let screen_effects = if self.layout.screen_visible {
            effects::filters(
                project,
                source_start,
                chunk.frames,
                options.fps,
                &self.directory,
                cancel,
                &self.mask_patches,
            )?
        } else {
            String::new()
        };
        let filters = video_filters
            .replace(
                "__CLICKS__",
                if clicks {
                    "ass=f=clicks.ass:alpha=1:shaping=simple"
                } else {
                    "null"
                },
            )
            .replace("__SCREEN_EFFECTS__", &screen_effects)
            .replace(
                "__SUBTITLES__",
                if captions {
                    "ass=f=subtitles.ass:alpha=1:shaping=complex,"
                } else {
                    ""
                },
            )
            .replace("__SOURCE_START__", &format!("{source_start:.9}"))
            .replace("__CAMERA_VISIBLE__", &visibility)
            .replace("__SOURCE_END__", &format!("{:.9}", chunk.source_end))
            .replace(
                "__CAMERA_SOURCE_END__",
                &format!("{:.9}", chunk.source_end + project.camera_offset_seconds),
            )
            .replace(
                "__CAMERA_PAD__",
                if self
                    .camera_duration
                    .is_some_and(|end| end >= chunk.source_end + project.camera_offset_seconds)
                {
                    "tpad=stop_mode=clone:stop_duration=__DURATION__,"
                } else {
                    ""
                },
            )
            .replace(
                "__CAMERA_SOURCE_START__",
                &format!("{:.9}", source_start + project.camera_offset_seconds),
            )
            .replace("__FRAME_COUNT__", &chunk.frames.to_string())
            .replace("__DURATION__", &format!("{duration:.9}"));
        std::fs::write(self.directory.join("filters.txt"), filters).map_err(|e| e.to_string())?;
        let mut command = Command::new(ffmpeg);
        command.current_dir(&self.directory).args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "warning",
            "-y",
            "-filter_complex_threads",
            "2",
            "-copyts",
        ]);
        for input in &self.inputs {
            if input.image {
                command.args([
                    "-loop",
                    "1",
                    "-framerate",
                    &options.fps.to_string(),
                    "-t",
                    &format!("{duration:.9}"),
                ]);
            } else {
                // Keep the preceding source frame when the output grid falls between input frames.
                command.args([
                    "-ss",
                    &format!(
                        "{:.9}",
                        (source_start + input.offset_seconds - 1.0).max(0.0)
                    ),
                    "-noaccurate_seek",
                ]);
            }
            command
                .args(["-protocol_whitelist", "file,pipe", "-i"])
                .arg(process::media_path(&input.path));
        }
        command.args(["-filter_complex_script", "filters.txt", "-map", "[outv]"]);
        command.arg("-an");
        command
            .args(encoder.args_for_bitrate(bitrate))
            .args([
                "-pix_fmt",
                "yuv420p",
                "-colorspace",
                "bt709",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-color_range",
                "tv",
                "-r",
                &options.fps.to_string(),
                "-frames:v",
                &chunk.frames.to_string(),
                "-metadata",
                "encoder=Screenrecord",
                "-progress",
                "pipe:1",
                "-stats_period",
                "0.2",
                "-f",
                "mp4",
            ])
            .arg(chunk_name(chunk.index));
        Ok(command)
    }

    pub(crate) fn mux_command(
        &self,
        _project: &Project,
        options: &ExportOptions,
        ffmpeg: &Path,
        chunks: &[VideoChunk],
    ) -> Result<Command, String> {
        let mut list = BufWriter::new(
            File::create(self.directory.join("chunks.txt")).map_err(|e| e.to_string())?,
        );
        for (index, chunk) in chunks.iter().enumerate() {
            writeln!(
                list,
                "file '{}'\nduration {:.9}",
                chunk_name(index),
                concat_duration(chunk.first_frame, chunk.frames, options.fps)
            )
            .map_err(|e| e.to_string())?;
        }
        list.flush().map_err(|e| e.to_string())?;
        let mut command = Command::new(ffmpeg);
        command.current_dir(&self.directory).args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "warning",
            "-y",
            "-f",
            "concat",
            "-safe",
            "1",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
            "chunks.txt",
        ]);
        if self.audio.is_empty() {
            command.args(["-map", "0:v:0", "-c:v", "copy", "-an"]);
        } else {
            if self.processes_audio() {
                command.args([
                    "-f",
                    "f32le",
                    "-ar",
                    "48000",
                    "-ac",
                    "2",
                    "-i",
                    ducking::MIX_FILE,
                ]);
            } else {
                let mut audio_list = BufWriter::new(
                    File::create(self.directory.join("audio.txt"))
                        .map_err(|error| error.to_string())?,
                );
                for chunk in self.audio_chunks() {
                    writeln!(
                        audio_list,
                        "file '{}'\nduration {:.12}",
                        audio_name(chunk.index),
                        concat_duration(chunk.first_sample, chunk.samples, 48_000)
                    )
                    .map_err(|error| error.to_string())?;
                }
                audio_list.flush().map_err(|error| error.to_string())?;
                command.args([
                    "-f",
                    "concat",
                    "-safe",
                    "1",
                    "-protocol_whitelist",
                    "file,pipe",
                    "-i",
                    "audio.txt",
                ]);
            }
            command.args([
                "-map",
                "0:v:0",
                "-c:v",
                "copy",
                "-af",
                &format!("asetpts=N/SR/TB,{}", audio::LIMITER),
                "-map",
                "1:a:0",
                "-c:a",
                "aac",
                "-b:a",
                &self
                    .whatsapp
                    .map_or(192_000, |plan| plan.audio_bitrate)
                    .to_string(),
                "-ar",
                "48000",
            ]);
        }
        command.args([
            "-t",
            &format!("{:.9}", self.duration),
            "-movflags",
            "+faststart",
            "-metadata",
            "encoder=Screenrecord",
            "-progress",
            "pipe:1",
            "-stats_period",
            "0.2",
            "-f",
            "mp4",
            "encoded.mp4",
        ]);
        Ok(command)
    }

    pub(crate) fn audio_chunks(&self) -> Vec<AudioChunk> {
        let mut chunks = Vec::new();
        for interval in &self.intervals {
            let first_sample = timeline::unit_at(interval.output_start, 48_000);
            let end = timeline::unit_at(timeline::end(interval), 48_000);
            if first_sample < end {
                chunks.push(AudioChunk {
                    index: chunks.len(),
                    first_sample,
                    samples: end - first_sample,
                    source_start: interval.start + first_sample as f64 / 48_000.0
                        - interval.output_start,
                    source_end: interval.end,
                });
            }
        }
        chunks
    }

    pub(crate) fn ducker(&self) -> Option<ducking::Ducker> {
        self.ducking
            .as_ref()
            .map(|settings| {
                ducking::Ducker::new(
                    settings.amount_db,
                    settings.enabled,
                    settings.system_enabled,
                )
            })
            .or_else(|| {
                self.envelopes
                    .as_ref()
                    .map(|_| ducking::Ducker::new(0.0, false, false))
            })
    }

    fn processes_audio(&self) -> bool {
        self.ducking.is_some() || self.envelopes.is_some()
    }

    pub(crate) fn mix_audio_stream(
        &self,
        input: impl std::io::Read,
        chunk: AudioChunk,
        ducker: &mut ducking::Ducker,
        cancel: &AtomicBool,
        progress: &std::sync::atomic::AtomicU64,
    ) -> Result<(), String> {
        if let Some(envelopes) = &self.envelopes {
            let mut cursor = envelopes.at(chunk.source_start);
            return ducker.mix_stream_with(
                input,
                &self.directory,
                chunk.first_sample,
                chunk.samples,
                cancel,
                progress,
                |sample, buses| cursor.apply(sample, buses),
            );
        }
        ducker.mix_stream(
            input,
            &self.directory,
            chunk.first_sample,
            chunk.samples,
            cancel,
            progress,
        )
    }

    pub(crate) fn audio_command(&self, ffmpeg: &Path, chunk: AudioChunk) -> Command {
        let tracks = audio::slice(&self.audio, chunk.source_start, chunk.source_end);
        let mut command = Command::new(ffmpeg);
        command.current_dir(&self.directory).args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "warning",
            "-y",
            "-filter_complex_threads",
            "2",
        ]);
        let filters = if self.processes_audio() {
            audio::append_bus_inputs(&mut command, &tracks, chunk.samples)
        } else {
            audio::append_range_inputs(&mut command, &tracks, chunk.samples, 0, false)
        };
        command.args([
            "-filter_complex",
            &filters.join(";"),
            "-map",
            "[outa]",
            "-c:a",
            "pcm_f32le",
            "-ar",
            "48000",
        ]);
        if self.processes_audio() {
            command.args(["-ac", "6", "-f", "f32le", "pipe:1"]);
        } else {
            command
                .args([
                    "-progress",
                    "pipe:1",
                    "-stats_period",
                    "0.2",
                    "-ac",
                    "2",
                    "-rf64",
                    "auto",
                    "-f",
                    "wav",
                ])
                .arg(audio_name(chunk.index));
        }
        command
    }
}

fn audio_name(index: usize) -> String {
    format!("audio-{index:05}.wav")
}

fn camera_visibility(project: &Project, source_start: f64, duration: f64, fps: u32) -> String {
    let Some(intervals) = &project.camera_intervals else {
        return "1".into();
    };
    let expressions: Vec<_> = intervals
        .iter()
        .filter(|interval| interval.end > source_start && interval.start < source_start + duration)
        .map(|interval| {
            format!(
                "(gte(n,{})*lt(n,{}))",
                effects::frame_at(interval.start, source_start, fps),
                effects::frame_at(interval.end, source_start, fps)
            )
        })
        .collect();
    if expressions.is_empty() {
        "0".into()
    } else {
        expressions.join("+")
    }
}

fn concat_duration(first: u64, count: u64, rate: u32) -> f64 {
    let start = (first as f64 / rate as f64 * 1_000_000.0).round();
    let end = ((first + count) as f64 / rate as f64 * 1_000_000.0).round();
    (end - start) / 1_000_000.0
}

fn chunk_name(index: usize) -> String {
    format!("video-{index:05}.mp4")
}

pub(crate) fn output_frame_count(duration: f64, fps: u32) -> u64 {
    // Decimal trim values can land a few floating-point ulps beyond an exact frame boundary.
    (duration * fps as f64 - 1e-7).ceil().max(1.0) as u64
}

fn write_commands(
    project: &Project,
    options: &ExportOptions,
    layout: &Layout,
    cursor: &[screenrecord_core::CursorSample],
    directory: &Path,
    chunk: VideoChunk,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if !layout.screen_visible {
        return Ok(());
    }
    let mut file =
        BufWriter::new(File::create(directory.join("motion.txt")).map_err(|e| e.to_string())?);
    let mut last = None;
    let mut background_file = if background::enabled(project, layout) {
        Some(BufWriter::new(
            File::create(directory.join("background-motion.txt")).map_err(|e| e.to_string())?,
        ))
    } else {
        None
    };
    let mut previous_background = None;
    let mut bytes = 0usize;
    let pointer_size = (28.0 * layout.reference_scale * project.edits.cursor_size)
        .round()
        .clamp(8.0, 256.0);
    for frame in 0..chunk.frames {
        if frame % 256 == 0 && cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled.".into());
        }
        let time = frame as f64 / options.fps as f64;
        let source_time = chunk.source_start + frame as f64 / options.fps as f64;
        let view = math::viewport(project, layout, cursor, source_time);
        if let Some(file) = &mut background_file {
            let crop = background::rectangle(&view, layout);
            let current = (crop.width, crop.height, crop.x, crop.y);
            if previous_background != Some(current) {
                let command_time = (time * 1_000_000.0).floor() / 1_000_000.0;
                let line = format!("{command_time:.6} crop@background w {}, crop@background h {}, crop@background x {}, crop@background y {};\n", crop.width, crop.height, crop.x, crop.y);
                bytes += line.len();
                if bytes > 256 * 1024 {
                    return Err("This video chunk exceeds the motion-data budget.".into());
                }
                file.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
                previous_background = Some(current);
            }
        }
        let px = if view.cursor_visible {
            view.cursor_x - pointer_size * 0.08
        } else {
            -1000.0
        };
        let py = if view.cursor_visible {
            view.cursor_y - pointer_size * 0.05
        } else {
            -1000.0
        };
        let values = (
            view.width,
            view.height,
            view.x,
            view.y,
            (px * 4.0).round() as i64,
            (py * 4.0).round() as i64,
        );
        if last == Some(values) {
            continue;
        }
        let mut commands = Vec::new();
        if last.is_none_or(|previous: (u32, u32, u32, u32, i64, i64)| previous.0 != values.0) {
            commands.push(format!("crop@view w {}", view.width));
        }
        if last.is_none_or(|previous| previous.1 != values.1) {
            commands.push(format!("crop@view h {}", view.height));
        }
        if last.is_none_or(|previous| previous.2 != values.2) {
            commands.push(format!("crop@view x {}", view.x));
        }
        if last.is_none_or(|previous| previous.3 != values.3) {
            commands.push(format!("crop@view y {}", view.y));
        }
        if project.edits.show_cursor && !cursor.is_empty() {
            if last.is_none_or(|previous| previous.4 != values.4) {
                commands.push(format!("overlay@pointer x {px:.3}"));
            }
            if last.is_none_or(|previous| previous.5 != values.5) {
                commands.push(format!("overlay@pointer y {py:.3}"));
            }
        }
        if !commands.is_empty() {
            let command_time = (time * 1_000_000.0).floor() / 1_000_000.0;
            let line = format!("{command_time:.6} {};\n", commands.join(", "));
            bytes += line.len();
            if bytes > 256 * 1024 {
                return Err("This video chunk exceeds the motion-data budget.".into());
            }
            file.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        }
        last = Some(values);
    }
    file.flush().map_err(|e| e.to_string())?;
    if let Some(file) = &mut background_file {
        file.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}
