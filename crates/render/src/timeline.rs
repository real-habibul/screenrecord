use screenrecord_core::{kept_intervals, KeptInterval, Project};

pub(crate) fn merged(project: &Project) -> Vec<KeptInterval> {
    let mut result: Vec<KeptInterval> = Vec::new();
    for interval in kept_intervals(&project.edits, project.duration) {
        if let Some(previous) = result.last_mut() {
            if previous.end == interval.start {
                previous.end = interval.end;
                continue;
            }
        }
        result.push(interval);
    }
    result
}

pub(crate) fn unit_at(seconds: f64, rate: u32) -> u64 {
    (seconds * rate as f64 - 1e-7).ceil().max(0.0) as u64
}

pub(crate) fn end(interval: &KeptInterval) -> f64 {
    interval.output_start + interval.end - interval.start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_grid_does_not_round_each_interval_up() {
        let lengths = [0.101, 0.103, 0.107, 0.109];
        for rate in [30, 60, 48_000] {
            let mut time = 0.0;
            let mut units = 0;
            for length in lengths {
                units += unit_at(time + length, rate) - unit_at(time, rate);
                time += length;
            }
            assert_eq!(units, unit_at(lengths.iter().sum(), rate));
        }
        assert_eq!(unit_at(0.4 - 0.1, 30), 9);
    }
}
