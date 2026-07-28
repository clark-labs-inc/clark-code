use std::fmt;

use scout_adapter_protocol::{
    AdapterId, AdapterQuery, AuthContextHandle, AuthContextId, CoverageBinding, CursorHandle,
    TargetId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manifest::{QuotaKey, RouteKind};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SchedulerTaskId(String);

impl SchedulerTaskId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn from_spec(spec: &TaskSpec) -> Result<Self, String> {
        let digest = canonical_sha256(&(
            "scout-scheduler-task-v1",
            &spec.enterprise_id,
            &spec.charter_id,
            spec.discovery_epoch,
            &spec.target_id,
            &spec.adapter_id,
            &spec.auth_context_id,
            &spec.auth_context_handle,
            &spec.coverage,
            &spec.query,
            spec.page_ordinal,
            &spec.cursor_handle,
            &spec.origin,
            spec.priority,
        ))?;
        Ok(Self(format!("scheduler-task:{digest}")))
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_prefixed_digest("scheduler task id", &self.0, "scheduler-task:")
    }
}

impl fmt::Display for SchedulerTaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskOrigin {
    Root,
    Continuation {
        parent_task_id: SchedulerTaskId,
    },
    Expansion {
        parent_task_id: SchedulerTaskId,
        rule_id: String,
        source_evidence_sha256: String,
    },
}

impl TaskOrigin {
    pub(crate) fn parent(&self) -> Option<&SchedulerTaskId> {
        match self {
            Self::Root => None,
            Self::Continuation { parent_task_id } | Self::Expansion { parent_task_id, .. } => {
                Some(parent_task_id)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    pub task_id: SchedulerTaskId,
    pub enterprise_id: String,
    pub charter_id: String,
    pub discovery_epoch: u64,
    pub target_id: TargetId,
    pub adapter_id: AdapterId,
    pub auth_context_id: AuthContextId,
    pub auth_context_handle: AuthContextHandle,
    pub coverage: CoverageBinding,
    pub query: AdapterQuery,
    pub page_ordinal: u32,
    pub cursor_handle: Option<CursorHandle>,
    pub origin: TaskOrigin,
    pub priority: u16,
}

impl TaskSpec {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        enterprise_id: impl Into<String>,
        charter_id: impl Into<String>,
        discovery_epoch: u64,
        target_id: TargetId,
        adapter_id: AdapterId,
        auth_context_id: AuthContextId,
        auth_context_handle: AuthContextHandle,
        coverage: CoverageBinding,
        query: AdapterQuery,
        page_ordinal: u32,
        cursor_handle: Option<CursorHandle>,
        origin: TaskOrigin,
        priority: u16,
    ) -> Result<Self, String> {
        let mut spec = Self {
            task_id: SchedulerTaskId(String::new()),
            enterprise_id: enterprise_id.into(),
            charter_id: charter_id.into(),
            discovery_epoch,
            target_id,
            adapter_id,
            auth_context_id,
            auth_context_handle,
            coverage,
            query,
            page_ordinal,
            cursor_handle,
            origin,
            priority,
        };
        spec.task_id = SchedulerTaskId::from_spec(&spec)?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn route_kind(&self) -> RouteKind {
        RouteKind {
            adapter_id: self.adapter_id.clone(),
            operation: self.query.operation.clone(),
            provider_resource_type: self.query.provider_resource_type.clone(),
        }
    }

    pub fn quota_key(&self) -> QuotaKey {
        QuotaKey::new(
            &self.target_id,
            &self.adapter_id,
            &self.query.authority_scope,
        )
        .expect("validated task bindings derive a valid quota key")
    }

    pub fn validate(&self) -> Result<(), String> {
        self.task_id.validate()?;
        validate_text("scheduler enterprise id", &self.enterprise_id, 256)?;
        validate_text("scheduler charter id", &self.charter_id, 256)?;
        self.target_id
            .validate()
            .map_err(|error| error.to_string())?;
        self.adapter_id
            .validate()
            .map_err(|error| error.to_string())?;
        self.auth_context_id
            .validate()
            .map_err(|error| error.to_string())?;
        self.auth_context_handle
            .validate()
            .map_err(|error| error.to_string())?;
        self.coverage
            .validate()
            .map_err(|error| error.to_string())?;
        self.query.validate().map_err(|error| error.to_string())?;
        if self.coverage.enterprise_id != self.enterprise_id
            || self.coverage.charter_id != self.charter_id
            || self.coverage.discovery_epoch != self.discovery_epoch
            || self.coverage.adapter_id != self.adapter_id
            || self.coverage.auth_context_id != self.auth_context_id
            || self.coverage.tenant != self.query.authority_scope
        {
            return Err("scheduler task bindings disagree".into());
        }
        match &self.origin {
            TaskOrigin::Root => {
                if self.page_ordinal != 0 || self.cursor_handle.is_some() {
                    return Err(
                        "root scheduler tasks must start at page zero without a cursor".into(),
                    );
                }
            }
            TaskOrigin::Continuation { parent_task_id } => {
                parent_task_id.validate()?;
                if self.page_ordinal == 0 || self.cursor_handle.is_none() {
                    return Err(
                        "continuation scheduler tasks require a positive page and cursor".into(),
                    );
                }
            }
            TaskOrigin::Expansion {
                parent_task_id,
                rule_id,
                source_evidence_sha256,
            } => {
                parent_task_id.validate()?;
                validate_identifier("scheduler expansion rule", rule_id, 128)?;
                validate_digest("scheduler expansion evidence", source_evidence_sha256)?;
                if self.page_ordinal != 0 || self.cursor_handle.is_some() {
                    return Err(
                        "expanded scheduler routes must start at page zero without a cursor".into(),
                    );
                }
            }
        }
        if self.task_id != SchedulerTaskId::from_spec(self)? {
            return Err("scheduler task id does not match its immutable specification".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryClass {
    RateLimited,
    TransientTransport,
    ServiceUnavailable,
    AuthenticationExpired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalDisposition {
    Succeeded,
    Empty,
    Denied,
    Unsupported,
    Unsafe,
    Unreachable,
    Stale,
    RetryExhausted,
}

impl TerminalDisposition {
    pub fn is_complete(self) -> bool {
        matches!(self, Self::Succeeded | Self::Empty)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompletionDisposition {
    Success {
        final_page: bool,
    },
    Empty,
    Gap {
        terminal: TerminalDisposition,
    },
    Retry {
        class: RetryClass,
        retry_after_ms: Option<u64>,
        error_sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageCompletion {
    pub task_id: SchedulerTaskId,
    pub machine_id: String,
    pub fence: u64,
    pub completed_at_ms: u64,
    pub disposition: CompletionDisposition,
    pub receipt_id: Option<String>,
    pub evidence_sha256: Option<String>,
    pub continuation: Option<TaskSpec>,
    #[serde(default)]
    pub expansions: Vec<TaskSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseClaim {
    pub task: TaskSpec,
    pub machine_id: String,
    pub fence: u64,
    pub attempt: u16,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskStatus {
    Pending {
        not_before_ms: u64,
    },
    Leased {
        machine_id: String,
        fence: u64,
        expires_at_ms: u64,
    },
    RetryWait {
        not_before_ms: u64,
        class: RetryClass,
        error_sha256: String,
    },
    Terminal {
        disposition: TerminalDisposition,
        receipt_id: Option<String>,
        evidence_sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TaskRecord {
    pub spec: TaskSpec,
    pub status: TaskStatus,
    pub attempts: u16,
    pub fence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerReceipt {
    pub manifest_id: String,
    pub generation: u64,
    pub tasks: usize,
    pub pending: usize,
    pub leased: usize,
    pub retry_wait: usize,
    pub terminal: usize,
    pub complete_terminal: usize,
    pub gap_terminal: usize,
    pub sealed: bool,
    pub complete: bool,
    pub state_sha256: String,
}

pub(crate) fn canonical_sha256<T: Serialize>(value: &T) -> Result<String, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("scheduler canonical encoding failed: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(crate) fn validate_text(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(format!("{label} must contain 1 to {max} safe characters"));
    }
    Ok(())
}

pub(crate) fn validate_identifier(label: &str, value: &str, max: usize) -> Result<(), String> {
    validate_text(label, value, max)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!("{label} contains unsupported characters"));
    }
    Ok(())
}

pub(crate) fn validate_digest(label: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} must be a lowercase SHA-256 digest"));
    }
    Ok(())
}

pub(crate) fn validate_prefixed_digest(
    label: &str,
    value: &str,
    prefix: &str,
) -> Result<(), String> {
    let digest = value
        .strip_prefix(prefix)
        .ok_or_else(|| format!("{label} must start with {prefix}"))?;
    validate_digest(label, digest)
}
