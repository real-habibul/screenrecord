use screenrecord_core::{CropSettings, CursorSample, Project, ZoomSegment};

#[derive(Clone, Copy, Debug)]
pub struct FrameRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frame_x: u32,
    pub frame_y: u32,
    pub reference_scale: f64,
    pub screen_visible: bool,
    pub camera_pane: Option<FrameRect>,
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub cursor_x: f64,
    pub cursor_y: f64,
    pub cursor_visible: bool,
    pub pressed: bool,
}

pub(crate) fn even(value: f64) -> u32 {
    ((value.round().max(2.0) as u32) / 2).max(1) * 2
}

pub fn layout(project: &Project, resolution: &str) -> Result<Layout, String> {
    let short = match resolution {
        "1080p" => 1080.0,
        "1440p" => 1440.0,
        "2160p" => 2160.0,
        _ => return Err("Unsupported export resolution.".into()),
    };
    let crop = crop(project);
    let source_ratio = project.width as f64 * crop.width / (project.height as f64 * crop.height);
    let ratio = match project.edits.aspect_ratio.as_str() {
        "source" => source_ratio,
        "16:9" => 16.0 / 9.0,
        "9:16" => 9.0 / 16.0,
        "1:1" => 1.0,
        _ => return Err("Unsupported canvas aspect ratio.".into()),
    };
    let width = even(short * ratio.max(1.0));
    let height = even(short * (1.0 / ratio).max(1.0));
    layout_dimensions(project, width, height)
}

pub(crate) fn layout_dimensions(
    project: &Project,
    width: u32,
    height: u32,
) -> Result<Layout, String> {
    let short = width.min(height) as f64;
    let crop = crop(project);
    if width > 8192 || height > 8192 {
        return Err(
            "This crop produces an export larger than 8192 pixels. Choose a standard aspect ratio."
                .into(),
        );
    }
    let padding = project.edits.padding as f64 * short / 1080.0;
    let available_width = (width as f64 - 2.0 * padding).max(2.0);
    let available_height = (height as f64 - 2.0 * padding).max(2.0);
    let camera_layout = project.edits.camera.layout.as_str();
    let inner_width = even(available_width).min(width);
    let inner_height = even(available_height).min(height);
    let inner = FrameRect {
        x: (width - inner_width) / 2,
        y: (height - inner_height) / 2,
        width: inner_width,
        height: inner_height,
    };
    let mut screen_pane = None;
    let camera_pane = if camera_layout == "fullscreen" {
        Some(inner)
    } else if camera_layout == "split" {
        let gap = ((project.edits.padding.min(24) as f64 * short / 1080.0).round() as u32 / 2) * 2;
        if width >= height {
            let usable = inner.width - gap;
            let camera_width = even(usable as f64 * project.edits.camera.width).min(usable - 2);
            let screen_width = usable - camera_width;
            screen_pane = Some(FrameRect {
                width: screen_width,
                ..inner
            });
            Some(FrameRect {
                x: inner.x + screen_width + gap,
                width: camera_width,
                ..inner
            })
        } else {
            let usable = inner.height - gap;
            let camera_height = even(usable as f64 * project.edits.camera.width).min(usable - 2);
            let screen_height = usable - camera_height;
            screen_pane = Some(FrameRect {
                height: screen_height,
                ..inner
            });
            Some(FrameRect {
                y: inner.y + screen_height + gap,
                height: camera_height,
                ..inner
            })
        }
    } else {
        None
    };
    let fill_pane = screen_pane.unwrap_or(inner);
    let fill = project.edits.fit_mode == "fill";
    let (fit_width, fit_height) = screen_pane
        .map(|pane| (pane.width as f64, pane.height as f64))
        .unwrap_or((available_width, available_height));
    let fit = (fit_width / (project.width as f64 * crop.width))
        .min(fit_height / (project.height as f64 * crop.height));
    let frame_width = if fill {
        fill_pane.width
    } else {
        even(project.width as f64 * crop.width * fit)
            .min(screen_pane.map_or(width, |pane| pane.width))
    };
    let frame_height = if fill {
        fill_pane.height
    } else {
        even(project.height as f64 * crop.height * fit)
            .min(screen_pane.map_or(height, |pane| pane.height))
    };
    Ok(Layout {
        width,
        height,
        frame_width,
        frame_height,
        frame_x: if fill {
            fill_pane.x
        } else {
            screen_pane.map_or((width - frame_width) / 2, |pane| {
                pane.x + (pane.width - frame_width) / 2
            })
        },
        frame_y: if fill {
            fill_pane.y
        } else {
            screen_pane.map_or((height - frame_height) / 2, |pane| {
                pane.y + (pane.height - frame_height) / 2
            })
        },
        reference_scale: short / 1080.0,
        screen_visible: camera_layout != "fullscreen",
        camera_pane,
    })
}

pub fn camera_rect(
    project: &Project,
    layout: &Layout,
    source_width: u32,
    source_height: u32,
) -> FrameRect {
    if let Some(pane) = layout.camera_pane {
        return pane;
    }
    let camera = &project.edits.camera;
    let desired_width = even(camera.width * layout.width as f64).min(layout.width);
    let ratio = if camera.shape == "circle" {
        1.0
    } else {
        source_width as f64 / source_height as f64
    };
    let width = desired_width.min(even(layout.height as f64 * ratio));
    let height = even(width as f64 / ratio).min(layout.height);
    FrameRect {
        x: (camera.x * layout.width as f64)
            .round()
            .clamp(0.0, (layout.width - width) as f64) as u32,
        y: (camera.y * layout.height as f64)
            .round()
            .clamp(0.0, (layout.height - height) as f64) as u32,
        width,
        height,
    }
}

pub fn camera_crop(source_width: u32, source_height: u32, pane: &FrameRect) -> FrameRect {
    let ratio = pane.width as f64 / pane.height as f64;
    let width = (source_width as f64)
        .min(source_height as f64 * ratio)
        .round()
        .clamp(1.0, source_width as f64) as u32;
    let height = (source_height as f64)
        .min(source_width as f64 / ratio)
        .round()
        .clamp(1.0, source_height as f64) as u32;
    FrameRect {
        x: (source_width - width) / 2,
        y: (source_height - height) / 2,
        width,
        height,
    }
}

pub(crate) fn crop(project: &Project) -> CropSettings {
    project.edits.crop.clone().unwrap_or(CropSettings {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    })
}

pub fn smooth_cursor(samples: &[CursorSample], smoothing: f64) -> Vec<CursorSample> {
    let mut result = Vec::with_capacity(samples.len());
    let tau = smoothing.clamp(0.0, 1.0) * 0.12;
    let mut previous: Option<CursorSample> = None;
    for sample in samples {
        let mut next = *sample;
        if let Some(last) = previous {
            let dt = (sample.time - last.time).max(0.0);
            if tau > 0.0 && sample.visible && last.visible && dt < 0.5 {
                let alpha = 1.0 - (-dt / tau).exp();
                next.x = last.x + (sample.x - last.x) * alpha;
                next.y = last.y + (sample.y - last.y) * alpha;
            }
        }
        previous = Some(next);
        result.push(next);
    }
    result
}

pub fn cursor_at(samples: &[CursorSample], time: f64) -> CursorSample {
    if samples.is_empty() || time < samples[0].time {
        return CursorSample {
            buttons: 0,
            time,
            x: 0.5,
            y: 0.5,
            pressed: false,
            visible: false,
        };
    }
    let index = samples
        .partition_point(|sample| sample.time <= time)
        .saturating_sub(1);
    let mut sample = samples[index];
    if let Some(next) = samples.get(index + 1) {
        let dt = next.time - sample.time;
        if dt > 0.0 && dt < 0.5 && sample.visible && next.visible {
            let amount = ((time - sample.time) / dt).clamp(0.0, 1.0);
            sample.x += (next.x - sample.x) * amount;
            sample.y += (next.y - sample.y) * amount;
        }
    }
    sample.time = time;
    sample
}

fn smoothstep(value: f64) -> f64 {
    let u = value.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

pub fn zoom_at(segment: &ZoomSegment, time: f64) -> f64 {
    if time < segment.start || time >= segment.end {
        return 1.0;
    }
    let ramp = 0.35_f64.min((segment.end - segment.start) / 2.0).max(0.001);
    let weight =
        smoothstep((time - segment.start) / ramp).min(smoothstep((segment.end - time) / ramp));
    1.0 + (segment.scale - 1.0) * weight
}

pub fn viewport(
    project: &Project,
    layout: &Layout,
    cursor: &[CursorSample],
    time: f64,
) -> Viewport {
    let crop = crop(project);
    let sample = cursor_at(cursor, time);
    let segment = project
        .edits
        .zooms
        .iter()
        .find(|zoom| time >= zoom.start && time < zoom.end);
    let mut zoom = segment.map(|segment| zoom_at(segment, time)).unwrap_or(1.0);
    let (mut center_x, mut center_y) = match segment {
        Some(segment) if !segment.follow_cursor || !sample.visible => (segment.x, segment.y),
        _ => (sample.x, sample.y),
    };
    let base_x = (crop.x * project.width as f64).floor() as u32;
    let base_y = (crop.y * project.height as f64).floor() as u32;
    let base_width = even(crop.width * project.width as f64).min(project.width - base_x);
    let base_height = even(crop.height * project.height as f64).min(project.height - base_y);
    let (cover_width, cover_height) = if project.edits.fit_mode == "fill" {
        let base_center_x = (base_x as f64 + base_width as f64 / 2.0) / project.width as f64;
        let base_center_y = (base_y as f64 + base_height as f64 / 2.0) / project.height as f64;
        let weight = segment
            .filter(|segment| segment.scale > 1.0)
            .map(|segment| ((zoom - 1.0) / (segment.scale - 1.0)).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        center_x = base_center_x + (center_x - base_center_x) * weight;
        center_y = base_center_y + (center_y - base_center_y) * weight;
        let ratio = layout.frame_width as f64 / layout.frame_height as f64;
        (
            even((base_width as f64).min(base_height as f64 * ratio)).min(base_width),
            even((base_height as f64).min(base_width as f64 / ratio)).min(base_height),
        )
    } else {
        (base_width, base_height)
    };
    if let Some(reframe) = screenrecord_core::reframe_at(&project.edits.reframe_keyframes, time) {
        center_x = reframe.x;
        center_y = reframe.y;
        zoom *= reframe.scale;
    }
    let width = even(cover_width as f64 / zoom).min(cover_width);
    let height = even(cover_height as f64 / zoom).min(cover_height);
    let x = (center_x * project.width as f64 - width as f64 / 2.0)
        .clamp(base_x as f64, (base_x + base_width - width) as f64)
        .round() as u32;
    let y = (center_y * project.height as f64 - height as f64 / 2.0)
        .clamp(base_y as f64, (base_y + base_height - height) as f64)
        .round() as u32;
    let cursor_x =
        (sample.x * project.width as f64 - x as f64) / width as f64 * layout.frame_width as f64;
    let cursor_y =
        (sample.y * project.height as f64 - y as f64) / height as f64 * layout.frame_height as f64;
    Viewport {
        x,
        y,
        width,
        height,
        cursor_x,
        cursor_y,
        cursor_visible: project.edits.show_cursor
            && sample.visible
            && cursor_x >= 0.0
            && cursor_x < layout.frame_width as f64
            && cursor_y >= 0.0
            && cursor_y < layout.frame_height as f64,
        pressed: sample.pressed,
    }
}

pub fn auto_zoom(cursor: &[CursorSample], duration: f64) -> Vec<ZoomSegment> {
    if !duration.is_finite() || duration <= 0.0 {
        return Vec::new();
    }
    let mut clicks: Vec<_> = screenrecord_core::cursor_clicks(cursor)
        .filter(|click| click.time.is_finite() && click.time >= 0.0 && click.time < duration)
        .collect();
    clicks.dedup_by(|a, b| a.time == b.time);
    let mut groups: Vec<Vec<screenrecord_core::CursorClick>> = Vec::new();
    for click in clicks {
        match groups.last_mut() {
            Some(group) if click.time - group.last().unwrap().time <= 3.0 => group.push(click),
            _ => groups.push(vec![click]),
        }
    }
    groups
        .into_iter()
        .filter(|group| group.len() >= 2)
        .take(2000)
        .enumerate()
        .filter_map(|(i, group)| {
            let start = (group[0].time - 0.35).max(0.0);
            let end = (group.last().unwrap().time + 1.3).min(duration);
            (end - start >= 0.1).then(|| ZoomSegment {
                id: format!("auto-{}", i + 1),
                start,
                end,
                scale: 1.65,
                x: group[0].x,
                y: group[0].y,
                follow_cursor: true,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        serde_json::from_value(serde_json::json!({
            "version": screenrecord_core::PROJECT_VERSION, "id": "test", "name": "test",
            "createdAt": "2026-11-11", "directory": "", "screenPath": "", "width": 320,
            "height": 180, "fps": 30, "duration": 2.0, "cursor": [], "status": "ready",
            "edits": screenrecord_core::EditSettings::new(2.0)
        }))
        .unwrap()
    }

    #[test]
    fn auto_zoom_uses_button_edges_and_deduplicates_simultaneous_buttons() {
        let mut both = sample(1.0, 0.3, true);
        both.buttons = 3;
        let baseline = sample(0.0, 0.3, false);
        assert!(auto_zoom(&[baseline, both], 8.0).is_empty());
        let mut right = sample(2.0, 0.6, true);
        right.buttons = 2;
        let zooms = auto_zoom(&[baseline, both, sample(1.2, 0.3, false), right], 8.0);
        assert_eq!(zooms.len(), 1);
        assert_eq!(zooms[0].start, 0.65);
        assert_eq!(zooms[0].end, 3.3);
    }
    #[test]
    fn framing_matches_the_independent_shared_reference() {
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../../../testdata/reframe-reference.json")).unwrap();
        for case in reference["cases"].as_array().unwrap() {
            let mut project = project();
            project.width = case["source"]["width"].as_u64().unwrap() as u32;
            project.height = case["source"]["height"].as_u64().unwrap() as u32;
            let mut edits = serde_json::to_value(&project.edits).unwrap();
            for (name, value) in case["edits"].as_object().unwrap() {
                if name == "camera" {
                    for (field, value) in value.as_object().unwrap() {
                        edits["camera"][field] = value.clone();
                    }
                } else {
                    edits[name] = value.clone();
                }
            }
            project.edits = serde_json::from_value(edits).unwrap();
            let cursor = if case["cursor"].is_null() {
                vec![]
            } else {
                vec![serde_json::from_value(case["cursor"].clone()).unwrap()]
            };
            let layout = layout(&project, reference["resolution"].as_str().unwrap()).unwrap();
            let view = viewport(&project, &layout, &cursor, case["time"].as_f64().unwrap());
            let rectangle = |x, y, width, height| serde_json::json!({ "x":x, "y":y, "width":width, "height":height });
            let actual = serde_json::json!({
                "canvas": {"width":layout.width, "height":layout.height},
                "frame": rectangle(layout.frame_x,layout.frame_y,layout.frame_width,layout.frame_height),
                "viewport": rectangle(view.x,view.y,view.width,view.height),
                "screenVisible": layout.screen_visible,
                "cameraPane": layout.camera_pane.map(|pane|rectangle(pane.x,pane.y,pane.width,pane.height))
            });
            assert_eq!(actual, case["expected"], "{}", case["id"]);
        }
    }

    #[test]
    fn split_layout_reserves_the_camera_pane_without_camera_media() {
        let mut project = project();
        project.edits.camera.layout = "split".into();
        project.edits.camera.enabled = false;
        project.edits.camera.width = 0.3;
        project.edits.aspect_ratio = "16:9".into();
        let landscape = layout(&project, "1080p").unwrap();
        let camera = landscape.camera_pane.unwrap();
        assert_eq!(
            (
                landscape.frame_x,
                landscape.frame_y,
                landscape.frame_width,
                landscape.frame_height
            ),
            (32, 180, 1282, 720)
        );
        assert_eq!(
            (camera.x, camera.y, camera.width, camera.height),
            (1338, 32, 550, 1016)
        );
        project.edits.aspect_ratio = "9:16".into();
        let portrait = layout(&project, "1080p").unwrap();
        let camera = portrait.camera_pane.unwrap();
        assert_eq!(
            (
                portrait.frame_x,
                portrait.frame_y,
                portrait.frame_width,
                portrait.frame_height
            ),
            (32, 387, 1016, 572)
        );
        assert_eq!(
            (camera.x, camera.y, camera.width, camera.height),
            (32, 1338, 1016, 550)
        );
        project.edits.camera.layout = "fullscreen".into();
        let full = layout(&project, "1080p").unwrap();
        assert!(!full.screen_visible);
        assert_eq!(full.camera_pane.unwrap().height, 1856);
    }

    #[test]
    fn split_zero_padding_and_fractional_resolution_keep_even_panes_inside_canvas() {
        let mut project = project();
        project.edits.camera.layout = "split".into();
        for aspect in ["16:9", "9:16", "1:1"] {
            project.edits.aspect_ratio = aspect.into();
            for padding in [0, 1, 23, 240] {
                project.edits.padding = padding;
                for share in [0.08, 0.6] {
                    project.edits.camera.width = share;
                    let geometry = layout(&project, "1440p").unwrap();
                    let pane = geometry.camera_pane.unwrap();
                    assert_eq!(pane.width % 2, 0);
                    assert_eq!(pane.height % 2, 0);
                    assert!(pane.x + pane.width <= geometry.width);
                    assert!(pane.y + pane.height <= geometry.height);
                    assert!(geometry.frame_x + geometry.frame_width <= geometry.width);
                    assert!(geometry.frame_y + geometry.frame_height <= geometry.height);
                    if geometry.width >= geometry.height {
                        assert!(geometry.frame_x + geometry.frame_width <= pane.x);
                    } else {
                        assert!(geometry.frame_y + geometry.frame_height <= pane.y);
                    }
                }
            }
        }
    }

    #[test]
    fn fill_centers_the_crop_without_following_an_idle_cursor() {
        let mut project = project();
        project.edits.fit_mode = "fill".into();
        project.edits.aspect_ratio = "9:16".into();
        project.width = 321;
        project.height = 181;
        project.edits.crop = Some(CropSettings {
            x: 0.1,
            y: 0.2,
            width: 0.7,
            height: 0.6,
        });
        let layout = layout(&project, "1080p").unwrap();
        let source = [sample(0.0, 0.1, false), sample(1.0, 0.9, false)];
        for time in [0.0, 1.0] {
            let view = viewport(&project, &layout, &source, time);
            assert_eq!(
                (view.x, view.y, view.width, view.height),
                (115, 36, 58, 108)
            );
        }
        project.edits.zooms = vec![ZoomSegment {
            id: "scale-one".into(),
            start: 0.0,
            end: 2.0,
            scale: 1.0,
            x: 1.0,
            y: 1.0,
            follow_cursor: true,
        }];
        let view = viewport(&project, &layout, &source, 1.0);
        assert_eq!(
            (view.x, view.y, view.width, view.height),
            (115, 36, 58, 108)
        );
        project.edits.zooms[0].scale = 2.0;
        let initial = viewport(&project, &layout, &source, 0.0);
        assert_eq!((initial.x, initial.y), (115, 36));
        let zoomed = viewport(&project, &layout, &source, 1.0);
        assert_eq!(
            (zoomed.x, zoomed.y, zoomed.width, zoomed.height),
            (228, 64, 28, 54)
        );
    }

    #[test]
    fn fill_keeps_every_zoom_view_inside_integer_crop_and_reserved_pane() {
        let mut project = project();
        project.width = 321;
        project.height = 181;
        project.edits.fit_mode = "fill".into();
        project.edits.crop = Some(CropSettings {
            x: 0.17,
            y: 0.12,
            width: 0.61,
            height: 0.73,
        });
        project.edits.zooms = vec![ZoomSegment {
            id: "edge".into(),
            start: 0.0,
            end: 2.0,
            scale: 3.0,
            x: 1.0,
            y: 0.0,
            follow_cursor: false,
        }];
        for resolution in ["1080p", "1440p", "2160p"] {
            for aspect in ["source", "16:9", "9:16", "1:1"] {
                project.edits.aspect_ratio = aspect.into();
                for camera_layout in ["bubble", "split", "fullscreen"] {
                    project.edits.camera.layout = camera_layout.into();
                    let layout = layout(&project, resolution).unwrap();
                    assert!(layout.frame_x + layout.frame_width <= layout.width);
                    assert!(layout.frame_y + layout.frame_height <= layout.height);
                    assert_eq!(layout.screen_visible, camera_layout != "fullscreen");
                    for time in [0.0, 0.175, 0.35, 1.0, 1.825, 2.0] {
                        let view = viewport(&project, &layout, &[], time);
                        assert!(view.x >= 54 && view.y >= 21);
                        assert!(view.x + view.width <= 250);
                        assert!(view.y + view.height <= 153);
                        assert!(view.width > 0 && view.height > 0);
                    }
                    if camera_layout == "split" {
                        let camera = layout.camera_pane.unwrap();
                        if layout.width >= layout.height {
                            assert!(layout.frame_x + layout.frame_width <= camera.x);
                        } else {
                            assert!(layout.frame_y + layout.frame_height <= camera.y);
                        }
                    }
                }
            }
        }
    }

    fn sample(time: f64, x: f64, pressed: bool) -> CursorSample {
        CursorSample {
            buttons: 0,
            time,
            x,
            y: 0.5,
            pressed,
            visible: true,
        }
    }

    #[test]
    fn held_button_is_one_click() {
        assert!(auto_zoom(&[sample(1.0, 0.2, true), sample(2.0, 0.5, true)], 8.0).is_empty());
        let zooms = auto_zoom(
            &[
                sample(1.0, 0.2, true),
                sample(1.1, 0.2, false),
                sample(2.0, 0.5, true),
            ],
            8.0,
        );
        assert_eq!(zooms.len(), 1);
        assert!(zooms[0].start >= 0.0 && zooms[0].end <= 8.0);
    }

    #[test]
    fn zoom_eases_at_both_ends() {
        let zoom = ZoomSegment {
            id: "a".into(),
            start: 1.0,
            end: 4.0,
            scale: 2.0,
            x: 0.5,
            y: 0.5,
            follow_cursor: false,
        };
        assert_eq!(zoom_at(&zoom, 1.0), 1.0);
        assert_eq!(zoom_at(&zoom, 2.0), 2.0);
        assert!((zoom_at(&zoom, 1.175) - zoom_at(&zoom, 3.825)).abs() < 0.0001);
        assert_eq!(zoom_at(&zoom, 4.0), 1.0);
    }

    #[test]
    fn smoothing_resets_after_capture_gap() {
        let smoothed = smooth_cursor(
            &[
                sample(0.0, 0.0, false),
                sample(0.05, 1.0, false),
                sample(1.0, 1.0, false),
            ],
            1.0,
        );
        assert!(smoothed[1].x > 0.0 && smoothed[1].x < 1.0);
        assert_eq!(smoothed[2].x, 1.0);
        assert!(!cursor_at(&smoothed, -0.1).visible);
    }
}
