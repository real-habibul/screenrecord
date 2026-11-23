use super::TrackingRect;
use std::sync::atomic::{AtomicBool, Ordering};

const RADIUS: i32 = 24;
const MIN_CORRELATION: f64 = 0.94;
const MIN_MARGIN: f64 = 0.035;

struct Template {
    samples: Vec<(usize, usize, f64)>,
    sum: f64,
    variance: f64,
}

impl Template {
    fn new(
        frame: &[u8],
        stride: usize,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        grid: usize,
    ) -> Option<Self> {
        let mut samples = Vec::with_capacity(grid * grid);
        for row in 0..grid.min(h) {
            for column in 0..grid.min(w) {
                let px = column * (w - 1) / (grid.min(w) - 1);
                let py = row * (h - 1) / (grid.min(h) - 1);
                samples.push((px, py, f64::from(frame[(y + py) * stride + x + px])));
            }
        }
        let sum: f64 = samples.iter().map(|v| v.2).sum();
        let variance =
            samples.iter().map(|v| v.2 * v.2).sum::<f64>() - sum * sum / samples.len() as f64;
        if variance / (samples.len() as f64) < 64.0 {
            return None;
        }
        Some(Self {
            samples,
            sum,
            variance,
        })
    }

    fn score(&self, frame: &[u8], stride: usize, x: usize, y: usize) -> f64 {
        let mut sum = 0.0;
        let mut square = 0.0;
        let mut product = 0.0;
        for &(px, py, template) in &self.samples {
            let value = f64::from(frame[(y + py) * stride + x + px]);
            sum += value;
            square += value * value;
            product += template * value;
        }
        let n = self.samples.len() as f64;
        let variance = square - sum * sum / n;
        if variance / n < 64.0 {
            return -1.0;
        }
        ((product - self.sum * sum / n) / (self.variance * variance).sqrt()).clamp(-1.0, 1.0)
    }
}

pub(super) struct Tracker {
    coarse: Template,
    fine: Template,
    width: usize,
    height: usize,
    w: usize,
    h: usize,
    first: (i32, i32),
    last: (i32, i32),
    rect: TrackingRect,
}

impl Tracker {
    pub(super) fn new(
        frame: &[u8],
        width: usize,
        height: usize,
        rect: TrackingRect,
    ) -> Option<Self> {
        let x = (rect.x * width as f64).round() as usize;
        let y = (rect.y * height as f64).round() as usize;
        let w = (rect.width * width as f64).round() as usize;
        let h = (rect.height * height as f64).round() as usize;
        if w < 8 || h < 8 || x + w > width || y + h > height {
            return None;
        }
        Some(Self {
            coarse: Template::new(frame, width, x, y, w, h, 8)?,
            fine: Template::new(frame, width, x, y, w, h, 16)?,
            width,
            height,
            w,
            h,
            first: (x as i32, y as i32),
            last: (x as i32, y as i32),
            rect,
        })
    }

    pub(super) fn locate(&mut self, frame: &[u8], cancel: &AtomicBool) -> Option<(f64, f64, f64)> {
        let xmin = (self.last.0 - RADIUS).max(0);
        let xmax = (self.last.0 + RADIUS).min((self.width - self.w) as i32);
        let ymin = (self.last.1 - RADIUS).max(0);
        let ymax = (self.last.1 + RADIUS).min((self.height - self.h) as i32);
        let mut best = Vec::with_capacity(33);
        for y in ymin..=ymax {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            for x in xmin..=xmax {
                let score = self.coarse.score(frame, self.width, x as usize, y as usize);
                let insert = best.partition_point(|&(value, _, _)| value > score);
                if insert < 32 {
                    best.insert(insert, (score, x, y));
                    best.truncate(32);
                }
            }
        }
        for candidate in &mut best {
            candidate.0 = self.fine.score(
                frame,
                self.width,
                candidate.1 as usize,
                candidate.2 as usize,
            );
        }
        best.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        let &(score, x, y) = best.first()?;
        let runner = best
            .iter()
            .find(|&&(_, px, py)| (px - x).abs() > 3 || (py - y).abs() > 3)
            .map_or(-1.0, |v| v.0);
        if score < MIN_CORRELATION || score - runner < MIN_MARGIN {
            return None;
        }
        let nx = self.rect.x + f64::from(x - self.first.0) / self.width as f64;
        let ny = self.rect.y + f64::from(y - self.first.1) / self.height as f64;
        if nx < -0.000_001
            || ny < -0.000_001
            || nx + self.rect.width > 1.000_001
            || ny + self.rect.height > 1.000_001
        {
            return None;
        }
        self.last = (x, y);
        Some((
            nx.clamp(0.0, 1.0 - self.rect.width),
            ny.clamp(0.0, 1.0 - self.rect.height),
            score,
        ))
    }
}
