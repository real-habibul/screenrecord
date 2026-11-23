use crate::math::{FrameRect, Layout, Viewport};
use screenrecord_core::Project;

pub(crate) const MAX_SCRATCH_SIDE: u32 = 960;

pub(crate) fn enabled(project: &Project, layout: &Layout) -> bool {
    layout.screen_visible && project.edits.background_mode == "videoBlur"
}

pub(crate) fn rectangle(view: &Viewport, layout: &Layout) -> FrameRect {
    let width = ((view.height as f64 * layout.width as f64 / layout.height as f64).floor() as u32)
        .max(1)
        .min(view.width);
    let height = ((view.width as f64 * layout.height as f64 / layout.width as f64).floor() as u32)
        .max(1)
        .min(view.height);
    FrameRect {
        x: view.x + (view.width - width) / 2,
        y: view.y + (view.height - height) / 2,
        width,
        height,
    }
}

pub(crate) fn scratch(layout: &Layout) -> (u32, u32) {
    let scale = (MAX_SCRATCH_SIDE as f64 / layout.width.max(layout.height) as f64).min(1.0);
    (
        (layout.width as f64 * scale).round().max(2.0) as u32,
        (layout.height as f64 * scale).round().max(2.0) as u32,
    )
}

pub(crate) fn filters(project: &Project, layout: &Layout, initial: &Viewport) -> String {
    let crop = rectangle(initial, layout);
    let (width, height) = scratch(layout);
    let sigma = project.edits.background_blur * layout.reference_scale;
    let sigma_x = sigma * width as f64 / layout.width as f64;
    let sigma_y = sigma * height as f64 / layout.height as f64;
    format!(
        "[backgroundinput]sendcmd=f=background-motion.txt,crop@background=w={}:h={}:x={}:y={}:exact=1,scale={width}:{height}:flags=lanczos,setsar=1,format=gbrp,gblur=sigma={sigma_x:.9}:sigmaV={sigma_y:.9}:steps=3,scale={}:{}:flags=bilinear,format=rgba[canvas]",
        crop.width, crop.height, crop.x, crop.y, layout.width, layout.height
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_matches_independent_cover_scratch_and_sigma_reference() {
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../../../testdata/background-reference.json"))
                .unwrap();
        let mut project: Project = serde_json::from_value(serde_json::json!({
            "version":screenrecord_core::PROJECT_VERSION,"id":"test","name":"test","createdAt":"2026-11-15","directory":"","screenPath":"","width":320,"height":180,"fps":30,"duration":2.0,"cursor":[],"status":"ready","edits":screenrecord_core::EditSettings::new(2.0)
        })).unwrap();
        for case in reference["cases"].as_array().unwrap() {
            let integer =
                |item: &serde_json::Value, name: &str| item[name].as_u64().unwrap() as u32;
            let canvas = &case["canvas"];
            let view = &case["viewport"];
            let layout = Layout {
                width: integer(canvas, "width"),
                height: integer(canvas, "height"),
                frame_x: 0,
                frame_y: 0,
                frame_width: 1,
                frame_height: 1,
                reference_scale: integer(canvas, "width").min(integer(canvas, "height")) as f64
                    / 1080.0,
                screen_visible: true,
                camera_pane: None,
            };
            let view = Viewport {
                x: integer(view, "x"),
                y: integer(view, "y"),
                width: integer(view, "width"),
                height: integer(view, "height"),
                cursor_x: 0.0,
                cursor_y: 0.0,
                cursor_visible: false,
                pressed: false,
            };
            let cover = rectangle(&view, &layout);
            let (width, height) = scratch(&layout);
            assert_eq!(
                serde_json::json!({"x":cover.x,"y":cover.y,"width":cover.width,"height":cover.height}),
                case["expected"]["crop"],
                "{}",
                case["id"]
            );
            assert_eq!(
                serde_json::json!({"width":width,"height":height}),
                case["expected"]["scratch"],
                "{}",
                case["id"]
            );
            project.edits.background_blur = case["strength"].as_f64().unwrap();
            let filter = filters(&project, &layout, &view);
            let sigma = format!(
                "gblur=sigma={:.9}:sigmaV={:.9}:",
                case["expected"]["sigmaX"].as_f64().unwrap(),
                case["expected"]["sigmaY"].as_f64().unwrap()
            );
            assert!(filter.contains(&sigma), "{}: {filter}", case["id"]);
        }
    }

    #[test]
    fn cover_never_samples_outside_viewport_and_scratch_is_bounded() {
        for (canvas_width, canvas_height) in [(1080, 1920), (1920, 1080), (8192, 2160), (1, 8192)] {
            let layout = Layout {
                width: canvas_width,
                height: canvas_height,
                frame_x: 0,
                frame_y: 0,
                frame_width: canvas_width,
                frame_height: canvas_height,
                reference_scale: 1.0,
                screen_visible: true,
                camera_pane: None,
            };
            let (width, height) = scratch(&layout);
            assert!((2..=MAX_SCRATCH_SIDE).contains(&width));
            assert!((2..=MAX_SCRATCH_SIDE).contains(&height));
            for (width, height) in [(1, 1), (1, 2), (2, 1), (1820, 6), (320, 180), (17, 39)] {
                let view = Viewport {
                    x: 11,
                    y: 37,
                    width,
                    height,
                    cursor_x: 0.0,
                    cursor_y: 0.0,
                    cursor_visible: false,
                    pressed: false,
                };
                let crop = rectangle(&view, &layout);
                assert!(crop.width > 0 && crop.height > 0);
                assert!(crop.x >= view.x && crop.y >= view.y);
                assert!(crop.x + crop.width <= view.x + view.width);
                assert!(crop.y + crop.height <= view.y + view.height);
                assert!((crop.x - view.x).abs_diff(view.x + view.width - crop.x - crop.width) <= 1);
                assert!(
                    (crop.y - view.y).abs_diff(view.y + view.height - crop.y - crop.height) <= 1
                );
            }
        }
    }
}
