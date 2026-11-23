use screenrecord_core::{VolumeKeyframe, VolumeKeyframes};

#[derive(Clone, Copy)]
struct Point {
    time: f64,
    volume: f64,
    slope: f64,
}

pub(crate) struct Envelopes {
    curves: [Vec<Point>; 3],
}

impl Envelopes {
    pub(crate) fn new(keys: &VolumeKeyframes, active: [bool; 3]) -> Option<Self> {
        let mut curves = [Vec::new(), Vec::new(), Vec::new()];
        for (index, keys) in [&keys.microphone, &keys.system, &keys.music]
            .into_iter()
            .enumerate()
        {
            if !active[index] {
                continue;
            }
            curves[index] = keys
                .iter()
                .enumerate()
                .map(|(index, key)| Point {
                    time: key.time,
                    volume: key.volume,
                    slope: keys.get(index + 1).map_or(0.0, |next| {
                        (next.volume - key.volume) / (next.time - key.time)
                    }),
                })
                .collect();
        }
        curves
            .iter()
            .any(|curve| !curve.is_empty())
            .then_some(Self { curves })
    }

    pub(crate) fn at(&self, source_start: f64) -> Cursor<'_> {
        Cursor {
            curves: &self.curves,
            right: self
                .curves
                .each_ref()
                .map(|curve| curve.partition_point(|point| point.time <= source_start)),
            source_start,
        }
    }
}

pub(crate) struct Cursor<'a> {
    curves: &'a [Vec<Point>; 3],
    right: [usize; 3],
    source_start: f64,
}

impl Cursor<'_> {
    pub(crate) fn apply(&mut self, sample: u64, buses: &mut [f32; 6]) {
        let time = self.source_start + sample as f64 / 48_000.0;
        for (bus, points) in self.curves.iter().enumerate() {
            if points.is_empty() {
                continue;
            }
            while self.right[bus] < points.len() && points[self.right[bus]].time <= time {
                self.right[bus] += 1;
            }
            let left = points[self.right[bus].saturating_sub(1)];
            let gain = if self.right[bus] == 0 || self.right[bus] == points.len() {
                left.volume
            } else {
                (left.volume + left.slope * (time - left.time)).clamp(0.0, 1.0)
            };
            for sample in &mut buses[bus * 2..bus * 2 + 2] {
                *sample = (f64::from(*sample) * gain) as f32;
            }
        }
    }
}

pub(crate) fn differs_from_unity(keys: &[VolumeKeyframe], start: f64, end: f64) -> bool {
    let interior =
        keys.partition_point(|key| key.time <= start)..keys.partition_point(|key| key.time < end);
    !keys.is_empty()
        && (screenrecord_core::volume_at(keys, start) != 1.0
            || screenrecord_core::volume_at(keys, end) != 1.0
            || keys[interior].iter().any(|key| key.volume != 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points() -> Vec<VolumeKeyframe> {
        [
            (0.05, 0.25),
            (0.201234, 1.0),
            (0.31, 0.0),
            (0.511234, 0.75),
            (0.9, 1.0),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (time, volume))| VolumeKeyframe {
            id: index.to_string(),
            time,
            volume,
        })
        .collect()
    }

    #[test]
    fn cached_source_envelopes_match_core_for_fractional_starts_holds_and_all_buses() {
        let keys = points();
        let curves = VolumeKeyframes {
            microphone: keys.clone(),
            system: keys.clone(),
            music: keys.clone(),
        };
        let envelopes = Envelopes::new(&curves, [true; 3]).unwrap();
        for start in [0.0, 0.201234, 0.511234, 1.5] {
            let mut cursor = envelopes.at(start);
            for frame in 0..48_000 {
                let original = [0.2, -0.1, 0.3, -0.4, 0.5, -0.6];
                let mut actual = original;
                cursor.apply(frame, &mut actual);
                let expected = screenrecord_core::volume_at(&keys, start + frame as f64 / 48_000.0);
                for (actual, original) in actual.into_iter().zip(original) {
                    assert!((f64::from(actual) - f64::from(original) * expected).abs() < 4e-8);
                }
            }
        }
        assert!(!differs_from_unity(&keys, 1.0, 2.0));
        assert!(differs_from_unity(&keys, 0.25, 0.27));
        assert!(Envelopes::new(&curves, [false; 3]).is_none());
    }
}
