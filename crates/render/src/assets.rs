use std::{fs::File, io::BufWriter, path::Path};

fn write_rgba(path: &Path, width: u32, height: u32, data: &[u8]) -> Result<(), String> {
    let writer = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|e| e.to_string())?
        .write_image_data(data)
        .map_err(|e| e.to_string())
}

pub(crate) fn mask(
    path: &Path,
    width: u32,
    height: u32,
    radius: f64,
    circle: bool,
) -> Result<(), String> {
    let mut data = vec![0u8; width as usize * height as usize * 4];
    let r = radius.min(width.min(height) as f64 / 2.0).max(0.0);
    for y in 0..height {
        for x in 0..width {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let distance = if circle {
                let cx = width as f64 / 2.0;
                let cy = height as f64 / 2.0;
                (cx.min(cy) - ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()).clamp(0.0, 1.0)
            } else if r < 0.5 {
                1.0
            } else {
                let cx = px.clamp(r, width as f64 - r);
                let cy = py.clamp(r, height as f64 - r);
                (r - ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()).clamp(0.0, 1.0)
            };
            let index = (y as usize * width as usize + x as usize) * 4;
            let alpha = (distance * 255.0).round() as u8;
            data[index..index + 4].copy_from_slice(&[alpha, alpha, alpha, 255]);
        }
    }
    write_rgba(path, width, height, &data)
}

fn inside_polygon(x: f64, y: f64, points: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let (xi, yi) = points[i];
        let (xj, yj) = points[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub(crate) fn pointer(path: &Path, height: u32) -> Result<(), String> {
    let width = height;
    let polygon = [
        (0.08, 0.05),
        (0.08, 0.80),
        (0.28, 0.62),
        (0.43, 0.96),
        (0.60, 0.88),
        (0.44, 0.55),
        (0.73, 0.55),
    ];
    let mut data = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let mut fill = 0;
            let mut edge = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = (x as f64 + (sx as f64 + 0.5) / 4.0) / width as f64;
                    let py = (y as f64 + (sy as f64 + 0.5) / 4.0) / height as f64;
                    let point_inside = inside_polygon(px, py, &polygon);
                    if point_inside {
                        fill += 1;
                    } else {
                        let d = 1.2 / height as f64;
                        if [(-d, 0.0), (d, 0.0), (0.0, -d), (0.0, d)]
                            .iter()
                            .any(|(dx, dy)| inside_polygon(px + dx, py + dy, &polygon))
                        {
                            edge += 1;
                        }
                    }
                }
            }
            let index = (y as usize * width as usize + x as usize) * 4;
            let alpha = ((fill + edge) * 255 / 16) as u8;
            let color = if fill + edge > 0 {
                (fill * 245 + edge * 25) / (fill + edge)
            } else {
                0
            } as u8;
            data[index..index + 4].copy_from_slice(&[color, color, color, alpha]);
        }
    }
    write_rgba(path, width, height, &data)
}
