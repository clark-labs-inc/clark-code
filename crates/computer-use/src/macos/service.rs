use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{
    ensure_bundle_allowed, ensure_window_allowed, ActionAuthorization, ApprovalStore, ClickRequest,
    ComputerBackend, ComputerUseError, KeyPressRequest, Observation, PermissionRequest,
    PermissionStatus, TypeTextRequest, WindowFilter, WindowInfo, WindowTarget,
};

use super::{accessibility, capture, input, permissions, windows};

mod prepared;

const OBSERVATION_TTL: Duration = Duration::from_secs(30);
const INPUT_WINDOW: Duration = Duration::from_secs(1);
const MAX_INPUTS_PER_WINDOW: usize = 8;
const MAX_TRACKED_OBSERVATIONS: usize = 32;

#[derive(Clone)]
pub(super) struct LatestElement {
    pub info: crate::ElementInfo,
    pub global_bounds: crate::Rect,
}

#[derive(Clone)]
pub(super) struct LatestObservation {
    pub id: String,
    pub observed_at: Instant,
    pub window: WindowInfo,
    pub screenshot_width: u32,
    pub screenshot_height: u32,
    pub elements: HashMap<String, LatestElement>,
    pub element_list: Vec<crate::ElementInfo>,
}

pub(super) struct PreparedRecord {
    pub public: crate::PreparedAction,
    pub request: crate::PrepareActionRequest,
    pub observation: LatestObservation,
    pub authorization: Option<ActionAuthorization>,
}

#[derive(Default)]
pub(super) struct BackendState {
    pub latest: HashMap<WindowTarget, LatestObservation>,
    /// Read-only baseline for the next Accessibility diff. Unlike `latest`,
    /// this is not an action capability and therefore survives one-use
    /// observation consumption.
    pub last_observed: HashMap<WindowTarget, LatestObservation>,
    pub prepared: HashMap<String, PreparedRecord>,
    pub input_times: HashMap<WindowTarget, VecDeque<Instant>>,
    pub requested_screen_recording: bool,
}

/// Native APIs compiled only into the separately signed helper target.
#[derive(Clone)]
pub(super) struct MacServiceBackend {
    pub(super) state: Arc<Mutex<BackendState>>,
    pub(super) approvals: ApprovalStore,
    pub(super) leases: crate::lease::InputLeaseCoordinator,
    pub(super) input_monitor: input::PhysicalInputMonitor,
}

impl MacServiceBackend {
    pub fn new(approvals: ApprovalStore) -> Self {
        let leases = crate::lease::InputLeaseCoordinator::default();
        Self {
            state: Arc::new(Mutex::new(BackendState::default())),
            approvals,
            input_monitor: input::PhysicalInputMonitor::new(leases.clone()),
            leases,
        }
    }

    pub(super) fn validate_target(
        &self,
        target: &WindowTarget,
    ) -> Result<WindowInfo, ComputerUseError> {
        ensure_bundle_allowed(&target.bundle_id)?;
        let window = windows::resolve_window(target)?;
        ensure_window_allowed(&window)?;
        Ok(window)
    }

    pub(super) fn ensure_window_unchanged(
        &self,
        observed: &WindowInfo,
    ) -> Result<(), ComputerUseError> {
        let current = self.validate_target(&observed.target)?;
        if frame_changed(observed.frame, current.frame) {
            return Err(ComputerUseError::ObservationStale);
        }
        Ok(())
    }
}

impl ComputerBackend for MacServiceBackend {
    fn permissions(&self) -> Result<PermissionStatus, ComputerUseError> {
        let mut status = permissions::preflight();
        status.screen_recording_restart_required = self
            .state
            .lock()
            .map(|state| state.requested_screen_recording && !status.screen_recording)
            .unwrap_or(false);
        Ok(status)
    }

    fn request_permissions(
        &self,
        request: PermissionRequest,
    ) -> Result<PermissionStatus, ComputerUseError> {
        let before = permissions::preflight();
        if request.screen_recording && !before.screen_recording {
            if let Ok(mut state) = self.state.lock() {
                state.requested_screen_recording = true;
            }
        }
        permissions::request(request);
        self.permissions()
    }

    fn list_windows(&self, filter: WindowFilter) -> Result<Vec<WindowInfo>, ComputerUseError> {
        if let Some(bundle_id) = filter.bundle_id.as_deref() {
            ensure_bundle_allowed(bundle_id)?;
        }
        let mut listed = windows::list_windows(&filter)?;
        listed.retain(|window| ensure_window_allowed(window).is_ok());
        Ok(listed)
    }

    fn launch_application(&self, bundle_id: &str) -> Result<(), ComputerUseError> {
        ensure_bundle_allowed(bundle_id)?;
        windows::launch_application(bundle_id)
    }

    fn observe(&self, target: &WindowTarget) -> Result<Observation, ComputerUseError> {
        let status = self.permissions()?;
        if !status.screen_recording {
            return Err(ComputerUseError::PermissionMissing("Screen Recording"));
        }
        if !status.accessibility {
            return Err(ComputerUseError::PermissionMissing("Accessibility"));
        }
        let window = self.validate_target(target)?;
        let initial_screenshot = capture::capture_window(&window)?;
        let (mut walk, settlement) =
            prepared::settled_walk(&window, initial_screenshot.width, initial_screenshot.height)?;
        let screenshot = capture::capture_window(&window)?;
        if screenshot.width != initial_screenshot.width
            || screenshot.height != initial_screenshot.height
        {
            walk = accessibility::walk_window(&window, screenshot.width, screenshot.height)?;
        }
        let elements = walk
            .elements
            .iter()
            .map(|element| element.info.clone())
            .collect::<Vec<_>>();
        let observation_id = uuid::Uuid::new_v4().to_string();
        let latest = LatestObservation {
            id: observation_id.clone(),
            observed_at: Instant::now(),
            window: window.clone(),
            screenshot_width: screenshot.width,
            screenshot_height: screenshot.height,
            elements: walk
                .elements
                .into_iter()
                .map(|element| {
                    (
                        element.info.id.clone(),
                        LatestElement {
                            info: element.info,
                            global_bounds: element.global_bounds,
                        },
                    )
                })
                .collect(),
            element_list: elements.clone(),
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| ComputerUseError::Os("computer-use state lock poisoned".to_string()))?;
        state
            .latest
            .retain(|_, observation| observation.observed_at.elapsed() <= OBSERVATION_TTL);
        state
            .last_observed
            .retain(|_, observation| observation.observed_at.elapsed() <= OBSERVATION_TTL);
        if state.latest.len() >= MAX_TRACKED_OBSERVATIONS && !state.latest.contains_key(target) {
            let oldest = state
                .latest
                .iter()
                .max_by_key(|(_, observation)| observation.observed_at.elapsed())
                .map(|(target, _)| target.clone());
            if let Some(oldest) = oldest {
                state.latest.remove(&oldest);
            }
        }
        if state.last_observed.len() >= MAX_TRACKED_OBSERVATIONS
            && !state.last_observed.contains_key(target)
        {
            let oldest = state
                .last_observed
                .iter()
                .max_by_key(|(_, observation)| observation.observed_at.elapsed())
                .map(|(target, _)| target.clone());
            if let Some(oldest) = oldest {
                state.last_observed.remove(&oldest);
            }
        }
        state.latest.insert(target.clone(), latest.clone());
        let previous = state.last_observed.insert(target.clone(), latest);
        state
            .prepared
            .retain(|_, prepared| prepared.public.window != *target);
        drop(state);
        let accessibility_diff = previous.map(|previous| {
            crate::observation::diff_elements(previous.id, &previous.element_list, &elements)
        });
        Ok(Observation {
            window,
            observation_id,
            screenshot,
            elements,
            accessibility_truncated: walk.truncated,
            observed_at_ms: now_ms(),
            accessibility_diff,
            settlement,
        })
    }

    fn prepare_action(
        &self,
        request: crate::PrepareActionRequest,
    ) -> Result<crate::PreparedAction, ComputerUseError> {
        self.prepare_impl(request)
    }

    fn prepared_action(&self, id: &str) -> Result<crate::PreparedAction, ComputerUseError> {
        self.prepared_impl(id)
    }

    fn authorize_action(
        &self,
        id: &str,
        authorization: ActionAuthorization,
    ) -> Result<(), ComputerUseError> {
        self.authorize_impl(id, authorization)
    }

    fn commit_action(&self, id: &str) -> Result<crate::ActionReceipt, ComputerUseError> {
        self.commit_impl(id)
    }

    fn cancel_active(&self) -> Result<crate::CancelAck, ComputerUseError> {
        self.leases.cancel_active()
    }

    fn click(&self, request: ClickRequest) -> Result<(), ComputerUseError> {
        let declared = request.intent.risk;
        let prepared = self.prepare_impl(crate::PrepareActionRequest {
            intent: request.intent,
            window: request.window,
            observation_id: request.observation_id,
            action: crate::ComputerAction::Click {
                element_id: request.element_id,
                point: request.point,
                button: request.button,
            },
            dry_run: request.dry_run,
        })?;
        self.commit_legacy(prepared, declared)
    }

    fn type_text(&self, request: TypeTextRequest) -> Result<(), ComputerUseError> {
        let declared = request.intent.risk;
        let prepared = self.prepare_impl(crate::PrepareActionRequest {
            intent: request.intent,
            window: request.window,
            observation_id: request.observation_id,
            action: crate::ComputerAction::TypeText {
                element_id: request.element_id,
                text: request.text,
                replace: request.replace,
            },
            dry_run: request.dry_run,
        })?;
        self.commit_legacy(prepared, declared)
    }

    fn keypress(&self, request: KeyPressRequest) -> Result<(), ComputerUseError> {
        let declared = request.intent.risk;
        let prepared = self.prepare_impl(crate::PrepareActionRequest {
            intent: request.intent,
            window: request.window,
            observation_id: request.observation_id,
            action: crate::ComputerAction::Keypress {
                key: request.key,
                modifiers: request.modifiers,
            },
            dry_run: request.dry_run,
        })?;
        self.commit_legacy(prepared, declared)
    }
}

pub(super) fn frame_changed(before: crate::Rect, after: crate::Rect) -> bool {
    (before.x - after.x).abs() > 2.0
        || (before.y - after.y).abs() > 2.0
        || (before.width - after.width).abs() > 2.0
        || (before.height - after.height).abs() > 2.0
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
