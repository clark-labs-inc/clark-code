use serde::{Deserialize, Serialize};

use crate::{ActionIntent, ActionRisk, MouseButton, Point, WindowTarget};

/// Code-signing identity resolved from the running target process. Bundle IDs
/// are presentation and lookup metadata; durable approval is keyed by
/// `identity_key`, which covers the bundle ID and designated requirement.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApplicationIdentity {
    pub bundle_id: String,
    pub team_identifier: Option<String>,
    pub designated_requirement: String,
    pub identity_key: String,
    pub durable_approval_eligible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionDisposition {
    Deny,
    MandatoryHandoff,
    ActionTimeConfirmation,
    PreapprovalEligible,
    Allow,
}

impl std::fmt::Display for ActionDisposition {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Deny => "deny",
            Self::MandatoryHandoff => "mandatory_handoff",
            Self::ActionTimeConfirmation => "action_time_confirmation",
            Self::PreapprovalEligible => "preapproval_eligible",
            Self::Allow => "allow",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Click,
    TypeText,
    Keypress,
    Scroll,
    Drag,
    SecondaryAction,
    SelectText,
    SetValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionLocation {
    pub element_id: Option<String>,
    /// Screenshot-local pixels.
    pub point: Option<Point>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComputerAction {
    Click {
        element_id: Option<String>,
        point: Option<Point>,
        button: MouseButton,
    },
    TypeText {
        element_id: String,
        text: String,
        replace: bool,
    },
    Keypress {
        key: crate::Key,
        modifiers: Vec<crate::Modifier>,
    },
    Scroll {
        element_id: Option<String>,
        delta_x: i32,
        delta_y: i32,
    },
    Drag {
        start: ActionLocation,
        end: ActionLocation,
        button: MouseButton,
        duration_ms: u32,
    },
    SecondaryAction {
        element_id: String,
        action: String,
    },
    SelectText {
        element_id: String,
        start: u32,
        end: u32,
    },
    SetValue {
        element_id: String,
        value: f64,
    },
}

impl ComputerAction {
    pub fn kind(&self) -> ActionKind {
        match self {
            Self::Click { .. } => ActionKind::Click,
            Self::TypeText { .. } => ActionKind::TypeText,
            Self::Keypress { .. } => ActionKind::Keypress,
            Self::Scroll { .. } => ActionKind::Scroll,
            Self::Drag { .. } => ActionKind::Drag,
            Self::SecondaryAction { .. } => ActionKind::SecondaryAction,
            Self::SelectText { .. } => ActionKind::SelectText,
            Self::SetValue { .. } => ActionKind::SetValue,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrepareActionRequest {
    /// Advisory model classification. The trusted backend computes the
    /// authoritative assessment and disposition from the observation.
    pub intent: ActionIntent,
    pub window: WindowTarget,
    pub observation_id: String,
    pub action: ComputerAction,
    pub dry_run: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedActionAssessment {
    pub risk: ActionRisk,
    pub disposition: ActionDisposition,
    pub reason_code: String,
    pub reason: String,
    pub model_underclassified: bool,
}

/// Presentation-safe action description. It deliberately excludes window
/// titles, element values, entered text, and model-provided rationale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedActionPreview {
    pub summary: String,
    pub app_name: String,
    pub bundle_id: String,
    pub pid: i32,
    pub window_id: u32,
    pub element_id: Option<String>,
    pub payload_summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedAction {
    pub id: String,
    pub window: WindowTarget,
    pub application: ApplicationIdentity,
    pub kind: ActionKind,
    pub assessment: TrustedActionAssessment,
    pub preview: RedactedActionPreview,
    pub approval_revision: u64,
    pub expires_at_ms: u64,
    pub dry_run: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionAuthorization {
    Once,
    Durable,
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptOutcome {
    Succeeded,
    DryRun,
    Cancelled,
    UserTakeover,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub receipt_id: String,
    pub prepared_action_id: String,
    pub application_identity_key: String,
    pub bundle_id: String,
    pub pid: i32,
    pub window_id: u32,
    pub action_kind: ActionKind,
    pub disposition: ActionDisposition,
    pub outcome: ReceiptOutcome,
    pub payload_summary: String,
    pub completed_at_ms: u64,
    pub persisted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelAck {
    pub lease_id: Option<String>,
    pub quiesced: bool,
    pub helper_terminated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppApproval {
    pub identity_key: String,
    pub bundle_id: String,
    pub app_name: String,
    pub team_identifier: Option<String>,
    pub granted_at_ms: u64,
    pub last_used_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalSnapshot {
    pub revision: u64,
    pub approvals: Vec<AppApproval>,
}
