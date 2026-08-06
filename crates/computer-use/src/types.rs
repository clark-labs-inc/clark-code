use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ComputerUseError {
    #[error("computer use is unsupported on {0}")]
    UnsupportedPlatform(String),
    #[error("macOS {0} permission is required")]
    PermissionMissing(&'static str),
    #[error("window not found: {0}")]
    WindowNotFound(String),
    #[error("the target window changed identity: {0}")]
    TargetChanged(String),
    #[error("computer use forbids target `{bundle_id}`: {reason}")]
    TargetForbidden { bundle_id: String, reason: String },
    #[error("observe the target window before acting")]
    ObservationRequired,
    #[error("the last observation is stale; observe the window again")]
    ObservationStale,
    #[error("element `{0}` is not in the latest observation")]
    ElementNotFound(String),
    #[error("element `{0}` is disabled")]
    ElementDisabled(String),
    #[error("element `{0}` is not actionable")]
    ElementNotActionable(String),
    #[error(
        "action risk was declared as `{declared}` but must be `{required}`: {reason}; observe again and retry with the required risk"
    )]
    RiskDeclarationMismatch {
        declared: ActionRisk,
        required: ActionRisk,
        reason: String,
    },
    #[error("invalid action intent: {0}")]
    InvalidActionIntent(String),
    #[error("point ({x:.1}, {y:.1}) is outside the latest screenshot")]
    PointOutOfBounds { x: f64, y: f64 },
    #[error("input rate limit reached for this window")]
    RateLimited,
    #[error("prepared action not found: {0}")]
    PreparedActionNotFound(String),
    #[error("the prepared action expired; observe and prepare the action again")]
    PreparedActionExpired,
    #[error("action denied by trusted computer-use policy: {0}")]
    ActionDenied(String),
    #[error("this action must be completed by the user: {0}")]
    HumanHandoffRequired(String),
    #[error("the prepared action requires current user authorization")]
    ApprovalRequired,
    #[error("computer input was cancelled")]
    InputCancelled,
    #[error("computer input stopped because physical user input took over")]
    UserTakeover,
    #[error("physical-input takeover monitoring is unavailable")]
    TakeoverMonitorUnavailable,
    #[error("invalid computer action: {0}")]
    InvalidAction(String),
    #[error("computer-use approval storage failed: {0}")]
    ApprovalStore(String),
    #[error("computer-use helper is unavailable: {0}")]
    HelperUnavailable(String),
    #[error("computer-use helper protocol failed closed: {0}")]
    HelperProtocol(String),
    #[error("computer-use helper rejected the request: {0}")]
    HelperRejected(String),
    #[error("{0}")]
    Os(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x
            && point.y >= self.y
            && point.x <= self.x + self.width
            && point.y <= self.y + self.height
    }

    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.width / 2.0,
            y: self.y + self.height / 2.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WindowTarget {
    pub pid: i32,
    pub window_id: u32,
    pub bundle_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub target: WindowTarget,
    pub app_name: String,
    pub title: String,
    pub frame: Rect,
    pub layer: i32,
    pub on_screen: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowFilter {
    pub bundle_id: Option<String>,
    pub title_contains: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub accessibility: bool,
    pub screen_recording: bool,
    pub screen_recording_restart_required: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub accessibility: bool,
    pub screen_recording: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    #[serde(with = "serde_bytes")]
    pub png: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElementInfo {
    pub id: String,
    pub role: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    /// Coordinates in screenshot pixels, not global display points.
    pub bounds: Rect,
    pub enabled: bool,
    pub focused: bool,
    pub actionable: bool,
    /// Accessibility action names advertised by the element. These are part
    /// of the safety assessment, not permission to invoke arbitrary actions.
    pub actions: Vec<String>,
    /// True for secure/protected text controls whose content is credential
    /// material even when the visible label is generic.
    pub sensitive_text: bool,
    /// Whether AXValue can be changed directly. Text fields still use the
    /// dedicated type-text contract; direct value setting is numeric-only.
    #[serde(default)]
    pub value_settable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_constraints: Option<crate::ValueConstraints>,
}

impl ElementInfo {
    pub fn label(&self) -> String {
        self.name
            .as_deref()
            .or(self.description.as_deref())
            .or(self.value.as_deref())
            .unwrap_or(&self.role)
            .to_string()
    }

    pub fn semantic_label(&self) -> Option<String> {
        self.name
            .as_deref()
            .or(self.description.as_deref())
            .or(self.value.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub window: WindowInfo,
    /// Unpredictable capability that binds exactly one subsequent action to
    /// this observation. A newer observation or a successful action revokes it.
    pub observation_id: String,
    pub screenshot: Screenshot,
    pub elements: Vec<ElementInfo>,
    pub accessibility_truncated: bool,
    pub observed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_diff: Option<crate::AccessibilityDiff>,
    #[serde(default)]
    pub settlement: crate::ObservationSettlement,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    Command,
    Control,
    Option,
    Shift,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Return,
    Escape,
    Tab,
    Space,
    Backspace,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Character(char),
}

/// The consequential effect the caller believes an action can have. Every
/// action must commit to one category before its payload is executed. The
/// backend independently infers a minimum category from the observation and
/// rejects under-classification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRisk {
    #[default]
    Routine,
    Destructive,
    Financial,
    ExternalCommunication,
    Credential,
    SecuritySensitive,
    Ambiguous,
}

impl std::fmt::Display for ActionRisk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Routine => "routine",
            Self::Destructive => "destructive",
            Self::Financial => "financial",
            Self::ExternalCommunication => "external_communication",
            Self::Credential => "credential",
            Self::SecuritySensitive => "security_sensitive",
            Self::Ambiguous => "ambiguous",
        };
        formatter.write_str(value)
    }
}

impl std::str::FromStr for ActionRisk {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "routine" => Ok(Self::Routine),
            "destructive" => Ok(Self::Destructive),
            "financial" => Ok(Self::Financial),
            "external_communication" => Ok(Self::ExternalCommunication),
            "credential" => Ok(Self::Credential),
            "security_sensitive" => Ok(Self::SecuritySensitive),
            "ambiguous" => Ok(Self::Ambiguous),
            _ => Err(format!("unknown action risk `{value}`")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionIntent {
    pub risk: ActionRisk,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub risk: ActionRisk,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClickRequest {
    pub intent: ActionIntent,
    pub window: WindowTarget,
    pub observation_id: String,
    pub element_id: Option<String>,
    /// Screenshot-local pixels. Requires a latest observation.
    pub point: Option<Point>,
    pub button: MouseButton,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TypeTextRequest {
    pub intent: ActionIntent,
    pub window: WindowTarget,
    pub observation_id: String,
    pub element_id: String,
    pub text: String,
    pub replace: bool,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyPressRequest {
    pub intent: ActionIntent,
    pub window: WindowTarget,
    pub observation_id: String,
    pub key: Key,
    pub modifiers: Vec<Modifier>,
    pub dry_run: bool,
}

/// Synchronous on purpose: native APIs are blocking and provider tools place
/// calls on Tokio's blocking pool. Keeping this boundary runtime-neutral makes
/// the simulator usable in unit and integration tests without an executor.
pub trait ComputerBackend: Send + Sync {
    fn permissions(&self) -> Result<PermissionStatus, ComputerUseError>;
    fn request_permissions(
        &self,
        request: PermissionRequest,
    ) -> Result<PermissionStatus, ComputerUseError>;
    fn list_windows(&self, filter: WindowFilter) -> Result<Vec<WindowInfo>, ComputerUseError>;
    fn launch_application(&self, bundle_id: &str) -> Result<(), ComputerUseError>;
    fn observe(&self, window: &WindowTarget) -> Result<Observation, ComputerUseError>;
    fn prepare_action(
        &self,
        request: crate::PrepareActionRequest,
    ) -> Result<crate::PreparedAction, ComputerUseError> {
        let _ = request;
        Err(ComputerUseError::UnsupportedPlatform(
            "prepared actions are not implemented by this backend".to_string(),
        ))
    }
    fn prepared_action(&self, id: &str) -> Result<crate::PreparedAction, ComputerUseError> {
        Err(ComputerUseError::PreparedActionNotFound(id.to_string()))
    }
    fn authorize_action(
        &self,
        id: &str,
        authorization: crate::ActionAuthorization,
    ) -> Result<(), ComputerUseError> {
        let _ = authorization;
        Err(ComputerUseError::PreparedActionNotFound(id.to_string()))
    }
    fn commit_action(&self, id: &str) -> Result<crate::ActionReceipt, ComputerUseError> {
        Err(ComputerUseError::PreparedActionNotFound(id.to_string()))
    }
    fn cancel_active(&self) -> Result<crate::CancelAck, ComputerUseError> {
        Ok(crate::CancelAck {
            lease_id: None,
            quiesced: true,
            helper_terminated: false,
        })
    }
    #[doc(hidden)]
    fn click(&self, request: ClickRequest) -> Result<(), ComputerUseError>;
    #[doc(hidden)]
    fn type_text(&self, request: TypeTextRequest) -> Result<(), ComputerUseError>;
    #[doc(hidden)]
    fn keypress(&self, request: KeyPressRequest) -> Result<(), ComputerUseError>;
}
