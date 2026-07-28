use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::lease::InputLeaseCoordinator;
use crate::ComputerUseError;

const START_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(target_os = "linux")]
const SYNTHETIC_SETTLE_TIMEOUT: Duration = Duration::from_millis(250);
pub(super) const EVENT_TAG: usize = 0x4b43_5531;

#[derive(Clone)]
pub struct PhysicalInputMonitor {
    state: Arc<(Mutex<MonitorState>, Condvar)>,
    expected: Arc<(Mutex<VecDeque<InputEventKind>>, Condvar)>,
    leases: InputLeaseCoordinator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InputEventKind {
    KeyPress,
    KeyRelease,
    ButtonPress,
    ButtonRelease,
    Motion,
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
            expected: Arc::new((Mutex::new(VecDeque::new()), Condvar::new())),
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
                    )));
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
        let state = self.state.clone();
        let expected = self.expected.clone();
        let leases = self.leases.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("clark-physical-input-monitor".to_string())
            .spawn(move || {
                if let Err(error) = platform_monitor(leases, state.clone(), expected) {
                    set_state(&state, MonitorState::Failed(error));
                }
            })
        {
            set_state(
                &self.state,
                MonitorState::Failed(format!("could not spawn monitor thread: {error}")),
            );
        }
    }

    pub(super) fn expect(&self, kind: InputEventKind) -> Result<(), ComputerUseError> {
        #[cfg(target_os = "linux")]
        {
            self.expected
                .0
                .lock()
                .map_err(|_| ComputerUseError::TakeoverMonitorUnavailable)?
                .push_back(kind);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = kind;
        Ok(())
    }

    pub(super) fn settle_expected(&self) -> Result<(), ComputerUseError> {
        #[cfg(target_os = "linux")]
        {
            let deadline = Instant::now() + SYNTHETIC_SETTLE_TIMEOUT;
            let mut expected = self
                .expected
                .0
                .lock()
                .map_err(|_| ComputerUseError::TakeoverMonitorUnavailable)?;
            while !expected.is_empty() {
                let now = Instant::now();
                if now >= deadline {
                    expected.clear();
                    return Err(ComputerUseError::TakeoverMonitorUnavailable);
                }
                let (next, _) = self
                    .expected
                    .1
                    .wait_timeout(expected, deadline.saturating_duration_since(now))
                    .map_err(|_| ComputerUseError::TakeoverMonitorUnavailable)?;
                expected = next;
            }
        }
        Ok(())
    }

    pub(super) fn clear_expected(&self) {
        if let Ok(mut expected) = self.expected.0.lock() {
            expected.clear();
            self.expected.1.notify_all();
        }
    }
}

fn set_state(shared: &Arc<(Mutex<MonitorState>, Condvar)>, next: MonitorState) {
    if let Ok(mut state) = shared.0.lock() {
        *state = next;
        shared.1.notify_all();
    }
}

#[cfg(target_os = "windows")]
fn platform_monitor(
    leases: InputLeaseCoordinator,
    state: Arc<(Mutex<MonitorState>, Condvar)>,
    _expected: Arc<(Mutex<VecDeque<InputEventKind>>, Condvar)>,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::OnceLock;
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG,
        MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_MOUSEMOVE,
    };

    static LEASES: OnceLock<InputLeaseCoordinator> = OnceLock::new();
    static LAST_SYNTHETIC_POINT: AtomicU64 = AtomicU64::new(u64::MAX);
    LEASES
        .set(leases)
        .map_err(|_| "Windows hook coordinator was already installed".to_string())?;

    unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            if event.dwExtraInfo != EVENT_TAG {
                if let Some(leases) = LEASES.get() {
                    leases.mark_user_takeover();
                }
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    unsafe extern "system" fn mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            let point = ((event.pt.x as u32 as u64) << 32) | event.pt.y as u32 as u64;
            if event.dwExtraInfo == EVENT_TAG {
                LAST_SYNTHETIC_POINT.store(point, Ordering::Relaxed);
            } else if wparam.0 != WM_MOUSEMOVE as usize
                || LAST_SYNTHETIC_POINT.load(Ordering::Relaxed) != point
            {
                if let Some(leases) = LEASES.get() {
                    leases.mark_user_takeover();
                }
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    let keyboard_hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), None, 0) }
        .map_err(|error| format!("keyboard hook failed: {error}"))?;
    let mouse_hook = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), None, 0) } {
        Ok(hook) => hook,
        Err(error) => {
            unsafe {
                let _ = UnhookWindowsHookEx(keyboard_hook);
            }
            return Err(format!("mouse hook failed: {error}"));
        }
    };
    set_state(&state, MonitorState::Ready);
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {}
    unsafe {
        let _ = UnhookWindowsHookEx(keyboard_hook);
        let _ = UnhookWindowsHookEx(mouse_hook);
    }
    Err("Windows hook message loop stopped".to_string())
}

#[cfg(target_os = "linux")]
fn platform_monitor(
    leases: InputLeaseCoordinator,
    state: Arc<(Mutex<MonitorState>, Condvar)>,
    expected: Arc<(Mutex<VecDeque<InputEventKind>>, Condvar)>,
) -> Result<(), String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xinput::{ConnectionExt as _, Device, EventMask, XIEventMask};
    use x11rb::protocol::Event;

    let (connection, screen) =
        x11rb::connect(None).map_err(|error| format!("X11 connection failed: {error}"))?;
    connection
        .xinput_xi_query_version(2, 0)
        .map_err(|error| format!("XInput2 query failed: {error}"))?
        .reply()
        .map_err(|error| format!("XInput2 is unavailable: {error}"))?;
    let devices = connection
        .xinput_xi_query_device(Device::ALL)
        .map_err(|error| format!("XInput2 device query failed: {error}"))?
        .reply()
        .map_err(|error| format!("XInput2 device reply failed: {error}"))?
        .infos;
    let synthetic_devices = devices
        .iter()
        .filter(|device| {
            String::from_utf8_lossy(&device.name)
                .to_ascii_lowercase()
                .contains("xtest")
        })
        .map(|device| device.deviceid)
        .collect::<std::collections::HashSet<_>>();
    if synthetic_devices.is_empty() {
        return Err("XInput2 did not expose synthetic XTEST devices".to_string());
    }
    let mask = XIEventMask::RAW_KEY_PRESS
        | XIEventMask::RAW_KEY_RELEASE
        | XIEventMask::RAW_BUTTON_PRESS
        | XIEventMask::RAW_BUTTON_RELEASE
        | XIEventMask::RAW_MOTION;
    let root = connection.setup().roots[screen].root;
    connection
        .xinput_xi_select_events(
            root,
            &[EventMask {
                deviceid: u16::from(Device::ALL),
                mask: vec![mask],
            }],
        )
        .map_err(|error| format!("XInput2 selection failed: {error}"))?
        .check()
        .map_err(|error| format!("XInput2 selection was rejected: {error}"))?;
    connection
        .flush()
        .map_err(|error| format!("XInput2 monitor flush failed: {error}"))?;
    set_state(&state, MonitorState::Ready);
    loop {
        let event = connection
            .wait_for_event()
            .map_err(|error| format!("XInput2 event loop failed: {error}"))?;
        let input = match event {
            Event::XinputRawKeyPress(event) => {
                Some((event.deviceid, event.sourceid, InputEventKind::KeyPress))
            }
            Event::XinputRawKeyRelease(event) => {
                Some((event.deviceid, event.sourceid, InputEventKind::KeyRelease))
            }
            Event::XinputRawButtonPress(event) => {
                Some((event.deviceid, event.sourceid, InputEventKind::ButtonPress))
            }
            Event::XinputRawButtonRelease(event) => Some((
                event.deviceid,
                event.sourceid,
                InputEventKind::ButtonRelease,
            )),
            Event::XinputRawMotion(event) => {
                Some((event.deviceid, event.sourceid, InputEventKind::Motion))
            }
            _ => None,
        };
        let Some((device, source, kind)) = input else {
            continue;
        };
        if device != source {
            continue;
        }
        let is_expected = if synthetic_devices.contains(&source) {
            let mut queue = expected
                .0
                .lock()
                .map_err(|_| "expected-input queue was poisoned".to_string())?;
            if queue.front().copied() == Some(kind) {
                queue.pop_front();
                expected.1.notify_all();
                true
            } else {
                false
            }
        } else {
            false
        };
        if !is_expected {
            leases.mark_user_takeover();
        }
    }
}
