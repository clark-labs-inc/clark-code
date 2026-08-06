use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_segment(&value)?;
        Ok(Self(value))
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrchestrationId(pub String);

impl OrchestrationId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_segment(&value)?;
        Ok(Self(value))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentPath(String);

impl AgentPath {
    pub fn root() -> Self {
        Self("/root".to_string())
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value == "/root" {
            return Ok(Self::root());
        }
        let Some(tail) = value.strip_prefix("/root/") else {
            return Err("agent path must start with /root".to_string());
        };
        if tail.is_empty() {
            return Err("agent path must include a task name".to_string());
        }
        for segment in tail.split('/') {
            validate_segment(segment)?;
        }
        Ok(Self(value))
    }

    pub fn child(&self, task: &TaskId) -> Result<Self, String> {
        Self::parse(format!("{}/{}", self.0, task.0))
    }

    pub fn resolve(&self, reference: &str) -> Result<Self, String> {
        if reference.starts_with('/') {
            Self::parse(reference)
        } else {
            let task = TaskId::new(reference)?;
            self.child(&task)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn depth(&self) -> usize {
        self.0.split('/').filter(|part| !part.is_empty()).count() - 1
    }
}

impl fmt::Display for AgentPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn validate_segment(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 64 {
        return Err("task names must contain 1 to 64 characters".to_string());
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
    }) {
        return Err("task names may contain lowercase letters, digits, _ and -".to_string());
    }
    Ok(())
}

/// Delegated roles are intentionally read-only. There is no writer variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Explorer,
    Reviewer,
    Verifier,
    ExternalResearcher,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessKind {
    Local,
    Acp,
    ClarkCloud,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    PendingInit,
    Running,
    Interrupted,
    Completed,
    Errored,
    Shutdown,
}

impl AgentStatus {
    pub fn is_final(self) -> bool {
        matches!(self, Self::Completed | Self::Errored | Self::Shutdown)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Running,
    Reported,
    Rework,
    Accepted,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    QueueOnly,
    TriggerTurn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportDecision {
    Accept,
    Rework,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadOnlyTask {
    pub id: TaskId,
    pub role: AgentRole,
    pub objective: String,
    pub scopes: BTreeSet<String>,
    pub acceptance: Vec<String>,
    pub harness: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEvidence {
    pub command: String,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestEvidence {
    pub name: String,
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimEvidence {
    pub evidence_ref: String,
    pub claim: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredReport {
    pub task_id: TaskId,
    pub attempt: u32,
    pub status: ReportStatus,
    #[serde(default)]
    pub changed_paths: BTreeSet<String>,
    #[serde(default)]
    pub commands: Vec<CommandEvidence>,
    #[serde(default)]
    pub tests: Vec<TestEvidence>,
    #[serde(default)]
    pub claims: Vec<ClaimEvidence>,
    #[serde(default)]
    pub unresolved: Vec<String>,
    pub summary: String,
}

impl StructuredReport {
    pub fn validate_read_only(&self, task: &ReadOnlyTask, attempt: u32) -> Result<(), String> {
        if self.task_id != task.id {
            return Err("report task id does not match the delegated task".to_string());
        }
        if self.attempt != attempt {
            return Err("report attempt does not match the active attempt".to_string());
        }
        if self.status != ReportStatus::Reported {
            return Err("a completed attempt must report status=reported".to_string());
        }
        if !self.changed_paths.is_empty() {
            return Err("read-only reports must have an empty changed_paths set".to_string());
        }
        if self.summary.trim().is_empty() {
            return Err("report summary must not be empty".to_string());
        }
        if self.commands.is_empty()
            && self.tests.is_empty()
            && self.claims.is_empty()
            && self.unresolved.is_empty()
        {
            return Err(
                "report must include concrete evidence or an explicit unresolved item".to_string(),
            );
        }
        if self
            .claims
            .iter()
            .any(|claim| claim.claim.trim().is_empty() || claim.evidence_ref.trim().is_empty())
        {
            return Err("report claims require a claim and evidence reference".to_string());
        }
        if self
            .commands
            .iter()
            .any(|command| command.command.trim().is_empty())
            || self.tests.iter().any(|test| test.name.trim().is_empty())
            || self.unresolved.iter().any(|item| item.trim().is_empty())
        {
            return Err("report evidence entries must not be empty".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: String,
    pub path: AgentPath,
    pub task: ReadOnlyTask,
    pub status: AgentStatus,
    pub report_status: ReportStatus,
    pub attempt: u32,
    pub report: Option<StructuredReport>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub sender: AgentPath,
    pub target: AgentPath,
    pub body: String,
    pub mode: DeliveryMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_paths_are_hierarchical_and_strict() {
        let root = AgentPath::root();
        let child = root.child(&TaskId::new("api_scan").unwrap()).unwrap();
        assert_eq!(child.as_str(), "/root/api_scan");
        assert_eq!(child.depth(), 1);
        assert!(AgentPath::parse("root/nope").is_err());
        assert!(TaskId::new("Uppercase").is_err());
    }

    #[test]
    fn report_requires_evidence_or_an_explicit_unresolved_item() {
        let task = ReadOnlyTask {
            id: TaskId::new("reader").unwrap(),
            role: AgentRole::Explorer,
            objective: "inspect".to_string(),
            scopes: BTreeSet::from(["src".to_string()]),
            acceptance: vec!["cite evidence".to_string()],
            harness: "local".to_string(),
        };
        let report = StructuredReport {
            task_id: task.id.clone(),
            attempt: 1,
            status: ReportStatus::Reported,
            summary: "unsupported conclusion".to_string(),
            changed_paths: BTreeSet::new(),
            commands: vec![],
            tests: vec![],
            claims: vec![],
            unresolved: vec![],
        };
        assert!(report.validate_read_only(&task, 1).is_err());
    }
}
