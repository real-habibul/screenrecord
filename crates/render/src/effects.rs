use screenrecord_core::{redaction_rectangle_at, MaskRectangle, Project, Redaction};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_COMMAND_BYTES: usize = 8 * 1024 * 1024;

fn blur(width: u32, height: u32, strength: f64) -> String {
    let filter = format!("gblur=sigma={strength:.6}:steps=3");
    // FFmpeg 8.1 mishandles one-pixel dimensions; duplicate the sole edge row or column internally.
    if width == 1 || height == 1 {
        format!(
            "scale={}:{}:flags=neighbor,{filter},crop=w={width}:h={height}:x=0:y=0:exact=1",
            width.max(2),
            height.max(2)
        )
    } else {
        filter
    }
}

pub(crate) fn frame_at(time: f64, trim_start: f64, fps: u32) -> u64 {
    (((time - trim_start).max(0.0) * fps as f64) - 1e-7)
        .ceil()
        .max(0.0) as u64
}

pub(crate) fn rectangle(region: MaskRectangle, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let x = (region.x * width as f64)
        .floor()
        .clamp(0.0, (width - 1) as f64) as u32;
    let y = (region.y * height as f64)
        .floor()
        .clamp(0.0, (height - 1) as f64) as u32;
    let right = ((region.x + region.width) * width as f64)
        .ceil()
        .clamp((x + 1) as f64, width as f64) as u32;
    let bottom = ((region.y + region.height) * height as f64)
        .ceil()
        .clamp((y + 1) as f64, height as f64) as u32;
    (x, y, right - x, bottom - y)
}

pub(crate) fn patch_size(region: &Redaction, width: u32, height: u32) -> (u32, u32) {
    region
        .tracking
        .iter()
        .flat_map(|tracking| &tracking.keyframes)
        .map(|key| redaction_rectangle_at(region, key.time))
        .filter(|rect| !rect.lost)
        .map(|rect| rectangle(rect, width, height))
        .fold((1, 1), |(w, h), (_, _, rw, rh)| (w.max(rw), h.max(rh)))
}

pub(crate) fn patch_sizes(project: &Project) -> Vec<(u32, u32)> {
    project
        .edits
        .redactions
        .iter()
        .map(|region| patch_size(region, project.width, project.height))
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
struct State {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    patch_x: u32,
    patch_y: u32,
    lost: bool,
}

fn state(region: &Redaction, time: f64, project: &Project, patch: (u32, u32)) -> State {
    let rect = redaction_rectangle_at(region, time);
    let (x, y, width, height) = rectangle(rect, project.width, project.height);
    State {
        x,
        y,
        width,
        height,
        patch_x: x.min(project.width - patch.0),
        patch_y: y.min(project.height - patch.1),
        lost: rect.lost,
    }
}

fn command(
    time: f64,
    values: &[String],
    text: &mut String,
    bytes: &mut usize,
) -> Result<(), String> {
    let line = format!(
        "{:.6} {};\n",
        (time * 1_000_000.0).floor() / 1_000_000.0,
        values.join(", ")
    );
    *bytes += line.len();
    if *bytes > MAX_COMMAND_BYTES {
        return Err("This video chunk exceeds the mask motion-data budget.".into());
    }
    text.push_str(&line);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn tracked(
    project: &Project,
    region: &Redaction,
    index: usize,
    source_start: f64,
    frames: u64,
    fps: u32,
    directory: &Path,
    cancel: &AtomicBool,
    label: &str,
    enabled: &str,
    bytes: &mut usize,
    patch: (u32, u32),
) -> Result<String, String> {
    let mut files = [String::new(), String::new(), String::new(), String::new()];
    let mut previous = None;
    for frame in 0..frames {
        if frame % 128 == 0 && cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled.".into());
        }
        let time = frame as f64 / fps as f64;
        let current = state(region, source_start + time, project, patch);
        if previous == Some(current) {
            continue;
        }
        if region.mode == "solid" {
            command(
                time,
                &[
                    format!("drawbox@tracked{index} x {}", current.x),
                    format!("drawbox@tracked{index} y {}", current.y),
                    format!("drawbox@tracked{index} w {}", current.width),
                    format!("drawbox@tracked{index} h {}", current.height),
                ],
                &mut files[0],
                bytes,
            )?;
        } else {
            // Each branch advances its own commands with its own frame, rather than mutating a queued sibling frame.
            command(
                time,
                &[
                    format!("crop@trackcrop{index} x {}", current.patch_x),
                    format!("crop@trackcrop{index} y {}", current.patch_y),
                ],
                &mut files[0],
                bytes,
            )?;
            command(
                time,
                &[
                    format!("drawbox@trackmask{index} x {}", current.x - current.patch_x),
                    format!("drawbox@trackmask{index} y {}", current.y - current.patch_y),
                    format!("drawbox@trackmask{index} w {}", current.width),
                    format!("drawbox@trackmask{index} h {}", current.height),
                ],
                &mut files[1],
                bytes,
            )?;
            command(
                time,
                &[
                    format!("overlay@trackoverlay{index} x {}", current.patch_x),
                    format!("overlay@trackoverlay{index} y {}", current.patch_y),
                ],
                &mut files[2],
                bytes,
            )?;
            command(
                time,
                &[
                    format!(
                        "drawbox@tracklost{index} w {}",
                        if current.lost { project.width } else { 1 }
                    ),
                    format!(
                        "drawbox@tracklost{index} h {}",
                        if current.lost { project.height } else { 1 }
                    ),
                    format!(
                        "drawbox@tracklost{index} color black@{}",
                        u8::from(current.lost)
                    ),
                ],
                &mut files[3],
                bytes,
            )?;
        }
        previous = Some(current);
    }
    for (branch, text) in files
        .iter()
        .enumerate()
        .filter(|(_, text)| !text.is_empty())
    {
        if cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled.".into());
        }
        fs::write(directory.join(format!("mask-{index}-{branch}.txt")), text)
            .map_err(|error| format!("Cannot write mask motion data: {error}"))?;
    }
    let initial = state(region, source_start, project, patch);
    if region.mode == "solid" {
        return Ok(format!("[{label}]sendcmd=f=mask-{index}-0.txt,drawbox@tracked{index}=x={}:y={}:w={}:h={}:color=black:t=fill:enable='{enabled}'[effect{index}];\n", initial.x, initial.y, initial.width, initial.height));
    }
    let blur = blur(patch.0, patch.1, region.strength);
    Ok(format!(
        "[{label}]split=2[trackkeep{index}][trackinput{index}];\n\
        [trackinput{index}]sendcmd=f=mask-{index}-0.txt,crop@trackcrop{index}=w={}:h={}:x={}:y={}:exact=1,{blur}[trackblur{index}];\n\
        color=c=black:s={}x{}:r={fps}:d={:.9},format=rgba,sendcmd=f=mask-{index}-1.txt,drawbox@trackmask{index}=x={}:y={}:w={}:h={}:color=white:t=fill,extractplanes=r[trackmask{index}];\n\
        [trackblur{index}][trackmask{index}]alphamerge=shortest=1[trackmasked{index}];\n\
        [trackkeep{index}]sendcmd=f=mask-{index}-2.txt[trackbase{index}];\n\
        [trackbase{index}][trackmasked{index}]overlay@trackoverlay{index}=x={}:y={}:format=auto:shortest=1:enable='{enabled}'[trackpatched{index}];\n\
        [trackpatched{index}]sendcmd=f=mask-{index}-3.txt,drawbox@tracklost{index}=x=0:y=0:w={}:h={}:color=black@{}:t=fill:enable='{enabled}'[effect{index}];\n",
        patch.0, patch.1, initial.patch_x, initial.patch_y, patch.0, patch.1, frames as f64 / fps as f64,
        initial.x - initial.patch_x, initial.y - initial.patch_y, initial.width, initial.height,
        initial.patch_x, initial.patch_y, if initial.lost { project.width } else { 1 }, if initial.lost { project.height } else { 1 }, u8::from(initial.lost),
    ))
}

pub(crate) fn filters(
    project: &Project,
    source_start: f64,
    frames: u64,
    fps: u32,
    directory: &Path,
    cancel: &AtomicBool,
    patches: &[(u32, u32)],
) -> Result<String, String> {
    if frames > crate::graph::MAX_CHUNK_FRAMES {
        return Err("The mask motion timeline exceeds the video chunk limit.".into());
    }
    let mut result = String::new();
    let mut label = "effectsource".to_owned();
    let mut bytes = 0usize;
    for (index, region) in project.edits.redactions.iter().enumerate() {
        let start = frame_at(region.start, source_start, fps);
        let end = frame_at(region.end, source_start, fps);
        if start >= frames || end == 0 {
            continue;
        }
        let enabled = format!("gte(n,{})*lt(n,{})", start, end);
        let (x, y, w, h) = rectangle(
            redaction_rectangle_at(region, source_start),
            project.width,
            project.height,
        );
        let next = format!("effect{index}");
        if region.tracking.is_some() {
            let patch = *patches.get(index).ok_or("Missing mask patch geometry.")?;
            result.push_str(&tracked(
                project,
                region,
                index,
                source_start,
                frames,
                fps,
                directory,
                cancel,
                &label,
                &enabled,
                &mut bytes,
                patch,
            )?);
        } else if region.mode == "solid" {
            result.push_str(&format!("[{label}]drawbox=x={x}:y={y}:w={w}:h={h}:color=black:t=fill:enable='{enabled}'[{next}];\n"));
        } else {
            let blur = blur(w, h, region.strength);
            result.push_str(&format!("[{label}]split=2[keep{index}][blurinput{index}];\n[blurinput{index}]crop=w={w}:h={h}:x={x}:y={y}:exact=1,{blur}[blurred{index}];\n[keep{index}][blurred{index}]overlay=x={x}:y={y}:format=auto:shortest=1:enable='{enabled}'[{next}];\n"));
        }
        label = next;
    }
    result.push_str(&format!("[{label}]"));
    Ok(result)
}
