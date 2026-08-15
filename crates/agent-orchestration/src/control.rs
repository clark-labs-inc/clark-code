use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::budget::{BudgetSnapshot, SharedBudget};
use crate::contract::{
    AgentPath, AgentRecord, AgentStatus, DeliveryMode, Message, ReadOnlyTask, ReportDecision,
    ReportStatus, StructuredReport,
};

struct State {
    agents: BTreeMap<AgentPath, AgentRecord>,
    reserved_paths: BTreeSet<AgentPath>,
    mailboxes: BTreeMap<AgentPath, Vec<Message>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlSnapshot {
    pub max_agents: usize,
    pub max_depth: usize,
    pub agents: BTreeMap<AgentPath, AgentRecord>,
    pub budget: BudgetSnapshot,
}

/// Root-tree control plane shared by every delegated read-only agent.
#[derive(Clone)]
pub struct ControlPlane {
    max_agents: usize,
    max_depth: usize,
    budget: SharedBudget,
    state: Arc<Mutex<State>>,
}

impl ControlPlane {
    pub fn new(max_agents: usize, max_depth: usize, budget: SharedBudget) -> Result<Self, String> {
        if max_agents == 0 {
            return Err("max_agents must be greater than zero".to_string());
        }
        if max_depth == 0 {
            return Err("max_depth must be greater than zero".to_string());
        }
        Ok(Self {
            max_agents,
            max_depth,
            budget,
            state: Arc::new(Mutex::new(State {
                agents: BTreeMap::new(),
                reserved_paths: BTreeSet::new(),
                mailboxes: BTreeMap::new(),
            })),
        })
    }

    /// Atomically reserve both an agent slot and its canonical path.
    pub fn reserve_spawn(
        &self,
        parent: &AgentPath,
        task: ReadOnlyTask,
    ) -> Result<SpawnReservation, String> {
        let path = parent.child(&task.id)?;
        if path.depth() > self.max_depth {
            return Err(format!(
                "agent depth {} exceeds configured maximum {}",
                path.depth(),
                self.max_depth
            ));
        }
        let mut state = self.state.lock().expect("control lock");
        if state.agents.len() + state.reserved_paths.len() >= self.max_agents {
            return Err("agent thread limit reached".to_string());
        }
        if state.agents.contains_key(&path) || !state.reserved_paths.insert(path.clone()) {
            return Err(format!("agent path already exists: {path}"));
        }
        drop(state);
        Ok(SpawnReservation {
            control: self.clone(),
            path,
            task: Some(task),
            active: true,
        })
    }

    fn commit_spawn(
        &self,
        path: AgentPath,
        task: ReadOnlyTask,
        agent_id: String,
    ) -> Result<AgentRecord, String> {
        let mut state = self.state.lock().expect("control lock");
        if !state.reserved_paths.remove(&path) {
            return Err(format!("agent path was not reserved: {path}"));
        }
        let record = AgentRecord {
            id: agent_id,
            path: path.clone(),
            task,
            status: AgentStatus::PendingInit,
            report_status: ReportStatus::Running,
            attempt: 1,
            report: None,
            error: None,
        };
        state.agents.insert(path, record.clone());
        Ok(record)
    }

    fn rollback_spawn(&self, path: &AgentPath) {
        self.state
            .lock()
            .expect("control lock")
            .reserved_paths
            .remove(path);
    }

    pub fn set_status(&self, path: &AgentPath, status: AgentStatus) -> Result<(), String> {
        let mut state = self.state.lock().expect("control lock");
        let record = state
            .agents
            .get_mut(path)
            .ok_or_else(|| format!("unknown agent: {path}"))?;
        record.status = status;
        Ok(())
    }

    pub fn fail(&self, path: &AgentPath, error: impl Into<String>) -> Result<(), String> {
        let mut state = self.state.lock().expect("control lock");
        let record = state
            .agents
            .get_mut(path)
            .ok_or_else(|| format!("unknown agent: {path}"))?;
        record.status = AgentStatus::Errored;
        record.report_status = ReportStatus::Failed;
        record.error = Some(error.into());
        Ok(())
    }

    pub fn report(&self, path: &AgentPath, report: StructuredReport) -> Result<bool, String> {
        let mut state = self.state.lock().expect("control lock");
        let record = state
            .agents
            .get_mut(path)
            .ok_or_else(|| format!("unknown agent: {path}"))?;
        report.validate_read_only(&record.task, record.attempt)?;
        if let Some(existing) = &record.report {
            return if existing == &report {
                Ok(false)
            } else {
                Err("the active attempt already submitted a different report".to_string())
            };
        }
        record.status = AgentStatus::Completed;
        record.report_status = ReportStatus::Reported;
        record.report = Some(report);
        Ok(true)
    }

    pub fn decide(&self, path: &AgentPath, decision: ReportDecision) -> Result<u32, String> {
        let mut state = self.state.lock().expect("control lock");
        let record = state
            .agents
            .get_mut(path)
            .ok_or_else(|| format!("unknown agent: {path}"))?;
        if record.report_status != ReportStatus::Reported || record.report.is_none() {
            return Err("only a reported result can be accepted or reworked".to_string());
        }
        match decision {
            ReportDecision::Accept => {
                record.report_status = ReportStatus::Accepted;
                Ok(record.attempt)
            }
            ReportDecision::Rework => {
                record.attempt += 1;
                record.status = AgentStatus::Interrupted;
                record.report_status = ReportStatus::Rework;
                record.report = None;
                record.error = None;
                Ok(record.attempt)
            }
        }
    }

    pub fn send_message(&self, message: Message) -> Result<(), String> {
        if message.body.trim().is_empty() {
            return Err("message must not be empty".to_string());
        }
        let mut state = self.state.lock().expect("control lock");
        if !state.agents.contains_key(&message.target) {
            return Err(format!("unknown target agent: {}", message.target));
        }
        if message.mode == DeliveryMode::TriggerTurn {
            let target = state
                .agents
                .get(&message.target)
                .expect("target checked above");
            if target.report_status != ReportStatus::Rework {
                return Err(
                    "triggering a new turn requires an explicit rework decision".to_string()
                );
            }
        }
        state
            .mailboxes
            .entry(message.target.clone())
            .or_default()
            .push(message);
        Ok(())
    }

    pub fn drain_mailbox(&self, target: &AgentPath) -> Vec<Message> {
        self.state
            .lock()
            .expect("control lock")
            .mailboxes
            .remove(target)
            .unwrap_or_default()
    }

    pub fn agent(&self, path: &AgentPath) -> Option<AgentRecord> {
        self.state
            .lock()
            .expect("control lock")
            .agents
            .get(path)
            .cloned()
    }

    pub fn snapshot(&self) -> ControlSnapshot {
        ControlSnapshot {
            max_agents: self.max_agents,
            max_depth: self.max_depth,
            agents: self.state.lock().expect("control lock").agents.clone(),
            budget: self.budget.snapshot(),
        }
    }

    pub fn budget(&self) -> &SharedBudget {
        &self.budget
    }
}

/// A reservation rolls back path and capacity when spawning fails or is cancelled.
pub struct SpawnReservation {
    control: ControlPlane,
    path: AgentPath,
    task: Option<ReadOnlyTask>,
    active: bool,
}

impl SpawnReservation {
    pub fn path(&self) -> &AgentPath {
        &self.path
    }

    pub fn commit(mut self, agent_id: impl Into<String>) -> Result<AgentRecord, String> {
        let task = self.task.take().expect("reservation task");
        let result = self
            .control
            .commit_spawn(self.path.clone(), task, agent_id.into());
        if result.is_ok() {
            self.active = false;
        }
        result
    }
}

impl Drop for SpawnReservation {
    fn drop(&mut self) {
        if self.active {
            self.control.rollback_spawn(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::budget::BudgetConfig;
    use crate::contract::{AgentRole, TaskId};

    use super::*;

    fn task(name: &str) -> ReadOnlyTask {
        ReadOnlyTask {
            id: TaskId::new(name).unwrap(),
            role: AgentRole::Explorer,
            objective: "inspect".to_string(),
            scopes: BTreeSet::from(["src".to_string()]),
            acceptance: vec!["cite paths".to_string()],
            harness: "local".to_string(),
        }
    }

    fn control(max_agents: usize) -> ControlPlane {
        ControlPlane::new(
            max_agents,
            1,
            SharedBudget::new(BudgetConfig::default()).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn failed_spawn_rolls_back_capacity_and_path() {
        let control = control(1);
        let reservation = control
            .reserve_spawn(&AgentPath::root(), task("reader"))
            .unwrap();
        assert!(control
            .reserve_spawn(&AgentPath::root(), task("other"))
            .is_err());
        drop(reservation);
        assert!(control
            .reserve_spawn(&AgentPath::root(), task("reader"))
            .is_ok());
    }

    #[test]
    fn report_must_precede_accept_and_rework_remains_available() {
        let control = control(1);
        let path = AgentPath::parse("/root/reader").unwrap();
        control
            .reserve_spawn(&AgentPath::root(), task("reader"))
            .unwrap()
            .commit("agent-1")
            .unwrap();
        assert!(control.decide(&path, ReportDecision::Accept).is_err());
        control
            .report(
                &path,
                StructuredReport {
                    task_id: TaskId::new("reader").unwrap(),
                    attempt: 1,
                    status: ReportStatus::Reported,
                    summary: "found evidence".to_string(),
                    changed_paths: BTreeSet::new(),
                    commands: vec![],
                    tests: vec![],
                    claims: vec![crate::contract::ClaimEvidence {
                        claim: "found evidence".to_string(),
                        evidence_ref: "src/lib.rs:1".to_string(),
                    }],
                    unresolved: vec![],
                },
            )
            .unwrap();
        assert_eq!(control.decide(&path, ReportDecision::Rework).unwrap(), 2);
    }
}
