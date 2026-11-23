use crate::math;
use screenrecord_core::{
    ClickButton, CursorClick, CursorSample, KeptInterval, Project, CLICK_DURATION,
};
use std::{
    fmt::Write,
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_ASS_BYTES: usize = 4 * 1024 * 1024;

pub(crate) struct ClickWindow {
    pub interval: KeptInterval,
    pub source_start: f64,
    pub frames: u64,
    pub fps: u32,
}

pub(crate) struct Clicks {
    channels: [Vec<CursorClick>; 3],
}

impl Clicks {
    pub(crate) fn new(project: &Project) -> Self {
        let mut channels: [Vec<CursorClick>; 3] = Default::default();
        if project.edits.show_cursor
            && project.edits.show_clicks
            && project.edits.camera.layout != "fullscreen"
        {
            for click in screenrecord_core::cursor_clicks(&project.cursor) {
                if click.time >= project.edits.trim_start && click.time < project.edits.trim_end {
                    channels[channel(click.button)].push(click);
                }
            }
        }
        Self { channels }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.channels.iter().all(Vec::is_empty)
    }

    pub(crate) fn write_chunk(
        &self,
        project: &Project,
        layout: &math::Layout,
        cursor: &[CursorSample],
        window: &ClickWindow,
        directory: &Path,
        cancel: &AtomicBool,
    ) -> Result<bool, String> {
        if self.is_empty() || !layout.screen_visible {
            return Ok(false);
        }
        let ClickWindow {
            interval,
            source_start,
            frames,
            fps,
        } = window;
        let fps = *fps;
        let source_start = *source_start;
        let frames = *frames;
        let final_time =
            (source_start + frames.saturating_sub(1) as f64 / fps as f64).min(interval.end);
        if !self.channels.iter().any(|events| {
            let start = events.partition_point(|event| event.time < source_start - CLICK_DURATION);
            events[start..]
                .iter()
                .take_while(|event| event.time <= final_time + screenrecord_core::TIME_EPSILON)
                .any(|event| {
                    event.time >= interval.start
                        && event.time < interval.end
                        && event.end > source_start
                })
        }) {
            return Ok(false);
        }
        let mut contents = format!("[Script Info]\nScriptType: v4.00+\nPlayResX: {}\nPlayResY: {}\nScaledBorderAndShadow: yes\nYCbCr Matrix: None\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Segoe UI,12,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,5,0,0,0,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n", layout.frame_width, layout.frame_height);
        let scale = project.edits.cursor_size * layout.reference_scale;
        let mut any = false;
        for frame in 0..frames {
            if frame % 64 == 0 && cancel.load(Ordering::Relaxed) {
                return Err("Export cancelled.".into());
            }
            let time = source_start + frame as f64 / fps as f64;
            if frame >= crate::effects::frame_at(interval.end, source_start, fps) {
                continue;
            }
            let active = self.channels.each_ref().map(|events| {
                let index = events.partition_point(|click| {
                    crate::effects::frame_at(click.time, source_start, fps) <= frame
                });
                index
                    .checked_sub(1)
                    .and_then(|index| events.get(index))
                    .filter(|click| {
                        click.time >= interval.start
                            && frame < crate::effects::frame_at(click.end, source_start, fps)
                    })
            });
            if active.iter().all(Option::is_none) {
                continue;
            }
            let view = math::viewport(project, layout, cursor, time);
            for (kind, click) in active.into_iter().enumerate() {
                let Some(click) = click else {
                    continue;
                };
                let x = (click.x * project.width as f64 - view.x as f64) / view.width as f64
                    * layout.frame_width as f64;
                let y = (click.y * project.height as f64 - view.y as f64) / view.height as f64
                    * layout.frame_height as f64;
                if x < 0.0
                    || y < 0.0
                    || x >= layout.frame_width as f64
                    || y >= layout.frame_height as f64
                {
                    continue;
                }
                let age = ((time - click.time) / CLICK_DURATION).clamp(0.0, 1.0);
                let radius = (12.0 + 16.0 * age) * scale;
                let alpha = (255.0 * (1.0 - (1.0 - age) * 0.95)).round() as u8;
                for (pass, width, color) in [
                    (0, 4.5 * scale, "2B2420"),
                    (1, 2.5 * scale, color(click.button)),
                ] {
                    // Libass anchors drawings from a nonnegative local origin before applying alignment.
                    let origin = radius + width / 2.0;
                    let mut drawing = annulus(radius, width, origin);
                    if click.button == ClickButton::Right {
                        drawing.push_str(&annulus(radius * 0.64, width, origin));
                    }
                    writeln!(contents, "Dialogue: {},{},{},Default,,0,0,0,,{{\\an5\\pos({x:.4},{y:.4})\\bord0\\shad0\\1c&H{color}&\\1a&H{alpha:02X}&\\p6}}{drawing}", kind * 2 + pass, timestamp(frame * 100 / fps as u64), timestamp((frame + 1) * 100 / fps as u64)).map_err(|error| error.to_string())?;
                }
                any = true;
                if contents.len() > MAX_ASS_BYTES {
                    return Err("This video chunk exceeds the click-overlay budget.".into());
                }
            }
        }
        if any {
            fs::write(directory.join("clicks.ass"), contents).map_err(|error| error.to_string())?;
        }
        Ok(any)
    }
}

fn channel(button: ClickButton) -> usize {
    match button {
        ClickButton::Left => 0,
        ClickButton::Right => 1,
        ClickButton::Generic => 2,
    }
}

fn color(button: ClickButton) -> &'static str {
    match button {
        ClickButton::Left => "4AB6F4",
        ClickButton::Right => "EFD966",
        ClickButton::Generic => "EEEEEE",
    }
}

fn timestamp(value: u64) -> String {
    format!(
        "{}:{:02}:{:02}.{:02}",
        value / 360_000,
        value / 6_000 % 60,
        value / 100 % 60,
        value % 100
    )
}

fn annulus(radius: f64, width: f64, origin: f64) -> String {
    let mut path = circle(radius + width / 2.0, 1.0, origin);
    path.push_str(&circle((radius - width / 2.0).max(0.0), -1.0, origin));
    path
}

fn circle(radius: f64, direction: f64, origin: f64) -> String {
    let coordinate = |value: f64| (value * 32.0).round() as i64;
    let mut path = format!("m {} {} ", coordinate(radius + origin), coordinate(origin));
    let step = std::f64::consts::FRAC_PI_2 * direction;
    let k = 0.552_284_749_830_793_6 * direction;
    for part in 0..4 {
        let a = part as f64 * step;
        let b = a + step;
        let points = [
            radius * (a.cos() - k * a.sin()),
            radius * (a.sin() + k * a.cos()),
            radius * (b.cos() + k * b.sin()),
            radius * (b.sin() - k * b.cos()),
            radius * b.cos(),
            radius * b.sin(),
        ]
        .map(|value| coordinate(value + origin));
        write!(
            path,
            "b {} {} {} {} {} {} ",
            points[0], points[1], points[2], points[3], points[4], points[5]
        )
        .unwrap();
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_schedule_is_bounded_cancellable_and_skips_empty_chunks() {
        let mut project: Project = serde_json::from_value(serde_json::json!({
            "version": screenrecord_core::PROJECT_VERSION, "id":"fixture", "name":"fixture",
            "createdAt":"2026-11-19", "directory":"", "screenPath":"", "width":320,
            "height":180, "fps":60, "duration":30.0, "cursor":[], "status":"ready",
            "edits":screenrecord_core::EditSettings::new(30.0)
        }))
        .unwrap();
        project.edits.cursor_size = 4.0;
        project.cursor = (0..1800)
            .map(|index| CursorSample {
                time: index as f64 / 120.0,
                x: 0.5,
                y: 0.5,
                visible: true,
                buttons: if index % 2 == 1 { 7 } else { 0 },
                pressed: index % 2 == 1,
            })
            .collect();
        let schedule = Clicks::new(&project);
        let directory = tempfile::tempdir().unwrap();
        let layout = math::layout(&project, "2160p").unwrap();
        let window = ClickWindow {
            interval: KeptInterval {
                start: 0.0,
                end: 30.0,
                output_start: 0.0,
            },
            source_start: 0.0,
            frames: 900,
            fps: 60,
        };
        assert!(schedule
            .write_chunk(
                &project,
                &layout,
                &project.cursor,
                &window,
                directory.path(),
                &AtomicBool::new(false)
            )
            .unwrap());
        let bytes = fs::metadata(directory.path().join("clicks.ass"))
            .unwrap()
            .len();
        assert!(bytes < MAX_ASS_BYTES as u64);
        let events = fs::read_to_string(directory.path().join("clicks.ass"))
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("Dialogue:"))
            .count();
        assert_eq!(events, 899 * 6);
        let empty = tempfile::tempdir().unwrap();
        let window = ClickWindow {
            source_start: 16.0,
            ..window
        };
        assert!(!schedule
            .write_chunk(
                &project,
                &layout,
                &project.cursor,
                &window,
                empty.path(),
                &AtomicBool::new(false)
            )
            .unwrap());
        assert!(!empty.path().join("clicks.ass").exists());
        let window = ClickWindow {
            source_start: 0.1,
            frames: 7,
            ..window
        };
        assert!(schedule
            .write_chunk(
                &project,
                &layout,
                &project.cursor,
                &window,
                empty.path(),
                &AtomicBool::new(true)
            )
            .unwrap_err()
            .contains("cancelled"));
        assert!(schedule
            .write_chunk(
                &project,
                &layout,
                &project.cursor,
                &window,
                empty.path(),
                &AtomicBool::new(false)
            )
            .unwrap());
        let text = fs::read_to_string(empty.path().join("clicks.ass")).unwrap();
        assert!(
            text.contains("Dialogue: 0,0:00:00.00,"),
            "An overlapping event before the chunk must remain visible on its first frame"
        );
        project.cursor.clear();
        assert!(Clicks::new(&project).is_empty());
        eprintln!("Click overlay stress: 900 frames, 3 channels, 4K/cursorSize4, {events} vector passes, {bytes} ASS bytes; empty chunks create no overlay file.");
    }
}
