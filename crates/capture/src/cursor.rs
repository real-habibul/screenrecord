use screenrecord_core::{CaptureRegion, CursorSample};
use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows::Win32::{
    Foundation::POINT,
    UI::{
        Input::KeyboardAndMouse::GetAsyncKeyState,
        WindowsAndMessaging::{
            GetAncestor, GetCursorInfo, GetWindowThreadProcessId, WindowFromPoint, CURSORINFO,
            CURSOR_SHOWING, GA_ROOT,
        },
    },
};

use super::{mouse, qpc_now, Target};

pub(super) fn canvas_size(width: u32, height: u32) -> u64 {
    ((width as u64) << 32) | height as u64
}

pub(super) fn publish_start(
    canvas: &AtomicU64,
    epoch: &AtomicI64,
    width: u32,
    height: u32,
    timestamp: i64,
) {
    canvas.store(canvas_size(width, height), Ordering::Relaxed);
    epoch.store(timestamp, Ordering::Release);
}

pub(super) type Worker = JoinHandle<Result<Vec<CursorSample>, String>>;
pub(super) fn spawn(
    target: Target,
    region: Option<CaptureRegion>,
    canvas: Arc<AtomicU64>,
    directory: PathBuf,
    epoch: Arc<AtomicI64>,
    stop: Arc<AtomicBool>,
    end: Arc<AtomicI64>,
) -> Result<(Worker, Receiver<Result<(), String>>), String> {
    let (ready, initialized) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("cursor-capture".into())
        .spawn(move || {
            let result = record(target, region, canvas, &directory, epoch, stop, end, &ready);
            if let Err(error) = &result {
                let _ = ready.try_send(Err(error.clone()));
            }
            result
        })
        .map_err(|error| error.to_string())?;
    Ok((worker, initialized))
}

fn physical_buttons() -> u8 {
    [(1, 1), (2, 2), (4, 4)]
        .into_iter()
        .fold(0, |buttons, (key, bit)| {
            buttons
                | if unsafe { GetAsyncKeyState(key) } < 0 {
                    bit
                } else {
                    0
                }
        })
}

fn point_position(
    target: Target,
    region: Option<CaptureRegion>,
    canvas: u64,
    point: POINT,
    shown: bool,
) -> Option<(f64, f64)> {
    if !shown {
        return None;
    }
    let rect = target.rect().ok()?;
    let local = (
        point.x as i64 - rect.left as i64,
        point.y as i64 - rect.top as i64,
    );
    let (x, y, inside) = if let Some(region) = region {
        let source = (
            u32::try_from(rect.right as i64 - rect.left as i64).ok()?,
            u32::try_from(rect.bottom as i64 - rect.top as i64).ok()?,
        );
        let (x, y) = super::region::cursor_position(local, source, region)?;
        (x, y, true)
    } else {
        normalized_position(
            i32::try_from(local.0).ok()?,
            i32::try_from(local.1).ok()?,
            (rect.right - rect.left, rect.bottom - rect.top),
            ((canvas >> 32) as u32, canvas as u32),
        )
    };
    if !inside {
        return None;
    }
    let window = unsafe { WindowFromPoint(point) };
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
    }
    let expected = match target {
        Target::Monitor(_) => None,
        Target::Window(handle) => Some(handle),
        Target::App(app) => Some(app.handle),
    };
    let actual_root = unsafe { GetAncestor(window, GA_ROOT) }.0 as usize;
    if !allowed_window(expected, actual_root, pid, std::process::id()) {
        return None;
    }
    Some((x, y))
}

fn allowed_window(expected: Option<usize>, actual_root: usize, pid: u32, own_pid: u32) -> bool {
    pid != own_pid && expected.is_none_or(|handle| handle == actual_root)
}

fn in_recording(qpc: i64, origin: i64, end: i64) -> bool {
    origin > 0 && qpc >= origin && (end == 0 || qpc <= end)
}

#[derive(Default)]
struct Timeline {
    buttons: u8,
    output_buttons: u8,
    last_time: f64,
    last_position: (f64, f64),
    visible: bool,
    baseline: bool,
}
impl Timeline {
    fn initial(buttons: u8) -> (Self, CursorSample) {
        let state = Self {
            buttons,
            output_buttons: buttons,
            baseline: true,
            ..Self::default()
        };
        let sample = state.sample(0.0, (0.0, 0.0), buttons, false);
        (state, sample)
    }
    fn sample(&self, time: f64, point: (f64, f64), buttons: u8, visible: bool) -> CursorSample {
        CursorSample {
            time,
            x: point.0,
            y: point.1,
            pressed: buttons != 0,
            buttons,
            visible,
        }
    }
    fn update(
        &mut self,
        time: f64,
        point: Option<(f64, f64)>,
        buttons: u8,
        edge: bool,
    ) -> [Option<CursorSample>; 2] {
        let before = self.buttons;
        self.buttons = buttons;
        let time = time.max(self.last_time);
        let mut output = [None, None];
        if let Some(point) = point {
            if self.baseline {
                output[0] = Some(self.sample(time, point, before, false));
                self.baseline = false;
            }
            output[1] = Some(self.sample(time, point, buttons, true));
            self.last_position = point;
            self.output_buttons = buttons;
            self.visible = true;
        } else {
            self.baseline = true;
            if !edge && self.visible {
                output[0] = Some(self.sample(time, self.last_position, self.output_buttons, false));
                self.visible = false;
            }
        }
        if output.iter().any(Option::is_some) {
            self.last_time = time;
        }
        output
    }
    fn resynchronize(&mut self, buttons: u8) {
        self.buttons = buttons;
        self.baseline = true;
    }
    #[cfg(test)]
    fn updates(
        &mut self,
        time: f64,
        point: Option<(f64, f64)>,
        buttons: u8,
        edge: bool,
    ) -> Vec<CursorSample> {
        self.update(time, point, buttons, edge)
            .into_iter()
            .flatten()
            .collect()
    }
}

fn pending_edges(
    receiver: &Receiver<mouse::Edge>,
    origin: i64,
) -> impl Iterator<Item = mouse::Edge> + '_ {
    receiver
        .try_iter()
        .take(if origin > 0 { mouse::QUEUE_CAPACITY } else { 0 })
}

fn begin_timeline(
    timeline: &mut Option<Timeline>,
    buttons: u8,
    append: &mut impl FnMut(CursorSample) -> Result<(), String>,
) -> Result<(), String> {
    if timeline.is_none() {
        let (state, initial) = Timeline::initial(buttons);
        append(initial)?;
        *timeline = Some(state);
    }
    Ok(())
}

fn replay_edge(
    timeline: &mut Option<Timeline>,
    buttons: &mut u8,
    event: mouse::Edge,
    origin: i64,
    end: i64,
    position: impl FnOnce() -> Option<(f64, f64)>,
    mut append: impl FnMut(CursorSample) -> Result<(), String>,
) -> Result<(), String> {
    if origin == 0 || (end > 0 && event.qpc > end) {
        return Ok(());
    }
    let before = *buttons;
    *buttons = if event.down {
        before | event.button
    } else {
        before & !event.button
    };
    if event.qpc < origin {
        return Ok(());
    }
    if before == *buttons {
        return Ok(());
    }
    begin_timeline(timeline, before, &mut append)?;
    for sample in timeline
        .as_mut()
        .unwrap()
        .update(
            (event.qpc - origin) as f64 / 10_000_000.0,
            position(),
            *buttons,
            true,
        )
        .into_iter()
        .flatten()
    {
        append(sample)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record(
    target: Target,
    region: Option<CaptureRegion>,
    canvas: Arc<AtomicU64>,
    directory: &Path,
    epoch: Arc<AtomicI64>,
    stop: Arc<AtomicBool>,
    end: Arc<AtomicI64>,
    ready: &mpsc::SyncSender<Result<(), String>>,
) -> Result<Vec<CursorSample>, String> {
    let _dpi = super::PhysicalCoordinates::enter();
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("cursor.ndjson"))
        .map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    let mut samples = Vec::new();
    let mut fallback = None;
    let mut events = match mouse::Events::start(stop.clone()) {
        Ok((events, initialized)) => {
            match initialized.recv_timeout(mouse::STARTUP_DEADLINE) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    fallback = Some(format!("Mouse event observer could not start: {error}"))
                }
                Err(_) => {
                    fallback = Some("Mouse event observer did not initialize in time.".into())
                }
            }
            if fallback.is_some() {
                events.disable();
            }
            Some(events)
        }
        Err(error) => {
            fallback = Some(format!("Mouse event thread could not start: {error}"));
            None
        }
    };
    let mut buttons = physical_buttons();
    let mut timeline: Option<Timeline> = None;
    let mut last_flush = qpc_now();
    let mut mismatch = 0;
    if let Some(reason) = &fallback {
        write_warning(directory, reason)?;
    }
    let _ = ready.try_send(Ok(()));
    loop {
        let stopping = stop.load(Ordering::Acquire);
        if stopping {
            if let Some(events) = &mut events {
                if let Err(error) = events.finish() {
                    write_warning(directory, &error)?;
                }
            }
        }
        let origin = epoch.load(Ordering::Acquire);
        if fallback.is_none()
            && events.as_ref().is_some_and(|events| {
                events.overflow.load(Ordering::Acquire)
                    || (!stopping && !stop.load(Ordering::Acquire) && events.is_finished())
            })
        {
            fallback = Some("Mouse event delivery stopped or its bounded queue overflowed.".into());
            if let Some(events) = &events {
                events.disable();
            }
            write_warning(directory, fallback.as_deref().unwrap())?;
            buttons = physical_buttons();
            if let Some(timeline) = &mut timeline {
                timeline.resynchronize(buttons);
            }
        }
        if origin == 0 {
            if stopping {
                break;
            }
            thread::sleep(Duration::from_millis(8));
            continue;
        }
        if fallback.is_some() && timeline.is_none() {
            buttons = physical_buttons();
        }
        let mut info = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        let cursor_info = unsafe { GetCursorInfo(&mut info) }.is_ok();
        let shown = cursor_info && info.flags == CURSOR_SHOWING;
        if let Some(events) = &events {
            for event in pending_edges(&events.receiver, origin) {
                if fallback.is_some() {
                    continue;
                }
                replay_edge(
                    &mut timeline,
                    &mut buttons,
                    event,
                    origin,
                    end.load(Ordering::Acquire),
                    || {
                        point_position(
                            target,
                            region,
                            canvas.load(Ordering::Acquire),
                            event.point,
                            shown,
                        )
                    },
                    |sample| super::append_cursor(&mut writer, &mut samples, sample),
                )?;
            }
        }
        begin_timeline(&mut timeline, buttons, &mut |sample| {
            super::append_cursor(&mut writer, &mut samples, sample)
        })?;
        if stopping {
            break;
        }
        let polled = physical_buttons();
        if fallback.is_some() {
            buttons = polled;
        } else {
            mismatch = if polled != buttons { mismatch + 1 } else { 0 };
            if mismatch >= 4 {
                fallback = Some("Mouse events disagreed with the current button state.".into());
                if let Some(events) = &events {
                    events.disable();
                }
                write_warning(directory, fallback.as_deref().unwrap())?;
                buttons = polled;
                if let Some(timeline) = &mut timeline {
                    timeline.resynchronize(buttons);
                }
            }
        }
        let position = cursor_info
            .then(|| {
                point_position(
                    target,
                    region,
                    canvas.load(Ordering::Acquire),
                    info.ptScreenPos,
                    shown,
                )
            })
            .flatten();
        let now = qpc_now();
        if let Some(timeline) = &mut timeline {
            if in_recording(now, origin, end.load(Ordering::Acquire)) {
                for sample in timeline
                    .update(
                        (now - origin) as f64 / 10_000_000.0,
                        position,
                        buttons,
                        false,
                    )
                    .into_iter()
                    .flatten()
                {
                    super::append_cursor(&mut writer, &mut samples, sample)?;
                }
            }
        }
        if qpc_now() - last_flush >= 10_000_000 {
            writer.flush().map_err(|error| error.to_string())?;
            writer
                .get_ref()
                .sync_data()
                .map_err(|error| error.to_string())?;
            last_flush = qpc_now();
        }
        thread::sleep(Duration::from_millis(8));
    }
    drop(events);
    writer.flush().map_err(|error| error.to_string())?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| error.to_string())?;
    Ok(samples)
}

fn write_warning(directory: &Path, reason: &str) -> Result<(), String> {
    let warning = format!("{reason} Mouse buttons switched to 8 ms polling, which may miss quick clicks. Video and audio recording continued.");
    log::warn!("{warning}");
    let path = directory.join("cursor-warning.txt");
    super::validate_owned_path(directory, &path)?;
    fs::write(path, warning).map_err(|error| error.to_string())
}

pub(super) fn warning(directory: &Path) -> Result<Option<String>, String> {
    let path = directory.join("cursor-warning.txt");
    if !path.exists() {
        return Ok(None);
    }
    super::validate_owned_path(directory, &path)?;
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 4096 {
        return Err("Cursor warning exceeds its size limit.".into());
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|e| e.to_string())
}

pub(super) fn recovered_sample(sample: CursorSample, duration: f64) -> Option<CursorSample> {
    (sample.time.is_finite()
        && sample.time >= 0.0
        && sample.time <= duration
        && sample.x.is_finite()
        && sample.y.is_finite()
        && sample.buttons <= 7
        && (sample.buttons == 0 || sample.pressed))
        .then_some(CursorSample {
            x: sample.x.clamp(0.0, 1.0),
            y: sample.y.clamp(0.0, 1.0),
            ..sample
        })
}

pub fn normalized_position(
    x: i32,
    y: i32,
    current: (i32, i32),
    canvas: (u32, u32),
) -> (f64, f64, bool) {
    let visible = x >= 0
        && y >= 0
        && x < current.0
        && y < current.1
        && x < canvas.0 as i32
        && y < canvas.1 as i32;
    (
        (x as f64 / canvas.0 as f64).clamp(0.0, 1.0),
        (y as f64 / canvas.1 as f64).clamp(0.0, 1.0),
        visible,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_preserves_original_canvas_coordinates_and_hides_cropped_cursor() {
        assert_eq!(
            normalized_position(480, 270, (960, 540), (1920, 1080)),
            (0.25, 0.25, true)
        );
        assert_eq!(
            normalized_position(2000, 600, (2560, 1440), (1920, 1080)),
            (1.0, 600.0 / 1080.0, false)
        );
        assert!(!normalized_position(1000, 600, (960, 540), (1920, 1080)).2);
    }

    #[test]
    fn short_clicks_and_chords_keep_every_down_up_between_position_polls() {
        let (mut state, initial) = Timeline::initial(0);
        let mut samples = vec![initial];
        for (time, buttons) in [
            (0.001, 1),
            (0.002, 3),
            (0.003, 2),
            (0.004, 6),
            (0.005, 4),
            (0.006, 0),
            (0.007, 1),
            (0.0075, 0),
        ] {
            samples.extend(state.updates(time, Some((time * 10.0, 0.5)), buttons, true));
        }
        assert_eq!(
            samples
                .iter()
                .filter(|s| s.visible)
                .map(|s| s.buttons)
                .collect::<Vec<_>>(),
            [1, 3, 2, 6, 4, 0, 1, 0]
        );
        assert!(samples.iter().all(|s| s.pressed == (s.buttons != 0)));
        assert!(samples.windows(2).all(|pair| pair[0].time <= pair[1].time));
        assert_eq!(samples.last().unwrap().x, 0.075);
        let clicks = screenrecord_core::cursor_clicks(&samples).collect::<Vec<_>>();
        assert_eq!(
            clicks.iter().map(|click| click.time).collect::<Vec<_>>(),
            [0.001, 0.002, 0.004, 0.007]
        );
    }

    #[test]
    fn preheld_and_reentry_state_do_not_fabricate_clicks_or_store_outside_positions() {
        let (mut state, initial) = Timeline::initial(5);
        assert!(!initial.visible);
        assert_eq!(initial.buttons, 5);
        let first = state.updates(0.01, Some((0.2, 0.3)), 5, false);
        assert_eq!(
            first
                .iter()
                .map(|s| (s.buttons, s.visible))
                .collect::<Vec<_>>(),
            [(5, false), (5, true)]
        );
        assert!(state.updates(0.011, None, 0, true).is_empty());
        assert!(state.updates(0.012, None, 2, true).is_empty());
        let hidden = state.updates(0.016, None, 2, false);
        assert_eq!(hidden.len(), 1);
        assert_eq!(
            (
                hidden[0].x,
                hidden[0].y,
                hidden[0].buttons,
                hidden[0].visible
            ),
            (0.2, 0.3, 5, false)
        );
        let reentry = state.updates(0.02, Some((0.8, 0.6)), 2, false);
        assert_eq!(
            reentry
                .iter()
                .map(|s| (s.buttons, s.visible))
                .collect::<Vec<_>>(),
            [(2, false), (2, true)]
        );
        let new_down = state.updates(0.021, Some((0.8, 0.6)), 3, true);
        assert_eq!(new_down.len(), 1);
        assert_eq!(new_down[0].buttons, 3);
        let all = [vec![initial], first, hidden, reentry, new_down].concat();
        let clicks = screenrecord_core::cursor_clicks(&all).collect::<Vec<_>>();
        assert_eq!(clicks.len(), 1);
        assert_eq!(clicks[0].time, 0.021);
    }

    #[test]
    fn fallback_resynchronizes_without_fabricating_an_edge_and_polling_keeps_chords() {
        let (mut state, _) = Timeline::initial(0);
        state.updates(0.01, Some((0.2, 0.3)), 0, false);
        state.resynchronize(3);
        let baseline = state.updates(0.02, Some((0.2, 0.3)), 3, false);
        assert_eq!(
            baseline
                .iter()
                .map(|s| (s.buttons, s.visible))
                .collect::<Vec<_>>(),
            [(3, false), (3, true)]
        );
        let released = state.updates(0.028, Some((0.2, 0.3)), 2, false);
        assert_eq!(released[0].buttons, 2);
        assert!(released[0].pressed);
    }

    #[test]
    fn occluding_windows_and_own_controls_are_hidden_but_child_roots_are_accepted() {
        assert!(allowed_window(Some(42), 42, 7, 8));
        assert!(!allowed_window(Some(42), 43, 7, 8));
        assert!(!allowed_window(Some(42), 42, 8, 8));
        assert!(allowed_window(None, 43, 7, 8));
        assert!(!allowed_window(None, 43, 8, 8));
    }

    #[test]
    fn event_bounds_preserve_queued_pre_stop_edges_and_reject_pause_gap() {
        assert!(!in_recording(100, 0, 0));
        assert!(!in_recording(99, 100, 0));
        assert!(in_recording(101, 100, 0));
        assert!(in_recording(199, 100, 200));
        assert!(in_recording(200, 100, 200));
        assert!(!in_recording(201, 100, 200));
        assert!(!in_recording(250, 300, 0));
        let (resumed, initial) = Timeline::initial(2);
        assert_eq!(resumed.buttons, 2);
        assert_eq!((initial.time, initial.visible), (0.0, false));
    }

    #[test]
    fn cursor_journal_preserves_buttons_and_legacy_unknown_state() {
        let (mut state, initial) = Timeline::initial(0);
        let mut samples = vec![];
        let mut bytes = vec![];
        super::super::append_cursor(&mut bytes, &mut samples, initial).unwrap();
        for sample in state.updates(0.002, Some((0.4, 0.6)), 4, true) {
            super::super::append_cursor(&mut bytes, &mut samples, sample).unwrap();
        }
        let decoded = String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<CursorSample>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(decoded.last().unwrap().buttons, 4);
        assert!(decoded.last().unwrap().pressed);
        let old: CursorSample =
            serde_json::from_str(r#"{"time":0,"x":0.5,"y":0.5,"pressed":true,"visible":true}"#)
                .unwrap();
        assert_eq!(old.buttons, 0);
        assert!(old.pressed);
    }

    #[test]
    fn hidden_cursor_never_maps_an_event_or_creates_a_visible_click() {
        assert_eq!(
            point_position(
                Target::Monitor(0),
                None,
                canvas_size(1920, 1080),
                POINT::default(),
                false
            ),
            None
        );
        let (mut state, _) = Timeline::initial(0);
        assert!(state.updates(0.001, None, 1, true).is_empty());
        assert!(state.updates(0.002, None, 0, true).is_empty());
        let shown = state.updates(0.008, Some((0.5, 0.5)), 0, false);
        assert!(shown.iter().all(|sample| !sample.pressed));
    }

    #[test]
    fn metadata_limit_rejects_extra_edge_before_writing_or_growing_memory() {
        let (_, sample) = Timeline::initial(0);
        let mut samples = vec![];
        let mut bytes = vec![];
        for _ in 0..2 {
            super::super::append_cursor_limited(&mut bytes, &mut samples, sample, 2).unwrap();
        }
        let before = bytes.clone();
        assert!(super::super::append_cursor_limited(&mut bytes, &mut samples, sample, 2).is_err());
        assert_eq!(bytes, before);
        assert_eq!(samples.len(), 2);
    }

    #[test]
    fn fallback_warning_is_recoverable_and_bounded() {
        let directory = tempfile::tempdir().unwrap();
        write_warning(directory.path(), "Injected observer failure.").unwrap();
        let stored = warning(directory.path()).unwrap().unwrap();
        assert!(stored.contains("8 ms polling"));
        assert!(stored.contains("Video and audio recording continued"));
        fs::write(
            directory.path().join("cursor-warning.txt"),
            vec![b'x'; 4097],
        )
        .unwrap();
        assert!(warning(directory.path()).is_err());
    }

    #[test]
    fn malformed_recovery_button_metadata_is_omitted_without_rejecting_good_samples() {
        let (_, sample) = Timeline::initial(1);
        assert!(recovered_sample(sample, 1.0).is_some());
        assert!(recovered_sample(
            CursorSample {
                buttons: 8,
                ..sample
            },
            1.0
        )
        .is_none());
        assert!(recovered_sample(
            CursorSample {
                pressed: false,
                ..sample
            },
            1.0
        )
        .is_none());
        let legacy = recovered_sample(
            CursorSample {
                buttons: 0,
                ..sample
            },
            1.0,
        )
        .unwrap();
        assert!(legacy.pressed);
        assert_eq!(legacy.buttons, 0);
        assert!(recovered_sample(
            CursorSample {
                time: 1.1,
                ..sample
            },
            1.0
        )
        .is_none());
    }

    #[test]
    fn unpublished_first_frame_keeps_queued_edges_and_uses_its_actual_canvas() {
        let (sender, receiver) = mpsc::sync_channel(mouse::QUEUE_CAPACITY);
        for (qpc, button, down, point) in [
            (9_900_000, 1, false, POINT { x: -200, y: -300 }),
            (9_990_000, 2, true, POINT { x: -250, y: -350 }),
            (10_010_000, 1, true, POINT { x: 480, y: 270 }),
            (10_020_000, 1, false, POINT { x: 490, y: 275 }),
            (10_030_000, 2, false, POINT { x: 495, y: 280 }),
        ] {
            sender
                .send(mouse::Edge {
                    qpc,
                    point,
                    button,
                    down,
                })
                .unwrap();
        }
        let epoch = Arc::new(AtomicI64::new(0));
        let canvas = Arc::new(AtomicU64::new(canvas_size(1920, 1080)));
        let mut buttons = 1;
        let mut timeline = None;
        let mut samples = vec![];
        let mut journal = vec![];
        assert_eq!(
            pending_edges(&receiver, epoch.load(Ordering::Acquire)).count(),
            0
        );
        assert!(journal.is_empty());
        let publisher_epoch = epoch.clone();
        let publisher_canvas = canvas.clone();
        let publisher = thread::spawn(move || {
            publish_start(&publisher_canvas, &publisher_epoch, 960, 540, 10_000_000)
        });
        let origin = loop {
            let origin = epoch.load(Ordering::Acquire);
            if origin != 0 {
                break origin;
            }
            thread::yield_now();
        };
        let actual_canvas = canvas.load(Ordering::Relaxed);
        assert_eq!(actual_canvas, canvas_size(960, 540));
        let mut mapped = 0;
        for edge in pending_edges(&receiver, origin) {
            replay_edge(
                &mut timeline,
                &mut buttons,
                edge,
                origin,
                0,
                || {
                    assert!(
                        edge.qpc >= origin,
                        "Pre-record positions must not be mapped or persisted."
                    );
                    mapped += 1;
                    let (x, y, visible) = normalized_position(
                        edge.point.x,
                        edge.point.y,
                        (960, 540),
                        ((actual_canvas >> 32) as u32, actual_canvas as u32),
                    );
                    visible.then_some((x, y))
                },
                |sample| super::super::append_cursor(&mut journal, &mut samples, sample),
            )
            .unwrap();
        }
        publisher.join().unwrap();
        assert_eq!(mapped, 3);
        assert_eq!(
            (samples[0].time, samples[0].buttons, samples[0].visible),
            (0.0, 2, false)
        );
        assert!(samples
            .iter()
            .all(|sample| sample.time >= 0.0 && sample.x >= 0.0 && sample.y >= 0.0));
        let clicks = screenrecord_core::cursor_clicks(&samples).collect::<Vec<_>>();
        assert_eq!(clicks.len(), 1);
        assert_eq!(
            (clicks[0].time, clicks[0].x, clicks[0].y),
            (0.001, 0.5, 0.5)
        );
        assert_eq!(buttons, 0);
    }
}
