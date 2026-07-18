use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::{AgentPath, AgentRole, AgentStatus, UsageCharge};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionId(String);

impl ExecutionId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.len() > 128 {
            return Err("execution ids must contain 1 to 128 characters".to_string());
        }
        if !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
        {
            return Err(
                "execution ids may contain letters, digits, dash, underscore, dot, and colon"
                    .to_string(),
            );
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ExecutionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AttemptId {
    pub execution: ExecutionId,
    pub path: AgentPath,
    pub sequence: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Queued,
    Running,
    AwaitingInput,
    Recovering,
    Verifying,
    Completed,
    Failed,
    Cancelled,
    Blocked,
}

impl ExecutionState {
    pub fn is_final(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Blocked
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Running,
    Completed,
    Failed,
    Cancelled,
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    TransientTransport,
    RateLimited,
    Provider,
    ContextOverflow,
    EmptyResponse,
    Tool,
    LocalState,
    Cancelled,
}

impl FailureClass {
    pub fn recoverable(self) -> bool {
        matches!(self, Self::TransientTransport | Self::RateLimited)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub max_attempts: u32,
    pub weighted_token_limit: Option<f64>,
    pub max_cost_usd: Option<f64>,
    pub non_cached_input_weight: f64,
    pub cached_input_weight: f64,
    pub output_weight: f64,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            weighted_token_limit: None,
            max_cost_usd: None,
            non_cached_input_weight: 1.0,
            cached_input_weight: 0.1,
            output_weight: 4.0,
        }
    }
}

impl ExecutionPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_attempts == 0 {
            return Err("execution max_attempts must be greater than zero".to_string());
        }
        for (name, value) in [
            ("non_cached_input_weight", self.non_cached_input_weight),
            ("cached_input_weight", self.cached_input_weight),
            ("output_weight", self.output_weight),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be finite and non-negative"));
            }
        }
        if self
            .weighted_token_limit
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err("weighted token limit must be finite and positive".to_string());
        }
        if self
            .max_cost_usd
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err("cost limit must be finite and non-negative".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub weighted_tokens: f64,
    pub cost_usd: f64,
}

impl ExecutionUsage {
    fn add(&mut self, usage: &UsageCharge, policy: &ExecutionPolicy) {
        let cached = usage.cached_input_tokens.min(usage.input_tokens);
        let non_cached = usage.input_tokens.saturating_sub(cached);
        self.input_tokens = self.input_tokens.saturating_add(usage.input_tokens);
        self.cached_input_tokens = self.cached_input_tokens.saturating_add(cached);
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        self.weighted_tokens += non_cached as f64 * policy.non_cached_input_weight
            + cached as f64 * policy.cached_input_weight
            + usage.output_tokens as f64 * policy.output_weight;
        self.cost_usd += usage.cost_usd.max(0.0);
    }

    pub fn exhausted(&self, policy: &ExecutionPolicy) -> bool {
        policy
            .weighted_token_limit
            .is_some_and(|limit| self.weighted_tokens >= limit)
            || policy
                .max_cost_usd
                .is_some_and(|limit| self.cost_usd >= limit)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEvidence {
    pub id: String,
    pub name: String,
    pub mutating: bool,
    pub status: ToolExecutionStatus,
    #[serde(default)]
    pub locations: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceReceipt {
    pub baseline_checkpoint: Option<String>,
    #[serde(default)]
    pub changed_paths: BTreeSet<String>,
    #[serde(default)]
    pub tools: BTreeMap<String, ToolEvidence>,
    #[serde(default)]
    pub verification_tools: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionAttempt {
    pub id: AttemptId,
    pub outcome: AttemptOutcome,
    pub usage: ExecutionUsage,
    pub failure_class: Option<FailureClass>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildExecution {
    pub path: AgentPath,
    pub role: AgentRole,
    pub status: AgentStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionSnapshot {
    pub id: ExecutionId,
    pub root: AgentPath,
    pub writer: AgentPath,
    pub state: ExecutionState,
    pub policy: ExecutionPolicy,
    pub active_attempt: Option<AttemptId>,
    pub attempts: Vec<ExecutionAttempt>,
    pub usage: ExecutionUsage,
    pub recoveries: u32,
    pub steering_messages: u32,
    pub active_tools: BTreeMap<String, ToolEvidence>,
    pub children: BTreeMap<AgentPath, ChildExecution>,
    pub evidence: EvidenceReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryDecision {
    pub allowed: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionEventKind {
    Created {
        policy: ExecutionPolicy,
    },
    AttemptStarted {
        attempt: AttemptId,
    },
    StateChanged {
        from: ExecutionState,
        to: ExecutionState,
        reason: Option<String>,
    },
    Checkpointed {
        id: String,
    },
    SteeringRecorded,
    ToolStarted {
        id: String,
        name: String,
        mutating: bool,
    },
    ToolFinished {
        id: String,
        status: ToolExecutionStatus,
        locations: BTreeSet<String>,
    },
    UsageRecorded {
        usage: UsageCharge,
    },
    RecoveryScheduled {
        failure_class: FailureClass,
        message: String,
    },
    ChildAttached {
        path: AgentPath,
        role: AgentRole,
    },
    ChildUpdated {
        path: AgentPath,
        status: AgentStatus,
    },
    EvidenceFinalized {
        receipt: EvidenceReceipt,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub sequence: u64,
    pub execution_id: ExecutionId,
    pub path: AgentPath,
    #[serde(rename = "active_attempt")]
    pub attempt: Option<AttemptId>,
    #[serde(flatten)]
    pub kind: ExecutionEventKind,
}

struct LedgerState {
    snapshot: ExecutionSnapshot,
    events: Vec<ExecutionEvent>,
}

#[derive(Clone)]
pub struct ExecutionLedger {
    state: Arc<Mutex<LedgerState>>,
}

impl ExecutionLedger {
    pub fn new_root(id: ExecutionId, policy: ExecutionPolicy) -> Result<Self, String> {
        policy.validate()?;
        let root = AgentPath::root();
        let snapshot = ExecutionSnapshot {
            id: id.clone(),
            root: root.clone(),
            writer: root.clone(),
            state: ExecutionState::Queued,
            policy: policy.clone(),
            active_attempt: None,
            attempts: Vec::new(),
            usage: ExecutionUsage::default(),
            recoveries: 0,
            steering_messages: 0,
            active_tools: BTreeMap::new(),
            children: BTreeMap::new(),
            evidence: EvidenceReceipt::default(),
        };
        let created = ExecutionEvent {
            sequence: 1,
            execution_id: id,
            path: root,
            attempt: None,
            kind: ExecutionEventKind::Created { policy },
        };
        Ok(Self {
            state: Arc::new(Mutex::new(LedgerState {
                snapshot,
                events: vec![created],
            })),
        })
    }

    pub fn snapshot(&self) -> ExecutionSnapshot {
        self.state
            .lock()
            .expect("execution ledger lock")
            .snapshot
            .clone()
    }

    pub fn events(&self) -> Vec<ExecutionEvent> {
        self.state
            .lock()
            .expect("execution ledger lock")
            .events
            .clone()
    }

    pub fn created_event(&self) -> ExecutionEvent {
        self.state
            .lock()
            .expect("execution ledger lock")
            .events
            .first()
            .expect("execution ledger always has a created event")
            .clone()
    }

    pub fn start_attempt(&self) -> Result<ExecutionEvent, String> {
        let snapshot = self.snapshot();
        let attempt = AttemptId {
            execution: snapshot.id,
            path: snapshot.root,
            sequence: snapshot.attempts.len() as u32 + 1,
        };
        self.record(ExecutionEventKind::AttemptStarted { attempt })
    }

    pub fn transition(
        &self,
        to: ExecutionState,
        reason: Option<String>,
    ) -> Result<ExecutionEvent, String> {
        let from = self.snapshot().state;
        self.record(ExecutionEventKind::StateChanged { from, to, reason })
    }

    pub fn checkpoint(&self, id: impl Into<String>) -> Result<ExecutionEvent, String> {
        self.record(ExecutionEventKind::Checkpointed { id: id.into() })
    }

    pub fn record_steering(&self) -> Result<ExecutionEvent, String> {
        self.record(ExecutionEventKind::SteeringRecorded)
    }

    pub fn tool_started(
        &self,
        id: impl Into<String>,
        name: impl Into<String>,
        mutating: bool,
    ) -> Result<ExecutionEvent, String> {
        self.record(ExecutionEventKind::ToolStarted {
            id: id.into(),
            name: name.into(),
            mutating,
        })
    }

    pub fn tool_finished(
        &self,
        id: impl Into<String>,
        status: ToolExecutionStatus,
        locations: BTreeSet<String>,
    ) -> Result<ExecutionEvent, String> {
        self.record(ExecutionEventKind::ToolFinished {
            id: id.into(),
            status,
            locations,
        })
    }

    pub fn record_usage(&self, usage: UsageCharge) -> Result<ExecutionEvent, String> {
        self.record(ExecutionEventKind::UsageRecorded { usage })
    }

    pub fn recovery_decision(&self, failure_class: FailureClass) -> RecoveryDecision {
        let snapshot = self.snapshot();
        let mut reasons = Vec::new();
        if !failure_class.recoverable() {
            reasons.push("failure class is not recoverable".to_string());
        }
        if snapshot.attempts.len() as u32 >= snapshot.policy.max_attempts {
            reasons.push("execution attempt limit reached".to_string());
        }
        if snapshot.state != ExecutionState::Running {
            reasons.push(format!(
                "execution is not at a running boundary: {:?}",
                snapshot.state
            ));
        }
        if !snapshot.active_tools.is_empty() {
            reasons.push("a tool has no terminal receipt".to_string());
        }
        if snapshot.usage.exhausted(&snapshot.policy) {
            reasons.push("execution budget is exhausted".to_string());
        }
        RecoveryDecision {
            allowed: reasons.is_empty(),
            reasons,
        }
    }

    pub fn schedule_recovery(
        &self,
        failure_class: FailureClass,
        message: impl Into<String>,
    ) -> Result<ExecutionEvent, String> {
        let decision = self.recovery_decision(failure_class);
        if !decision.allowed {
            return Err(decision.reasons.join("; "));
        }
        self.record(ExecutionEventKind::RecoveryScheduled {
            failure_class,
            message: message.into(),
        })
    }

    pub fn attach_child(&self, path: AgentPath, role: AgentRole) -> Result<ExecutionEvent, String> {
        self.record(ExecutionEventKind::ChildAttached { path, role })
    }

    pub fn update_child(
        &self,
        path: AgentPath,
        status: AgentStatus,
    ) -> Result<ExecutionEvent, String> {
        self.record(ExecutionEventKind::ChildUpdated { path, status })
    }

    pub fn finalize_evidence(&self, receipt: EvidenceReceipt) -> Result<ExecutionEvent, String> {
        self.record(ExecutionEventKind::EvidenceFinalized { receipt })
    }

    pub fn replay(events: &[ExecutionEvent]) -> Result<ExecutionSnapshot, String> {
        let Some(first) = events.first() else {
            return Err("execution replay requires at least one event".to_string());
        };
        let ExecutionEventKind::Created { policy } = &first.kind else {
            return Err("execution replay must begin with created".to_string());
        };
        if first.sequence != 1 || first.path != AgentPath::root() || first.attempt.is_some() {
            return Err("invalid execution created event".to_string());
        }
        policy.validate()?;
        let mut snapshot = ExecutionSnapshot {
            id: first.execution_id.clone(),
            root: AgentPath::root(),
            writer: AgentPath::root(),
            state: ExecutionState::Queued,
            policy: policy.clone(),
            active_attempt: None,
            attempts: Vec::new(),
            usage: ExecutionUsage::default(),
            recoveries: 0,
            steering_messages: 0,
            active_tools: BTreeMap::new(),
            children: BTreeMap::new(),
            evidence: EvidenceReceipt::default(),
        };
        for (index, event) in events.iter().enumerate().skip(1) {
            if event.sequence != index as u64 + 1 || event.execution_id != snapshot.id {
                return Err("execution event sequence or identity mismatch".to_string());
            }
            apply(&mut snapshot, event)?;
        }
        Ok(snapshot)
    }

    fn record(&self, kind: ExecutionEventKind) -> Result<ExecutionEvent, String> {
        let mut state = self.state.lock().expect("execution ledger lock");
        let event = ExecutionEvent {
            sequence: state.events.len() as u64 + 1,
            execution_id: state.snapshot.id.clone(),
            path: state.snapshot.root.clone(),
            attempt: state.snapshot.active_attempt.clone(),
            kind,
        };
        apply(&mut state.snapshot, &event)?;
        state.events.push(event.clone());
        Ok(event)
    }
}

mod reducer;
use reducer::apply;

#[cfg(test)]
mod tests;
