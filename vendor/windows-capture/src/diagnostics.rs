use std::sync::atomic::{AtomicI32, AtomicU8, AtomicU64, Ordering};

/// Metadata-only checkpoints; reading these never waits for a capture callback.
#[derive(Default)]
pub struct CaptureDiagnostics {
    startup: AtomicU8,
    frame: AtomicU8,
    events: AtomicU64,
    delivered: AtomicU64,
    error: AtomicI32,
}

/// The latest independent startup and frame-processing checkpoints.
#[derive(Clone, Copy, Debug)]
pub struct CaptureDiagnosticsSnapshot {
    /// Last completed or entered native startup operation.
    pub startup_stage: &'static str,
    /// Last entered frame processing operation, before the application callback.
    pub frame_stage: &'static str,
    /// Number of native FrameArrived callbacks entered.
    pub frame_events: u64,
    /// Number of calls into the application frame handler.
    pub delivered_frames: u64,
    /// Most recent failing native HRESULT, if any.
    pub last_hresult: Option<i32>,
}

impl CaptureDiagnostics {
    /// Returns atomic metadata even if a native call or callback mutex is blocked.
    pub fn snapshot(&self) -> CaptureDiagnosticsSnapshot {
        let startup = [
            "not_started",
            "winrt",
            "message_queue",
            "d3d_device",
            "capture_item",
            "frame_pool",
            "capture_session",
            "event_handlers",
            "session_configuration",
            "start_capture",
            "message_loop",
            "stopping",
            "stopped",
        ];
        let frame = [
            "no_event",
            "event_entered",
            "acquire_frame",
            "content_size",
            "recreate_pool",
            "surface",
            "texture",
            "callback_lock",
            "application_handler",
            "handler_returned",
            "native_error",
            "empty_content",
        ];
        let error = self.error.load(Ordering::Acquire);
        CaptureDiagnosticsSnapshot {
            startup_stage: startup.get(self.startup.load(Ordering::Acquire) as usize).copied().unwrap_or("unknown"),
            frame_stage: frame.get(self.frame.load(Ordering::Acquire) as usize).copied().unwrap_or("unknown"),
            frame_events: self.events.load(Ordering::Acquire),
            delivered_frames: self.delivered.load(Ordering::Acquire),
            last_hresult: (error != 0).then_some(error),
        }
    }
    pub(crate) fn startup(&self, stage: u8) {
        self.startup.store(stage, Ordering::Release);
    }
    pub(crate) fn frame(&self, stage: u8) {
        self.frame.store(stage, Ordering::Release);
    }
    pub(crate) fn arrived(&self) {
        self.events.fetch_add(1, Ordering::AcqRel);
        self.frame(1);
    }
    pub(crate) fn delivered(&self) {
        self.delivered.fetch_add(1, Ordering::AcqRel);
        self.frame(8);
    }
    pub(crate) fn record_error(&self, error: i32) {
        self.error.store(error, Ordering::Release);
    }
    pub(crate) fn error(&self, error: i32) {
        self.record_error(error);
        self.frame(10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkpoints_distinguish_absent_events_native_stalls_and_handler_delivery() {
        let state = CaptureDiagnostics::default();
        state.startup(10);
        assert_eq!(state.snapshot().frame_events, 0);
        state.arrived();
        state.frame(2);
        let blocked = state.snapshot();
        assert_eq!(blocked.frame_stage, "acquire_frame");
        state.frame(11);
        assert_eq!(state.snapshot().frame_stage, "empty_content");
        assert_eq!(blocked.frame_events, 1);
        assert_eq!(blocked.delivered_frames, 0);
        state.delivered();
        state.error(0x80004005u32 as i32);
        let failed = state.snapshot();
        assert_eq!(failed.delivered_frames, 1);
        assert_eq!(failed.last_hresult, Some(0x80004005u32 as i32));
        assert_eq!(failed.startup_stage, "message_loop");
    }
}
