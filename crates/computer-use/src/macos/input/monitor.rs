use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult,
};

use crate::lease::InputLeaseCoordinator;
use crate::ComputerUseError;

use super::EVENT_TAG;

const START_TIMEOUT: Duration = Duration::from_millis(750);

#[derive(Clone)]
pub struct PhysicalInputMonitor {
    state: Arc<(Mutex<MonitorState>, Condvar)>,
    leases: InputLeaseCoordinator,
}

#[derive(Clone, Debug)]
enum MonitorState {
    Idle,
    Starting,
    Ready,
    Failed(String),
}

impl PhysicalInputMonitor {
    pub fn new(leases: InputLeaseCoordinator) -> Self {
        Self {
            state: Arc::new((Mutex::new(MonitorState::Idle), Condvar::new())),
            leases,
        }
    }

    pub fn ensure_ready(&self) -> Result<(), ComputerUseError> {
        let (lock, condition) = self.state.as_ref();
        let mut state = lock
            .lock()
            .map_err(|_| ComputerUseError::TakeoverMonitorUnavailable)?;
        if matches!(*state, MonitorState::Idle | MonitorState::Failed(_)) {
            *state = MonitorState::Starting;
            self.spawn();
        }
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            match &*state {
                MonitorState::Ready => return Ok(()),
                MonitorState::Failed(reason) => {
                    return Err(ComputerUseError::Os(format!(
                        "physical-input monitor failed: {reason}"
                    )))
                }
                MonitorState::Idle | MonitorState::Starting => {}
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(ComputerUseError::TakeoverMonitorUnavailable);
            }
            let (next, _) = condition
                .wait_timeout(state, deadline.saturating_duration_since(now))
                .map_err(|_| ComputerUseError::TakeoverMonitorUnavailable)?;
            state = next;
        }
    }

    fn spawn(&self) {
        let shared_state = self.state.clone();
        let leases = self.leases.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("clark-physical-input-monitor".to_string())
            .spawn(move || {
                let callback_leases = leases.clone();
                let result = CGEventTap::with_enabled(
                    CGEventTapLocation::HID,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::ListenOnly,
                    monitored_events(),
                    move |_proxy, _kind, event| {
                        let tag = event.get_integer_value_field(
                            core_graphics::event::EventField::EVENT_SOURCE_USER_DATA,
                        );
                        if tag != EVENT_TAG {
                            callback_leases.mark_user_takeover();
                        }
                        CallbackResult::Keep
                    },
                    || {
                        set_state(&shared_state, MonitorState::Ready);
                        CFRunLoop::run_current();
                    },
                );
                if result.is_err() {
                    set_state(
                        &shared_state,
                        MonitorState::Failed(
                            "macOS refused the listen-only HID event tap".to_string(),
                        ),
                    );
                }
            })
        {
            set_state(
                &self.state,
                MonitorState::Failed(format!("could not spawn monitor thread: {error}")),
            );
        }
    }
}

fn set_state(shared: &Arc<(Mutex<MonitorState>, Condvar)>, next: MonitorState) {
    if let Ok(mut state) = shared.0.lock() {
        *state = next;
        shared.1.notify_all();
    }
}

fn monitored_events() -> Vec<CGEventType> {
    vec![
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        CGEventType::MouseMoved,
        CGEventType::LeftMouseDragged,
        CGEventType::RightMouseDragged,
        CGEventType::KeyDown,
        CGEventType::KeyUp,
        CGEventType::FlagsChanged,
        CGEventType::ScrollWheel,
        CGEventType::OtherMouseDown,
        CGEventType::OtherMouseUp,
        CGEventType::OtherMouseDragged,
    ]
}
