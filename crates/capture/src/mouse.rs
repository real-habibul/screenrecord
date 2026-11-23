use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows::Win32::{
    Foundation::{HINSTANCE, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetSystemMetrics, MsgWaitForMultipleObjectsEx,
        PeekMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK, MSG,
        MSLLHOOKSTRUCT, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, SM_SWAPBUTTON, WH_MOUSE_LL,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_QUIT, WM_RBUTTONDOWN,
        WM_RBUTTONUP,
    },
};

pub(super) const QUEUE_CAPACITY: usize = 512;

#[derive(Clone, Copy, Debug)]
pub(super) struct Edge {
    pub qpc: i64,
    pub point: POINT,
    pub button: u8,
    pub down: bool,
}

fn button_message(message: u32, swapped: bool) -> Option<(u8, bool)> {
    let (button, down) = match message {
        WM_LBUTTONDOWN => Some((1, true)),
        WM_LBUTTONUP => Some((1, false)),
        WM_RBUTTONDOWN => Some((2, true)),
        WM_RBUTTONUP => Some((2, false)),
        WM_MBUTTONDOWN => Some((4, true)),
        WM_MBUTTONUP => Some((4, false)),
        _ => None,
    }?;
    Some((physical_button(button, swapped), down))
}

fn physical_button(button: u8, swapped: bool) -> u8 {
    if swapped && button < 4 {
        button ^ 3
    } else {
        button
    }
}

struct Sink {
    sender: SyncSender<Edge>,
    overflow: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    session_stop: Arc<AtomicBool>,
}
impl Sink {
    fn push(&self, edge: Edge) {
        if !self.stop.load(Ordering::Acquire)
            && !self.session_stop.load(Ordering::Acquire)
            && matches!(self.sender.try_send(edge), Err(TrySendError::Full(_)))
        {
            self.overflow.store(true, Ordering::Release);
        }
    }
}
thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

unsafe extern "system" fn callback(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code == 0 {
        if let Some((button, down)) = button_message(message.0 as u32, false) {
            let button = physical_button(button, unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0);
            let event = unsafe { &*(data.0 as *const MSLLHOOKSTRUCT) };
            SINK.with(|slot| {
                if let Ok(sink) = slot.try_borrow() {
                    if let Some(sink) = sink.as_ref() {
                        sink.push(Edge {
                            qpc: super::qpc_now(),
                            point: event.pt,
                            button,
                            down,
                        });
                    }
                }
            });
        }
    }
    unsafe { CallNextHookEx(None, code, message, data) }
}

struct Hook(HHOOK);
impl Drop for Hook {
    fn drop(&mut self) {
        if let Err(error) = unsafe { UnhookWindowsHookEx(self.0) } {
            log::warn!("Cannot remove mouse observer before its owning thread exits: {error}");
        }
        SINK.with(|slot| *slot.borrow_mut() = None);
    }
}

pub(super) struct Events {
    pub receiver: Receiver<Edge>,
    pub overflow: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Events {
    pub fn start(
        session_stop: Arc<AtomicBool>,
    ) -> Result<(Self, Receiver<Result<(), String>>), String> {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let (ready, initialized) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let overflow = Arc::new(AtomicBool::new(false));
        let sink = Sink {
            sender,
            overflow: overflow.clone(),
            stop: stop.clone(),
            session_stop: session_stop.clone(),
        };
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("mouse-button-observer".into())
            .spawn(move || {
                let _dpi = super::PhysicalCoordinates::enter();
                let installed = (|| {
                    let module = unsafe { GetModuleHandleW(None) }.map_err(|e| e.to_string())?;
                    let handle = unsafe {
                        SetWindowsHookExW(WH_MOUSE_LL, Some(callback), Some(HINSTANCE(module.0)), 0)
                    }
                    .map_err(|e| e.to_string())?;
                    SINK.with(|slot| *slot.borrow_mut() = Some(sink));
                    Ok::<_, String>(Hook(handle))
                })();
                let hook = match installed {
                    Ok(hook) => hook,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                let _ = ready.send(Ok(()));
                while !worker_stop.load(Ordering::Acquire) && !session_stop.load(Ordering::Acquire)
                {
                    let mut message = MSG::default();
                    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                        if worker_stop.load(Ordering::Acquire)
                            || session_stop.load(Ordering::Acquire)
                        {
                            break;
                        }
                        if message.message == WM_QUIT {
                            worker_stop.store(true, Ordering::Release);
                            break;
                        }
                        unsafe {
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    if !worker_stop.load(Ordering::Acquire) {
                        unsafe {
                            MsgWaitForMultipleObjectsEx(
                                None,
                                100,
                                QS_ALLINPUT,
                                MWMO_INPUTAVAILABLE,
                            );
                        }
                    }
                }
                drop(hook);
            })
            .map_err(|e| e.to_string())?;
        Ok((
            Self {
                receiver,
                overflow,
                stop,
                worker: Some(worker),
            },
            initialized,
        ))
    }

    pub fn disable(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub fn finish(&mut self) -> Result<(), String> {
        self.disable();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "Mouse event observer panicked.".to_owned())?;
        }
        Ok(())
    }
}
impl Drop for Events {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            log::warn!("{error}");
        }
    }
}

pub(super) const STARTUP_DEADLINE: Duration = Duration::from_secs(2);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_buttons_have_independent_edges_and_other_input_is_ignored() {
        for (down, up, bit) in [
            (WM_LBUTTONDOWN, WM_LBUTTONUP, 1),
            (WM_RBUTTONDOWN, WM_RBUTTONUP, 2),
            (WM_MBUTTONDOWN, WM_MBUTTONUP, 4),
        ] {
            assert_eq!(button_message(down, false), Some((bit, true)));
            assert_eq!(button_message(up, false), Some((bit, false)));
            let physical = if bit < 4 { bit ^ 3 } else { bit };
            assert_eq!(button_message(down, true), Some((physical, true)));
            assert_eq!(button_message(up, true), Some((physical, false)));
        }
        for message in [0, 0x100, 0x101, 0x200, 0x20a, 0x20b] {
            assert_eq!(button_message(message, false), None);
        }
    }

    #[test]
    fn bounded_event_queue_reports_loss_without_blocking_or_growing() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let sink = Sink {
            sender,
            overflow: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            session_stop: Arc::new(AtomicBool::new(false)),
        };
        for qpc in 0..10 {
            sink.push(Edge {
                qpc,
                point: POINT { x: 5, y: 6 },
                button: 1,
                down: qpc % 2 == 0,
            });
        }
        assert!(sink.overflow.load(Ordering::Acquire));
        assert_eq!(
            receiver.try_iter().map(|edge| edge.qpc).collect::<Vec<_>>(),
            [0, 1]
        );
        sink.stop.store(true, Ordering::Release);
        sink.push(Edge {
            qpc: 11,
            point: POINT::default(),
            button: 2,
            down: true,
        });
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn stop_joins_owned_observer_before_draining_its_last_accepted_edges() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                thread::yield_now();
            }
            sender
                .send(Edge {
                    qpc: 100,
                    point: POINT { x: 1, y: 2 },
                    button: 4,
                    down: true,
                })
                .unwrap();
            sender
                .send(Edge {
                    qpc: 101,
                    point: POINT { x: 1, y: 2 },
                    button: 4,
                    down: false,
                })
                .unwrap();
        });
        let mut events = Events {
            receiver,
            overflow: Arc::new(AtomicBool::new(false)),
            stop,
            worker: Some(worker),
        };
        events.finish().unwrap();
        assert!(events.is_finished());
        assert_eq!(
            events
                .receiver
                .try_iter()
                .map(|edge| (edge.qpc, edge.down))
                .collect::<Vec<_>>(),
            [(100, true), (101, false)]
        );
        events.finish().unwrap();
    }
}
