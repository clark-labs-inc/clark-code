#[cfg(test)]
use std::collections::VecDeque;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{DeliveryMode, PermissionCeiling, StructuredHandoff, TaskStatus};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlMessage {
    pub sender: String,
    pub target: String,
    pub body: String,
    pub mode: DeliveryMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedControlState {
    pub max_active: usize,
    pub active_agents: BTreeMap<String, String>,
    pub task_statuses: BTreeMap<String, TaskStatus>,
    pub reported_attempts: BTreeMap<String, BTreeSet<String>>,
    pub accepted_attempts: BTreeMap<String, String>,
    pub writer_lease: Option<String>,
    pub token_budget: u64,
    pub tokens_used: u64,
    pub cancelled: bool,
    pub mailboxes: BTreeMap<String, Vec<ControlMessage>>,
}

struct ControlState {
    persisted: PersistedControlState,
    reservations: HashMap<String, String>,
    handoffs: BTreeMap<(String, String), StructuredHandoff>,
    wake_count: BTreeMap<String, u32>,
}

#[derive(Clone)]
pub struct ControlPlane {
    inner: Arc<Mutex<ControlState>>,
}

impl ControlPlane {
    pub fn new(max_active: usize, token_budget: u64) -> Self {
        Self::restore(PersistedControlState {
            max_active,
            active_agents: BTreeMap::new(),
            task_statuses: BTreeMap::new(),
            reported_attempts: BTreeMap::new(),
            accepted_attempts: BTreeMap::new(),
            writer_lease: None,
            token_budget,
            tokens_used: 0,
            cancelled: false,
            mailboxes: BTreeMap::new(),
        })
    }

    pub fn restore(persisted: PersistedControlState) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ControlState {
                persisted,
                reservations: HashMap::new(),
                handoffs: BTreeMap::new(),
                wake_count: BTreeMap::new(),
            })),
        }
    }

    pub fn snapshot(&self) -> PersistedControlState {
        self.inner.lock().expect("control lock").persisted.clone()
    }

    pub fn reserve_spawn(&self, agent_path: impl Into<String>) -> Result<SpawnReservation, String> {
        let agent_path = agent_path.into();
        let reservation_id = Uuid::new_v4().to_string();
        let mut state = self.inner.lock().expect("control lock");
        if state.persisted.cancelled {
            return Err("orchestration is cancelled".into());
        }
        if state.persisted.active_agents.len() + state.reservations.len()
            >= state.persisted.max_active
        {
            return Err("active-agent limit reached".into());
        }
        if state
            .persisted
            .active_agents
            .values()
            .any(|path| path == &agent_path)
            || state.reservations.values().any(|path| path == &agent_path)
        {
            return Err(format!("agent path already exists: {agent_path}"));
        }
        state
            .reservations
            .insert(reservation_id.clone(), agent_path);
        drop(state);
        Ok(SpawnReservation {
            control: self.clone(),
            reservation_id,
            committed: false,
        })
    }

    fn commit_spawn(&self, reservation_id: &str, attempt_id: String) -> Result<(), String> {
        let mut state = self.inner.lock().expect("control lock");
        let path = state
            .reservations
            .remove(reservation_id)
            .ok_or("unknown spawn reservation")?;
        state.persisted.active_agents.insert(attempt_id, path);
        Ok(())
    }

    fn rollback_spawn(&self, reservation_id: &str) {
        self.inner
            .lock()
            .expect("control lock")
            .reservations
            .remove(reservation_id);
    }

    pub fn release_agent(&self, attempt_id: &str) {
        self.inner
            .lock()
            .expect("control lock")
            .persisted
            .active_agents
            .remove(attempt_id);
    }

    pub fn acquire_writer(&self, task_id: &str) -> Result<WriterLease, String> {
        let mut state = self.inner.lock().expect("control lock");
        if let Some(owner) = &state.persisted.writer_lease {
            return Err(format!("writer lease held by {owner}"));
        }
        state.persisted.writer_lease = Some(task_id.to_string());
        drop(state);
        Ok(WriterLease {
            control: self.clone(),
            task_id: task_id.to_string(),
            active: true,
        })
    }

    fn release_writer(&self, task_id: &str) {
        let mut state = self.inner.lock().expect("control lock");
        if state.persisted.writer_lease.as_deref() == Some(task_id) {
            state.persisted.writer_lease = None;
        }
    }

    pub fn check_permission(
        &self,
        inherited: PermissionCeiling,
        requested: PermissionCeiling,
    ) -> Result<(), String> {
        inherited
            .permits(requested)
            .then_some(())
            .ok_or_else(|| format!("permission widening: {inherited:?} -> {requested:?}"))
    }

    pub fn record_usage(&self, tokens: u64) -> Result<(), String> {
        let mut state = self.inner.lock().expect("control lock");
        let next = state.persisted.tokens_used.saturating_add(tokens);
        if next > state.persisted.token_budget {
            return Err("tree token budget exhausted".into());
        }
        state.persisted.tokens_used = next;
        Ok(())
    }

    pub fn set_task_status(&self, task_id: &str, status: TaskStatus) {
        self.inner
            .lock()
            .expect("control lock")
            .persisted
            .task_statuses
            .insert(task_id.to_string(), status);
    }

    pub fn report_result(&self, handoff: StructuredHandoff) -> Result<bool, String> {
        let mut state = self.inner.lock().expect("control lock");
        if let Some(accepted_attempt) = state.persisted.accepted_attempts.get(&handoff.task_id) {
            return if accepted_attempt == &handoff.attempt_id {
                Ok(false)
            } else {
                Err(format!(
                    "task {} already accepted from attempt {}",
                    handoff.task_id, accepted_attempt
                ))
            };
        }
        let attempts = state
            .persisted
            .reported_attempts
            .entry(handoff.task_id.clone())
            .or_default();
        if !attempts.insert(handoff.attempt_id.clone()) {
            return Ok(false);
        }
        state
            .persisted
            .task_statuses
            .insert(handoff.task_id.clone(), TaskStatus::Reported);
        state.handoffs.insert(
            (handoff.task_id.clone(), handoff.attempt_id.clone()),
            handoff,
        );
        Ok(true)
    }

    pub fn accept_result(&self, task_id: &str, attempt_id: &str) -> Result<bool, String> {
        let mut state = self.inner.lock().expect("control lock");
        if let Some(accepted_attempt) = state.persisted.accepted_attempts.get(task_id) {
            return if accepted_attempt == attempt_id {
                Ok(false)
            } else {
                Err(format!(
                    "task {task_id} already accepted from attempt {accepted_attempt}"
                ))
            };
        }
        let reported = state
            .persisted
            .reported_attempts
            .get(task_id)
            .is_some_and(|attempts| attempts.contains(attempt_id));
        if !reported {
            return Err(format!(
                "cannot accept unreported attempt {attempt_id} for task {task_id}"
            ));
        }
        state
            .persisted
            .accepted_attempts
            .insert(task_id.to_string(), attempt_id.to_string());
        state
            .persisted
            .task_statuses
            .insert(task_id.to_string(), TaskStatus::Accepted);
        Ok(true)
    }

    pub fn send_message(&self, message: ControlMessage) {
        let mut state = self.inner.lock().expect("control lock");
        state
            .persisted
            .mailboxes
            .entry(message.target.clone())
            .or_default()
            .push(message.clone());
        if message.mode == DeliveryMode::TriggerTurn {
            *state.wake_count.entry(message.target).or_default() += 1;
        }
    }

    pub fn drain_mailbox(&self, target: &str) -> Vec<ControlMessage> {
        self.inner
            .lock()
            .expect("control lock")
            .persisted
            .mailboxes
            .remove(target)
            .unwrap_or_default()
    }

    pub fn wake_count(&self, target: &str) -> u32 {
        self.inner
            .lock()
            .expect("control lock")
            .wake_count
            .get(target)
            .copied()
            .unwrap_or(0)
    }

    pub fn cancel(&self) {
        self.inner.lock().expect("control lock").persisted.cancelled = true;
    }
}

pub struct SpawnReservation {
    control: ControlPlane,
    reservation_id: String,
    committed: bool,
}

impl SpawnReservation {
    pub fn commit(mut self, attempt_id: impl Into<String>) -> Result<(), String> {
        self.control
            .commit_spawn(&self.reservation_id, attempt_id.into())?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for SpawnReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.control.rollback_spawn(&self.reservation_id);
        }
    }
}

pub struct WriterLease {
    control: ControlPlane,
    task_id: String,
    active: bool,
}

impl Drop for WriterLease {
    fn drop(&mut self) {
        if self.active {
            self.control.release_writer(&self.task_id);
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct RetryQueue {
    attempts: BTreeMap<String, u32>,
    queue: VecDeque<String>,
}

#[cfg(test)]
impl RetryQueue {
    pub fn enqueue(&mut self, task_id: impl Into<String>) {
        self.queue.push_back(task_id.into());
    }

    pub fn next(&mut self, max_attempts: u32) -> Option<(String, u32)> {
        while let Some(task_id) = self.queue.pop_front() {
            let attempt = self.attempts.entry(task_id.clone()).or_default();
            if *attempt >= max_attempts {
                continue;
            }
            *attempt += 1;
            return Some((task_id, *attempt));
        }
        None
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetryDisposition {
    RetrySameScope,
    RetryWithNarrowerPermission,
    Terminal,
}

#[cfg(test)]
fn classify_failure(message: &str) -> RetryDisposition {
    let message = message.to_ascii_lowercase();
    if message.contains("permission widening") {
        RetryDisposition::RetryWithNarrowerPermission
    } else if ["crash", "timeout", "rate limit", "transport"]
        .iter()
        .any(|needle| message.contains(needle))
    {
        RetryDisposition::RetrySameScope
    } else {
        RetryDisposition::Terminal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClaimEvidence, CommandEvidence, TestEvidence};
    use std::collections::BTreeSet;

    fn handoff(task: &str, attempt: &str) -> StructuredHandoff {
        StructuredHandoff {
            task_id: task.into(),
            attempt_id: attempt.into(),
            reported_status: TaskStatus::Reported,
            summary: "done".into(),
            changed_paths: BTreeSet::new(),
            baseline_checkpoint: None,
            result_checkpoint: None,
            commands: Vec::<CommandEvidence>::new(),
            tests: Vec::<TestEvidence>::new(),
            claims: Vec::<ClaimEvidence>::new(),
            unresolved: vec![],
            artifact_refs: vec![],
        }
    }

    #[test]
    fn failed_spawn_rolls_back_capacity_and_path() {
        let control = ControlPlane::new(1, 100);
        let reservation = control.reserve_spawn("/root/research").unwrap();
        assert!(control.reserve_spawn("/root/other").is_err());
        drop(reservation);
        control
            .reserve_spawn("/root/research")
            .unwrap()
            .commit("attempt-2")
            .unwrap();
    }

    #[test]
    fn writer_lease_is_exclusive_and_drop_releases_it() {
        let control = ControlPlane::new(2, 100);
        let lease = control.acquire_writer("writer-a").unwrap();
        assert!(control.acquire_writer("writer-b").is_err());
        drop(lease);
        assert!(control.acquire_writer("writer-b").is_ok());
    }

    #[test]
    fn permission_ceiling_can_only_narrow() {
        let control = ControlPlane::new(1, 100);
        assert!(control
            .check_permission(
                PermissionCeiling::WorkspaceWrite,
                PermissionCeiling::ReadOnly
            )
            .is_ok());
        assert!(control
            .check_permission(PermissionCeiling::ReadOnly, PermissionCeiling::Full)
            .is_err());
    }

    #[test]
    fn result_reporting_is_idempotent_but_rejects_late_attempts() {
        let control = ControlPlane::new(1, 100);
        assert_eq!(control.report_result(handoff("t", "a")).unwrap(), true);
        assert_eq!(control.report_result(handoff("t", "a")).unwrap(), false);
        assert_eq!(control.report_result(handoff("t", "b")).unwrap(), true);
        assert_eq!(control.accept_result("t", "b").unwrap(), true);
        assert_eq!(control.accept_result("t", "b").unwrap(), false);
        assert!(control.report_result(handoff("t", "c")).is_err());
        assert!(control.accept_result("t", "a").is_err());
    }

    #[test]
    fn unreported_attempt_cannot_be_accepted() {
        let control = ControlPlane::new(1, 100);
        assert!(control.accept_result("t", "b").is_err());
    }

    #[test]
    fn queue_only_does_not_wake_but_followup_does() {
        let control = ControlPlane::new(1, 100);
        control.send_message(ControlMessage {
            sender: "/root".into(),
            target: "/root/reader".into(),
            body: "context".into(),
            mode: DeliveryMode::QueueOnly,
        });
        assert_eq!(control.wake_count("/root/reader"), 0);
        control.send_message(ControlMessage {
            sender: "/root".into(),
            target: "/root/reader".into(),
            body: "retry".into(),
            mode: DeliveryMode::TriggerTurn,
        });
        assert_eq!(control.wake_count("/root/reader"), 1);
        assert_eq!(control.drain_mailbox("/root/reader").len(), 2);
    }

    #[test]
    fn budget_is_tree_wide_and_fails_closed() {
        let control = ControlPlane::new(2, 10);
        control.record_usage(6).unwrap();
        assert!(control.record_usage(5).is_err());
        assert_eq!(control.snapshot().tokens_used, 6);
    }

    #[test]
    fn persisted_state_restores_without_live_reservations() {
        let control = ControlPlane::new(2, 100);
        control.set_task_status("task", TaskStatus::Running);
        control.record_usage(12).unwrap();
        let restored = ControlPlane::restore(control.snapshot());
        assert_eq!(restored.snapshot().tokens_used, 12);
        assert_eq!(
            restored.snapshot().task_statuses.get("task"),
            Some(&TaskStatus::Running)
        );
        assert!(restored.reserve_spawn("/root/reloaded").is_ok());
    }

    #[test]
    fn retry_queue_caps_attempts() {
        let mut queue = RetryQueue::default();
        queue.enqueue("task");
        queue.enqueue("task");
        queue.enqueue("task");
        assert_eq!(queue.next(2), Some(("task".into(), 1)));
        assert_eq!(queue.next(2), Some(("task".into(), 2)));
        assert_eq!(queue.next(2), None);
    }

    #[test]
    fn cancellation_persists_and_blocks_new_spawns() {
        let control = ControlPlane::new(1, 100);
        control.cancel();
        let restored = ControlPlane::restore(control.snapshot());
        assert!(restored.reserve_spawn("/root/late").is_err());
    }

    #[test]
    fn timeout_cleanup_releases_agent_path_and_writer_lease() {
        let control = ControlPlane::new(1, 100);
        control
            .reserve_spawn("/root/writer")
            .unwrap()
            .commit("timed-out")
            .unwrap();
        let lease = control.acquire_writer("task").unwrap();
        control.release_agent("timed-out");
        drop(lease);
        assert!(control.reserve_spawn("/root/writer").is_ok());
        assert!(control.acquire_writer("retry").is_ok());
    }

    #[test]
    fn retry_classification_distinguishes_transient_narrowed_and_terminal() {
        assert_eq!(
            classify_failure("worker crash"),
            RetryDisposition::RetrySameScope
        );
        assert_eq!(
            classify_failure("permission widening: read-only -> full"),
            RetryDisposition::RetryWithNarrowerPermission
        );
        assert_eq!(
            classify_failure("tree token budget exhausted"),
            RetryDisposition::Terminal
        );
    }
}
