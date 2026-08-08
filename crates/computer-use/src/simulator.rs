use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    encode_rgba_png, ensure_bundle_allowed, ClickRequest, ComputerBackend, ComputerUseError,
    ElementInfo, KeyPressRequest, Observation, PermissionRequest, PermissionStatus, Rect,
    Screenshot, TypeTextRequest, WindowFilter, WindowInfo, WindowTarget,
};

mod prepared;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 420;

#[derive(Debug)]
pub(super) struct SimulatorState {
    launched: bool,
    revision: u64,
    input: String,
    status: String,
    observation_sequence: u64,
    latest_observation_id: Option<String>,
    last_observation_id: Option<String>,
    last_elements: Vec<ElementInfo>,
    slider_value: f64,
    prepared_sequence: u64,
    receipt_sequence: u64,
    approval_revision: u64,
    approved_identities: HashSet<String>,
    prepared: HashMap<String, SimPreparedRecord>,
}

#[derive(Clone, Debug)]
pub(super) struct SimPreparedRecord {
    pub public: crate::PreparedAction,
    pub request: crate::PrepareActionRequest,
    pub authorization: Option<crate::ActionAuthorization>,
}

impl Default for SimulatorState {
    fn default() -> Self {
        Self {
            launched: true,
            revision: 0,
            input: String::new(),
            status: "Ready".to_string(),
            observation_sequence: 0,
            latest_observation_id: None,
            last_observation_id: None,
            last_elements: Vec::new(),
            slider_value: 50.0,
            prepared_sequence: 0,
            receipt_sequence: 0,
            approval_revision: 0,
            approved_identities: HashSet::new(),
            prepared: HashMap::new(),
        }
    }
}

/// Deterministic in-memory backend used to prove the model/tool loop without
/// TCC access or real input events.
#[derive(Default)]
pub struct SimulatedComputerBackend {
    pub(super) state: Mutex<SimulatorState>,
    pub(super) leases: crate::lease::InputLeaseCoordinator,
}

impl SimulatedComputerBackend {
    pub const BUNDLE_ID: &'static str = "com.agent-desktop.computer-use-simulator";

    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> (String, String) {
        let state = self.state.lock().expect("simulator lock");
        (state.input.clone(), state.status.clone())
    }

    pub(super) fn window() -> WindowInfo {
        WindowInfo {
            target: WindowTarget {
                pid: 42_424,
                window_id: 7,
                bundle_id: Self::BUNDLE_ID.to_string(),
            },
            app_name: "Agent Computer Use Simulator".to_string(),
            title: "Computer Use Test Surface".to_string(),
            frame: Rect {
                x: 100.0,
                y: 100.0,
                width: WIDTH as f64,
                height: HEIGHT as f64,
            },
            layer: 0,
            on_screen: true,
        }
    }

    pub(super) fn validate_window(window: &WindowTarget) -> Result<(), ComputerUseError> {
        if window == &Self::window().target {
            Ok(())
        } else {
            Err(ComputerUseError::TargetChanged(format!(
                "expected simulator window, got {}:{} ({})",
                window.pid, window.window_id, window.bundle_id
            )))
        }
    }

    pub(super) fn elements(state: &SimulatorState) -> Vec<ElementInfo> {
        vec![
            ElementInfo {
                id: "ax-0".to_string(),
                role: "AXWindow".to_string(),
                name: Some("Computer Use Test Surface".to_string()),
                value: None,
                description: None,
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: WIDTH as f64,
                    height: HEIGHT as f64,
                },
                enabled: true,
                focused: true,
                actionable: false,
                actions: Vec::new(),
                sensitive_text: false,
                value_settable: false,
                value_constraints: None,
            },
            ElementInfo {
                id: "ax-1".to_string(),
                role: "AXTextField".to_string(),
                name: Some("Text input".to_string()),
                value: Some(state.input.clone()),
                description: Some("Text input".to_string()),
                bounds: Rect {
                    x: 48.0,
                    y: 90.0,
                    width: 360.0,
                    height: 44.0,
                },
                enabled: true,
                focused: true,
                actionable: true,
                actions: vec!["AXSetValue".to_string()],
                sensitive_text: false,
                value_settable: true,
                value_constraints: None,
            },
            ElementInfo {
                id: "ax-2".to_string(),
                role: "AXButton".to_string(),
                name: Some("Open example".to_string()),
                value: None,
                description: None,
                bounds: Rect {
                    x: 440.0,
                    y: 90.0,
                    width: 150.0,
                    height: 44.0,
                },
                enabled: true,
                focused: false,
                actionable: true,
                actions: vec!["AXPress".to_string()],
                sensitive_text: false,
                value_settable: false,
                value_constraints: None,
            },
            ElementInfo {
                id: "ax-3".to_string(),
                role: "AXStaticText".to_string(),
                name: Some("Status".to_string()),
                value: Some(state.status.clone()),
                description: None,
                bounds: Rect {
                    x: 48.0,
                    y: 180.0,
                    width: 542.0,
                    height: 40.0,
                },
                enabled: true,
                focused: false,
                actionable: false,
                actions: Vec::new(),
                sensitive_text: false,
                value_settable: false,
                value_constraints: None,
            },
            ElementInfo {
                id: "ax-4".to_string(),
                role: "AXButton".to_string(),
                name: Some("Delete record".to_string()),
                value: None,
                description: None,
                bounds: Rect {
                    x: 440.0,
                    y: 300.0,
                    width: 150.0,
                    height: 44.0,
                },
                enabled: true,
                focused: false,
                actionable: true,
                actions: vec!["AXPress".to_string()],
                sensitive_text: false,
                value_settable: false,
                value_constraints: None,
            },
            ElementInfo {
                id: "ax-5".to_string(),
                role: "AXSlider".to_string(),
                name: Some("Example level".to_string()),
                value: Some(state.slider_value.to_string()),
                description: Some("Example level".to_string()),
                bounds: Rect {
                    x: 48.0,
                    y: 260.0,
                    width: 300.0,
                    height: 28.0,
                },
                enabled: true,
                focused: false,
                actionable: true,
                actions: vec!["AXIncrement".to_string(), "AXDecrement".to_string()],
                sensitive_text: false,
                value_settable: true,
                value_constraints: Some(crate::ValueConstraints {
                    minimum: 0.0,
                    maximum: 100.0,
                    step: Some(1.0),
                }),
            },
        ]
    }

    fn screenshot(state: &SimulatorState) -> Result<Screenshot, ComputerUseError> {
        let mut rgba = vec![0_u8; WIDTH as usize * HEIGHT as usize * 4];
        fill(&mut rgba, [23, 20, 33, 255]);
        draw_rect(
            &mut rgba,
            Rect {
                x: 24.0,
                y: 24.0,
                width: 592.0,
                height: 372.0,
            },
            [42, 36, 58, 255],
        );
        for (rect, color) in [
            (
                Rect {
                    x: 48.0,
                    y: 90.0,
                    width: 360.0,
                    height: 44.0,
                },
                [247, 244, 252, 255],
            ),
            (
                Rect {
                    x: 440.0,
                    y: 90.0,
                    width: 150.0,
                    height: 44.0,
                },
                [121, 92, 255, 255],
            ),
            (
                Rect {
                    x: 48.0,
                    y: 180.0,
                    width: 542.0,
                    height: 40.0,
                },
                if state.status == "Ready" {
                    [74, 67, 91, 255]
                } else {
                    [28, 128, 92, 255]
                },
            ),
            (
                Rect {
                    x: 440.0,
                    y: 300.0,
                    width: 150.0,
                    height: 44.0,
                },
                [183, 56, 80, 255],
            ),
        ] {
            draw_rect(&mut rgba, rect, color);
        }
        Ok(Screenshot {
            width: WIDTH,
            height: HEIGHT,
            png: encode_rgba_png(WIDTH, HEIGHT, &rgba)?,
        })
    }

    pub(super) fn require_latest(
        state: &SimulatorState,
        observation_id: &str,
    ) -> Result<(), ComputerUseError> {
        match state.latest_observation_id.as_deref() {
            Some(current) if current == observation_id => Ok(()),
            Some(_) => Err(ComputerUseError::ObservationStale),
            None => Err(ComputerUseError::ObservationRequired),
        }
    }

    pub(super) fn invalidate(state: &mut SimulatorState) {
        state.revision += 1;
        state.latest_observation_id = None;
    }
}

impl ComputerBackend for SimulatedComputerBackend {
    fn permissions(&self) -> Result<PermissionStatus, ComputerUseError> {
        Ok(PermissionStatus {
            accessibility: true,
            screen_recording: true,
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
        let state = self.state.lock().expect("simulator lock");
        if !state.launched {
            return Ok(Vec::new());
        }
        let window = Self::window();
        let bundle_matches = filter
            .bundle_id
            .as_deref()
            .is_none_or(|bundle| bundle == window.target.bundle_id);
        let title_matches = filter.title_contains.as_deref().is_none_or(|title| {
            window
                .title
                .to_ascii_lowercase()
                .contains(&title.to_ascii_lowercase())
        });
        Ok(if bundle_matches && title_matches {
            vec![window]
        } else {
            Vec::new()
        })
    }

    fn launch_application(&self, bundle_id: &str) -> Result<(), ComputerUseError> {
        ensure_bundle_allowed(bundle_id)?;
        if bundle_id != Self::BUNDLE_ID {
            return Err(ComputerUseError::WindowNotFound(format!(
                "simulator only knows {}",
                Self::BUNDLE_ID
            )));
        }
        self.state.lock().expect("simulator lock").launched = true;
        Ok(())
    }

    fn observe(&self, window: &WindowTarget) -> Result<Observation, ComputerUseError> {
        ensure_bundle_allowed(&window.bundle_id)?;
        Self::validate_window(window)?;
        let mut state = self.state.lock().expect("simulator lock");
        if !state.launched {
            return Err(ComputerUseError::WindowNotFound(
                "simulator is not launched".to_string(),
            ));
        }
        let screenshot = Self::screenshot(&state)?;
        let elements = Self::elements(&state);
        let observation_id = format!("sim-observation-{}", state.observation_sequence);
        state.observation_sequence += 1;
        state.latest_observation_id = Some(observation_id.clone());
        let accessibility_diff = state.last_observation_id.as_ref().map(|previous_id| {
            crate::observation::diff_elements(previous_id.clone(), &state.last_elements, &elements)
        });
        state.last_observation_id = Some(observation_id.clone());
        state.last_elements = elements.clone();
        Ok(Observation {
            window: Self::window(),
            observation_id,
            screenshot,
            elements,
            accessibility_truncated: false,
            observed_at_ms: now_ms(),
            accessibility_diff,
            settlement: crate::ObservationSettlement {
                stable: true,
                elapsed_ms: 0,
                samples: 1,
            },
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
        authorization: crate::ActionAuthorization,
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
        if prepared.assessment.model_underclassified {
            self.authorize_impl(&prepared.id, crate::ActionAuthorization::Denied)?;
            return Err(ComputerUseError::RiskDeclarationMismatch {
                declared,
                required: prepared.assessment.risk,
                reason: prepared.assessment.reason,
            });
        }
        self.authorize_impl(&prepared.id, crate::ActionAuthorization::Once)?;
        self.commit_impl(&prepared.id).map(|_| ())
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
        if prepared.assessment.model_underclassified {
            self.authorize_impl(&prepared.id, crate::ActionAuthorization::Denied)?;
            return Err(ComputerUseError::RiskDeclarationMismatch {
                declared,
                required: prepared.assessment.risk,
                reason: prepared.assessment.reason,
            });
        }
        self.authorize_impl(&prepared.id, crate::ActionAuthorization::Once)?;
        self.commit_impl(&prepared.id).map(|_| ())
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
        if prepared.assessment.model_underclassified {
            self.authorize_impl(&prepared.id, crate::ActionAuthorization::Denied)?;
            return Err(ComputerUseError::RiskDeclarationMismatch {
                declared,
                required: prepared.assessment.risk,
                reason: prepared.assessment.reason,
            });
        }
        self.authorize_impl(&prepared.id, crate::ActionAuthorization::Once)?;
        self.commit_impl(&prepared.id).map(|_| ())
    }
}

fn fill(rgba: &mut [u8], color: [u8; 4]) {
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&color);
    }
}

fn draw_rect(rgba: &mut [u8], rect: Rect, color: [u8; 4]) {
    let min_x = rect.x.max(0.0) as u32;
    let min_y = rect.y.max(0.0) as u32;
    let max_x = (rect.x + rect.width).min(WIDTH as f64) as u32;
    let max_y = (rect.y + rect.height).min(HEIGHT as f64) as u32;
    for y in min_y..max_y {
        for x in min_x..max_x {
            let offset = ((y * WIDTH + x) * 4) as usize;
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests;
