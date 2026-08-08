use std::collections::{HashMap, VecDeque};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use enigo::{Button, Coordinate, Direction, Enigo, Key as EnigoKey, Keyboard, Mouse, Settings};

use crate::{
    assess_proposed_action, ensure_bundle_allowed, ensure_window_allowed, ActionAuthorization,
    ActionDisposition, ActionReceipt, ApprovalStore, ClickRequest, ComputerAction, ComputerBackend,
    ComputerUseError, Key, KeyPressRequest, Modifier, Observation, PermissionRequest,
    PermissionStatus, Point, PrepareActionRequest, PreparedAction, ReceiptOutcome,
    RedactedActionPreview, TypeTextRequest, WindowFilter, WindowInfo, WindowTarget,
};

const OBSERVATION_TTL: Duration = Duration::from_secs(30);
const PREPARED_TTL_MS: u64 = 60_000;
const INPUT_WINDOW: Duration = Duration::from_secs(1);
const MAX_INPUTS_PER_WINDOW: usize = 8;

#[derive(Clone)]
struct LatestObservation {
    id: String,
    observed_at: Instant,
    window: WindowInfo,
    screenshot_width: u32,
    screenshot_height: u32,
}

struct PreparedRecord {
    public: PreparedAction,
    request: PrepareActionRequest,
    observation: LatestObservation,
    authorization: Option<ActionAuthorization>,
}

#[derive(Default)]
struct BackendState {
    latest: HashMap<WindowTarget, LatestObservation>,
    prepared: HashMap<String, PreparedRecord>,
    input_times: HashMap<WindowTarget, VecDeque<Instant>>,
}

#[derive(Clone)]
pub struct PortableNativeBackend {
    state: Arc<Mutex<BackendState>>,
    approvals: ApprovalStore,
    leases: crate::lease::InputLeaseCoordinator,
    input_monitor: super::input_monitor::PhysicalInputMonitor,
}

impl PortableNativeBackend {
    pub fn new(approvals: ApprovalStore) -> Self {
        let leases = crate::lease::InputLeaseCoordinator::default();
        Self {
            state: Arc::new(Mutex::new(BackendState::default())),
            approvals,
            input_monitor: super::input_monitor::PhysicalInputMonitor::new(leases.clone()),
            leases,
        }
    }

    fn resolve_window(
        &self,
        target: &WindowTarget,
    ) -> Result<(xcap::Window, WindowInfo), ComputerUseError> {
        ensure_bundle_allowed(&target.bundle_id)?;
        for window in xcap::Window::all().map_err(os_error)? {
            if window.id().map_err(os_error)? == target.window_id
                && window.pid().map_err(os_error)? == target.pid as u32
            {
                let info = window_info(&window)?;
                if info.target.bundle_id != target.bundle_id {
                    return Err(ComputerUseError::TargetChanged(
                        "the target application's platform identity changed".to_string(),
                    ));
                }
                ensure_window_allowed(&info)?;
                return Ok((window, info));
            }
        }
        Err(ComputerUseError::WindowNotFound(format!(
            "{}:{}",
            target.pid, target.window_id
        )))
    }

    fn consume_observation(
        &self,
        target: &WindowTarget,
        observation_id: &str,
    ) -> Result<LatestObservation, ComputerUseError> {
        let (_, current) = self.resolve_window(target)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        let latest = state
            .latest
            .get(target)
            .ok_or(ComputerUseError::ObservationRequired)?;
        if latest.id != observation_id
            || latest.observed_at.elapsed() > OBSERVATION_TTL
            || frame_changed(latest.window.frame, current.frame)
        {
            return Err(ComputerUseError::ObservationStale);
        }
        state
            .latest
            .remove(target)
            .ok_or(ComputerUseError::ObservationRequired)
    }

    fn prepare_impl(
        &self,
        request: PrepareActionRequest,
    ) -> Result<PreparedAction, ComputerUseError> {
        let observation = self.consume_observation(&request.window, &request.observation_id)?;
        validate_geometry(&observation, &request.action)?;
        reject_semantic_only_action(&request.action)?;
        let application = super::auth::application_identity(
            request.window.pid as u32,
            &request.window.bundle_id,
        )?;
        let (durable_grant, approval_revision) = self.approvals.is_granted(&application)?;
        let assessment = assess_proposed_action(
            &observation.window,
            &application,
            &[],
            &request.intent,
            &request.action,
            request.dry_run,
            durable_grant,
        )?;
        let id = uuid::Uuid::new_v4().to_string();
        let public = PreparedAction {
            id: id.clone(),
            window: request.window.clone(),
            application,
            kind: request.action.kind(),
            assessment,
            preview: redacted_preview(&observation.window, &request.action),
            approval_revision,
            expires_at_ms: now_ms().saturating_add(PREPARED_TTL_MS),
            dry_run: request.dry_run,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        state
            .prepared
            .retain(|_, record| record.public.expires_at_ms >= now_ms());
        state.prepared.insert(
            id,
            PreparedRecord {
                public: public.clone(),
                request,
                observation,
                authorization: None,
            },
        );
        Ok(public)
    }

    fn prepared_impl(&self, id: &str) -> Result<PreparedAction, ComputerUseError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        let record = state
            .prepared
            .get(id)
            .ok_or_else(|| ComputerUseError::PreparedActionNotFound(id.to_string()))?;
        if record.public.expires_at_ms < now_ms() {
            return Err(ComputerUseError::PreparedActionExpired);
        }
        Ok(record.public.clone())
    }

    fn authorize_impl(
        &self,
        id: &str,
        authorization: ActionAuthorization,
    ) -> Result<(), ComputerUseError> {
        if authorization == ActionAuthorization::Denied {
            self.state
                .lock()
                .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?
                .prepared
                .remove(id);
            return Ok(());
        }
        let prepared = self.prepared_impl(id)?;
        match prepared.assessment.disposition {
            ActionDisposition::Deny => {
                return Err(ComputerUseError::ActionDenied(prepared.assessment.reason));
            }
            ActionDisposition::MandatoryHandoff => {
                return Err(ComputerUseError::HumanHandoffRequired(
                    prepared.assessment.reason,
                ));
            }
            ActionDisposition::ActionTimeConfirmation
            | ActionDisposition::PreapprovalEligible
            | ActionDisposition::Allow => {}
        }
        if authorization == ActionAuthorization::Durable {
            return Err(ComputerUseError::ApprovalStore(
                "portable targets require action-time approval until their publisher identity is verified"
                    .to_string(),
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        let record = state
            .prepared
            .get_mut(id)
            .ok_or_else(|| ComputerUseError::PreparedActionNotFound(id.to_string()))?;
        record.authorization = Some(authorization);
        Ok(())
    }

    fn commit_impl(&self, id: &str) -> Result<ActionReceipt, ComputerUseError> {
        let record = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?
            .prepared
            .remove(id)
            .ok_or_else(|| ComputerUseError::PreparedActionNotFound(id.to_string()))?;
        if record.public.expires_at_ms < now_ms() {
            return Err(ComputerUseError::PreparedActionExpired);
        }
        authorize_commit(&record)?;
        let (_, current) = self.resolve_window(&record.public.window)?;
        if frame_changed(record.observation.window.frame, current.frame) {
            return Err(ComputerUseError::ObservationStale);
        }
        if record.request.dry_run {
            let mut receipt = receipt(&record, ReceiptOutcome::DryRun);
            receipt.persisted = self.approvals.record_receipt(receipt.clone()).is_ok();
            return Ok(receipt);
        }
        self.reserve_input(&record.public.window)?;
        self.input_monitor.ensure_ready()?;
        let lease = self.leases.begin()?;
        let result = execute_action(&record, &lease, &self.input_monitor);
        self.input_monitor.clear_expected();
        drop(lease);
        let outcome = match &result {
            Ok(()) => ReceiptOutcome::Succeeded,
            Err(ComputerUseError::InputCancelled) => ReceiptOutcome::Cancelled,
            Err(ComputerUseError::UserTakeover) => ReceiptOutcome::UserTakeover,
            Err(_) => ReceiptOutcome::Failed,
        };
        let mut receipt = receipt(&record, outcome);
        receipt.persisted = self.approvals.record_receipt(receipt.clone()).is_ok();
        result?;
        Ok(receipt)
    }

    fn reserve_input(&self, target: &WindowTarget) -> Result<(), ComputerUseError> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        let entries = state.input_times.entry(target.clone()).or_default();
        while entries
            .front()
            .is_some_and(|time| now.duration_since(*time) >= INPUT_WINDOW)
        {
            entries.pop_front();
        }
        if entries.len() >= MAX_INPUTS_PER_WINDOW {
            return Err(ComputerUseError::RateLimited);
        }
        entries.push_back(now);
        Ok(())
    }
}

impl ComputerBackend for PortableNativeBackend {
    fn permissions(&self) -> Result<PermissionStatus, ComputerUseError> {
        let screen_recording = xcap::Window::all().is_ok();
        let accessibility = Enigo::new(&input_settings()).is_ok();
        Ok(PermissionStatus {
            accessibility,
            screen_recording,
            screen_recording_restart_required: false,
        })
    }

    fn request_permissions(
        &self,
        _request: PermissionRequest,
    ) -> Result<PermissionStatus, ComputerUseError> {
        self.permissions()
    }

    fn list_windows(&self, filter: WindowFilter) -> Result<Vec<WindowInfo>, ComputerUseError> {
        if let Some(bundle_id) = filter.bundle_id.as_deref() {
            ensure_bundle_allowed(bundle_id)?;
        }
        let mut windows = Vec::new();
        for window in xcap::Window::all().map_err(os_error)? {
            let Ok(info) = window_info(&window) else {
                continue;
            };
            if ensure_window_allowed(&info).is_err()
                || filter
                    .bundle_id
                    .as_ref()
                    .is_some_and(|bundle| bundle != &info.target.bundle_id)
                || filter.title_contains.as_ref().is_some_and(|fragment| {
                    !info
                        .title
                        .to_ascii_lowercase()
                        .contains(&fragment.to_ascii_lowercase())
                })
            {
                continue;
            }
            windows.push(info);
        }
        Ok(windows)
    }

    fn launch_application(&self, bundle_id: &str) -> Result<(), ComputerUseError> {
        ensure_bundle_allowed(bundle_id)?;
        let mut command = Command::new(bundle_id);
        crate::suppress_portable_console_window(&mut command);
        command
            .spawn()
            .map(|_| ())
            .map_err(|error| ComputerUseError::Os(error.to_string()))
    }

    fn observe(&self, target: &WindowTarget) -> Result<Observation, ComputerUseError> {
        let (window, info) = self.resolve_window(target)?;
        let image = window.capture_image().map_err(os_error)?;
        let (width, height) = image.dimensions();
        let screenshot = crate::Screenshot {
            width,
            height,
            png: crate::encode_rgba_png(width, height, image.as_raw())?,
        };
        let observation_id = uuid::Uuid::new_v4().to_string();
        let latest = LatestObservation {
            id: observation_id.clone(),
            observed_at: Instant::now(),
            window: info.clone(),
            screenshot_width: width,
            screenshot_height: height,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        state
            .latest
            .retain(|_, value| value.observed_at.elapsed() <= OBSERVATION_TTL);
        state.latest.insert(target.clone(), latest);
        state
            .prepared
            .retain(|_, prepared| prepared.public.window != *target);
        Ok(Observation {
            window: info,
            observation_id,
            screenshot,
            elements: Vec::new(),
            accessibility_truncated: true,
            observed_at_ms: now_ms(),
            accessibility_diff: None,
            settlement: crate::ObservationSettlement {
                stable: true,
                elapsed_ms: 0,
                samples: 1,
            },
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> Result<PreparedAction, ComputerUseError> {
        self.prepare_impl(request)
    }

    fn prepared_action(&self, id: &str) -> Result<PreparedAction, ComputerUseError> {
        self.prepared_impl(id)
    }

    fn authorize_action(
        &self,
        id: &str,
        authorization: ActionAuthorization,
    ) -> Result<(), ComputerUseError> {
        self.authorize_impl(id, authorization)
    }

    fn commit_action(&self, id: &str) -> Result<ActionReceipt, ComputerUseError> {
        self.commit_impl(id)
    }

    fn cancel_active(&self) -> Result<crate::CancelAck, ComputerUseError> {
        self.leases.cancel_active()
    }

    fn click(&self, request: ClickRequest) -> Result<(), ComputerUseError> {
        let declared = request.intent.risk;
        let prepared = self.prepare_impl(PrepareActionRequest {
            intent: request.intent,
            window: request.window,
            observation_id: request.observation_id,
            action: ComputerAction::Click {
                element_id: request.element_id,
                point: request.point,
                button: request.button,
            },
            dry_run: request.dry_run,
        })?;
        commit_legacy(self, prepared, declared)
    }

    fn type_text(&self, request: TypeTextRequest) -> Result<(), ComputerUseError> {
        let declared = request.intent.risk;
        let prepared = self.prepare_impl(PrepareActionRequest {
            intent: request.intent,
            window: request.window,
            observation_id: request.observation_id,
            action: ComputerAction::TypeText {
                element_id: request.element_id,
                text: request.text,
                replace: request.replace,
            },
            dry_run: request.dry_run,
        })?;
        commit_legacy(self, prepared, declared)
    }

    fn keypress(&self, request: KeyPressRequest) -> Result<(), ComputerUseError> {
        let declared = request.intent.risk;
        let prepared = self.prepare_impl(PrepareActionRequest {
            intent: request.intent,
            window: request.window,
            observation_id: request.observation_id,
            action: ComputerAction::Keypress {
                key: request.key,
                modifiers: request.modifiers,
            },
            dry_run: request.dry_run,
        })?;
        commit_legacy(self, prepared, declared)
    }
}

fn commit_legacy(
    backend: &PortableNativeBackend,
    prepared: PreparedAction,
    declared: crate::ActionRisk,
) -> Result<(), ComputerUseError> {
    if prepared.assessment.model_underclassified {
        backend.authorize_impl(&prepared.id, ActionAuthorization::Denied)?;
        return Err(ComputerUseError::RiskDeclarationMismatch {
            declared,
            required: prepared.assessment.risk,
            reason: prepared.assessment.reason,
        });
    }
    backend.authorize_impl(&prepared.id, ActionAuthorization::Once)?;
    backend.commit_impl(&prepared.id).map(|_| ())
}

fn window_info(window: &xcap::Window) -> Result<WindowInfo, ComputerUseError> {
    let app_name = window.app_name().map_err(os_error)?;
    if app_name.trim().is_empty() {
        return Err(ComputerUseError::WindowNotFound(
            "window has no platform application identity".to_string(),
        ));
    }
    Ok(WindowInfo {
        target: WindowTarget {
            pid: window.pid().map_err(os_error)? as i32,
            window_id: window.id().map_err(os_error)?,
            bundle_id: app_name.clone(),
        },
        app_name,
        title: window.title().map_err(os_error)?,
        frame: crate::Rect {
            x: window.x().map_err(os_error)? as f64,
            y: window.y().map_err(os_error)? as f64,
            width: window.width().map_err(os_error)? as f64,
            height: window.height().map_err(os_error)? as f64,
        },
        layer: window.z().map_err(os_error)?,
        on_screen: !window.is_minimized().map_err(os_error)?,
    })
}

fn execute_action(
    record: &PreparedRecord,
    lease: &crate::lease::InputLease,
    input_monitor: &super::input_monitor::PhysicalInputMonitor,
) -> Result<(), ComputerUseError> {
    lease.check()?;
    focus_window(&record.observation.window)?;
    lease.check()?;
    let mut enigo = Enigo::new(&input_settings()).map_err(os_error)?;
    match &record.request.action {
        ComputerAction::Click {
            element_id: None,
            point: Some(point),
            button,
        } => {
            let point = screenshot_point(&record.observation, *point)?;
            input_monitor.expect(super::input_monitor::InputEventKind::Motion)?;
            enigo
                .move_mouse(point.x.round() as i32, point.y.round() as i32, Coordinate::Abs)
                .map_err(os_error)?;
            lease.check()?;
            input_monitor.expect(super::input_monitor::InputEventKind::ButtonPress)?;
            input_monitor.expect(super::input_monitor::InputEventKind::ButtonRelease)?;
            enigo
                .button(
                    match button {
                        crate::MouseButton::Left => Button::Left,
                        crate::MouseButton::Right => Button::Right,
                    },
                    Direction::Click,
                )
                .map_err(os_error)?;
        }
        ComputerAction::Drag {
            start,
            end,
            button,
            duration_ms,
        } => {
            let start = screenshot_point(
                &record.observation,
                start.point.ok_or_else(|| {
                    ComputerUseError::InvalidAction("drag start point is missing".to_string())
                })?,
            )?;
            let end = screenshot_point(
                &record.observation,
                end.point.ok_or_else(|| {
                    ComputerUseError::InvalidAction("drag end point is missing".to_string())
                })?,
            )?;
            let button = match button {
                crate::MouseButton::Left => Button::Left,
                crate::MouseButton::Right => Button::Right,
            };
            input_monitor.expect(super::input_monitor::InputEventKind::Motion)?;
            enigo
                .move_mouse(
                    start.x.round() as i32,
                    start.y.round() as i32,
                    Coordinate::Abs,
                )
                .map_err(os_error)?;
            input_monitor.expect(super::input_monitor::InputEventKind::ButtonPress)?;
            enigo
                .button(button, Direction::Press)
                .map_err(os_error)?;
            let steps = (duration_ms / 16).clamp(2, 300);
            let delay =
                Duration::from_millis((*duration_ms as u64 / steps as u64).max(1));
            for step in 1..=steps {
                if let Err(error) = lease.check() {
                    let _ = enigo.button(button, Direction::Release);
                    return Err(error);
                }
                let x = start.x + (end.x - start.x) * step as f64 / steps as f64;
                let y = start.y + (end.y - start.y) * step as f64 / steps as f64;
                input_monitor.expect(super::input_monitor::InputEventKind::Motion)?;
                enigo
                    .move_mouse(x.round() as i32, y.round() as i32, Coordinate::Abs)
                    .map_err(os_error)?;
                std::thread::sleep(delay);
            }
            input_monitor.expect(super::input_monitor::InputEventKind::ButtonRelease)?;
            enigo
                .button(button, Direction::Release)
                .map_err(os_error)?;
        }
        ComputerAction::Keypress { key, modifiers } => {
            for modifier in modifiers {
                input_monitor.expect(super::input_monitor::InputEventKind::KeyPress)?;
                enigo
                    .key(enigo_modifier(*modifier), Direction::Press)
                    .map_err(os_error)?;
            }
            lease.check()?;
            input_monitor.expect(super::input_monitor::InputEventKind::KeyPress)?;
            input_monitor.expect(super::input_monitor::InputEventKind::KeyRelease)?;
            enigo
                .key(enigo_key(*key), Direction::Click)
                .map_err(os_error)?;
            for modifier in modifiers.iter().rev() {
                input_monitor.expect(super::input_monitor::InputEventKind::KeyRelease)?;
                let _ = enigo.key(enigo_modifier(*modifier), Direction::Release);
            }
        }
        _ => return Err(ComputerUseError::HumanHandoffRequired(
            "this platform action needs accessibility semantics that were not present in the observation"
                .to_string(),
        )),
    }
    input_monitor.settle_expected()?;
    lease.check()
}

fn input_settings() -> Settings {
    Settings {
        windows_dw_extra_info: Some(super::input_monitor::EVENT_TAG),
        event_source_user_data: Some(super::input_monitor::EVENT_TAG as i64),
        open_prompt_to_get_permissions: false,
        ..Settings::default()
    }
}

fn enigo_modifier(modifier: Modifier) -> EnigoKey {
    match modifier {
        Modifier::Command => EnigoKey::Meta,
        Modifier::Control => EnigoKey::Control,
        Modifier::Option => EnigoKey::Alt,
        Modifier::Shift => EnigoKey::Shift,
    }
}

fn enigo_key(key: Key) -> EnigoKey {
    match key {
        Key::Return => EnigoKey::Return,
        Key::Escape => EnigoKey::Escape,
        Key::Tab => EnigoKey::Tab,
        Key::Space => EnigoKey::Space,
        Key::Backspace => EnigoKey::Backspace,
        Key::Delete => EnigoKey::Delete,
        Key::ArrowUp => EnigoKey::UpArrow,
        Key::ArrowDown => EnigoKey::DownArrow,
        Key::ArrowLeft => EnigoKey::LeftArrow,
        Key::ArrowRight => EnigoKey::RightArrow,
        Key::Home => EnigoKey::Home,
        Key::End => EnigoKey::End,
        Key::PageUp => EnigoKey::PageUp,
        Key::PageDown => EnigoKey::PageDown,
        Key::Character(value) => EnigoKey::Unicode(value),
    }
}

#[cfg(target_os = "windows")]
fn focus_window(window: &WindowInfo) -> Result<(), ComputerUseError> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_RESTORE};
    let handle = HWND(window.target.window_id as isize as *mut _);
    unsafe {
        let _ = ShowWindow(handle, SW_RESTORE);
        SetForegroundWindow(handle)
            .ok()
            .map_err(|error| ComputerUseError::Os(error.to_string()))
    }
}

#[cfg(target_os = "linux")]
fn focus_window(window: &WindowInfo) -> Result<(), ComputerUseError> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        ClientMessageData, ClientMessageEvent, ConnectionExt as _, EventMask,
    };
    let (connection, screen) = x11rb::connect(None).map_err(os_error)?;
    let atom = connection
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .map_err(os_error)?
        .reply()
        .map_err(os_error)?
        .atom;
    let event = ClientMessageEvent::new(
        32,
        window.target.window_id,
        atom,
        ClientMessageData::from([1, x11rb::CURRENT_TIME, 0, 0, 0]),
    );
    let root = connection.setup().roots[screen].root;
    connection
        .send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )
        .map_err(os_error)?
        .check()
        .map_err(os_error)?;
    connection.flush().map_err(os_error)
}

fn validate_geometry(
    observation: &LatestObservation,
    action: &ComputerAction,
) -> Result<(), ComputerUseError> {
    let points = match action {
        ComputerAction::Click {
            point: Some(point), ..
        } => vec![point],
        ComputerAction::Drag { start, end, .. } => {
            start.point.iter().chain(end.point.iter()).collect()
        }
        _ => Vec::new(),
    };
    for point in points {
        let bounds = crate::Rect {
            x: 0.0,
            y: 0.0,
            width: observation.screenshot_width as f64,
            height: observation.screenshot_height as f64,
        };
        if !bounds.contains(*point) {
            return Err(ComputerUseError::PointOutOfBounds {
                x: point.x,
                y: point.y,
            });
        }
    }
    Ok(())
}

fn reject_semantic_only_action(action: &ComputerAction) -> Result<(), ComputerUseError> {
    match action {
        ComputerAction::Click {
            element_id: None,
            point: Some(_),
            ..
        }
        | ComputerAction::Drag {
            start: crate::ActionLocation {
                element_id: None,
                point: Some(_),
            },
            end: crate::ActionLocation {
                element_id: None,
                point: Some(_),
            },
            duration_ms: 50..=2_000,
            ..
        }
        | ComputerAction::Keypress { .. } => Ok(()),
        _ => Err(ComputerUseError::HumanHandoffRequired(
            "the current Windows/Linux observation exposes pixels but no trusted accessibility element for this action"
                .to_string(),
        )),
    }
}

fn screenshot_point(
    observation: &LatestObservation,
    point: Point,
) -> Result<Point, ComputerUseError> {
    let frame = observation.window.frame;
    let global = Point {
        x: frame.x + point.x / observation.screenshot_width as f64 * frame.width,
        y: frame.y + point.y / observation.screenshot_height as f64 * frame.height,
    };
    frame
        .contains(global)
        .then_some(global)
        .ok_or(ComputerUseError::ObservationStale)
}

fn authorize_commit(record: &PreparedRecord) -> Result<(), ComputerUseError> {
    match record.public.assessment.disposition {
        ActionDisposition::Deny => Err(ComputerUseError::ActionDenied(
            record.public.assessment.reason.clone(),
        )),
        ActionDisposition::MandatoryHandoff => Err(ComputerUseError::HumanHandoffRequired(
            record.public.assessment.reason.clone(),
        )),
        ActionDisposition::Allow if record.request.dry_run => Ok(()),
        _ if record.authorization == Some(ActionAuthorization::Once) => Ok(()),
        _ => Err(ComputerUseError::ApprovalRequired),
    }
}

fn redacted_preview(window: &WindowInfo, action: &ComputerAction) -> RedactedActionPreview {
    RedactedActionPreview {
        summary: match action {
            ComputerAction::Click { .. } => "Click in the observed window",
            ComputerAction::Drag { .. } => "Drag within the observed window",
            ComputerAction::Keypress { .. } => "Send a bounded keypress",
            _ => "Perform a bounded action",
        }
        .to_string(),
        app_name: window.app_name.clone(),
        bundle_id: window.target.bundle_id.clone(),
        pid: window.target.pid,
        window_id: window.target.window_id,
        element_id: None,
        payload_summary: Some("no sensitive payload".to_string()),
    }
}

fn receipt(record: &PreparedRecord, outcome: ReceiptOutcome) -> ActionReceipt {
    ActionReceipt {
        receipt_id: uuid::Uuid::new_v4().to_string(),
        prepared_action_id: record.public.id.clone(),
        application_identity_key: record.public.application.identity_key.clone(),
        bundle_id: record.public.window.bundle_id.clone(),
        pid: record.public.window.pid,
        window_id: record.public.window.window_id,
        action_kind: record.public.kind,
        disposition: record.public.assessment.disposition,
        outcome,
        payload_summary: "no sensitive payload".to_string(),
        completed_at_ms: now_ms(),
        persisted: false,
    }
}

fn frame_changed(before: crate::Rect, after: crate::Rect) -> bool {
    (before.x - after.x).abs() > 2.0
        || (before.y - after.y).abs() > 2.0
        || (before.width - after.width).abs() > 2.0
        || (before.height - after.height).abs() > 2.0
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn os_error(error: impl std::fmt::Display) -> ComputerUseError {
    ComputerUseError::Os(error.to_string())
}
