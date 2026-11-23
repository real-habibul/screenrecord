use crate::CursorSample;

pub const CLICK_DURATION: f64 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClickButton {
    Left,
    Right,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct CursorClick {
    pub time: f64,
    pub end: f64,
    pub x: f64,
    pub y: f64,
    pub button: ClickButton,
}

pub fn cursor_clicks(samples: &[CursorSample]) -> impl Iterator<Item = CursorClick> + '_ {
    let mut held = 0;
    let mut next_hidden = 0;
    samples
        .iter()
        .enumerate()
        .filter_map(move |(i, sample)| {
            let buttons = if sample.buttons == 0 && sample.pressed {
                4
            } else {
                sample.buttons
            };
            let rising = buttons & !held;
            held = buttons;
            if rising == 0 || !sample.visible || (i == 0 && sample.buttons != 0) {
                return None;
            }
            // A visibility break also ends rings whose buttons were already released.
            if next_hidden <= i {
                next_hidden = samples[i..]
                    .iter()
                    .position(|s| !s.visible)
                    .map_or(samples.len(), |offset| i + offset);
            }
            let mut end = sample.time + CLICK_DURATION;
            if let Some(hidden) = samples.get(next_hidden) {
                end = end.min(hidden.time);
            }
            (end > sample.time).then_some((sample, rising, end))
        })
        .flat_map(|(sample, rising, end)| {
            [
                (1, ClickButton::Left),
                (2, ClickButton::Right),
                (4, ClickButton::Generic),
            ]
            .into_iter()
            .filter(move |(bit, _)| rising & bit != 0)
            .map(move |(_, button)| CursorClick {
                time: sample.time,
                end,
                x: sample.x,
                y: sample.y,
                button,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_click_edges_keep_their_location_duration_and_button() {
        #[derive(serde::Deserialize)]
        struct Fixture {
            samples: Vec<CursorSample>,
            clicks: Vec<CursorClick>,
        }
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../testdata/cursor-clicks.json")).unwrap();
        let result: Vec<_> = cursor_clicks(&fixture.samples).collect();
        assert_eq!(result.len(), fixture.clicks.len());
        for (actual, expected) in result.iter().zip(fixture.clicks) {
            assert_eq!(
                (actual.button, actual.time, actual.x, actual.y),
                (expected.button, expected.time, expected.x, expected.y)
            );
            assert!((actual.end - expected.end).abs() < 1e-9);
        }
    }

    #[test]
    fn initial_typed_hold_is_a_baseline_and_a_legacy_press_is_neutral() {
        let mut held = CursorSample {
            time: 0.0,
            x: 0.4,
            y: 0.7,
            visible: true,
            pressed: true,
            buttons: 3,
        };
        assert!(cursor_clicks(&[held]).next().is_none());
        held.buttons = 0;
        let pulses: Vec<_> = cursor_clicks(&[held]).collect();
        assert_eq!(pulses.len(), 1);
        assert_eq!(pulses[0].button, ClickButton::Generic);
    }
}
