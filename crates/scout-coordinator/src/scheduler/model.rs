use std::collections::{BTreeMap, BTreeSet};

use scout_adapter_protocol::{
    AdapterId, AdapterQuery, AuthContextHandle, AuthContextId, CoverageBinding, TargetId,
};
use scout_scheduler::{
    QuotaKey, ScheduleManifest, Scheduler, SchedulerReceipt, SchedulerTaskId, TaskSpec, TaskStatus,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SchedulerImage {
    pub(super) manifest: ScheduleManifest,
    pub(super) tasks: BTreeMap<SchedulerTaskId, TaskRecordImage>,
    pub(super) quotas: BTreeMap<QuotaKey, QuotaRuntimeImage>,
    pub(super) generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TaskRecordImage {
    pub(super) spec: TaskSpec,
    pub(super) status: TaskStatus,
    pub(super) attempts: u16,
    pub(super) fence: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct QuotaRuntimeImage {
    pub(super) in_flight: BTreeSet<SchedulerTaskId>,
    pub(super) next_start_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BindingImage {
    target_id: TargetId,
    adapter_id: AdapterId,
    auth_context_id: AuthContextId,
    auth_context_handle: AuthContextHandle,
    coverage: CoverageBinding,
    query: AdapterQuery,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TaskColumns<'a> {
    pub(super) state_kind: &'static str,
    pub(super) ready_at_ms: u64,
    pub(super) lease_machine_id: Option<&'a str>,
    pub(super) lease_expires_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LeaseState<'a> {
    pub(super) machine_id: &'a str,
    pub(super) fence: u64,
    pub(super) expires_at_ms: u64,
}

impl SchedulerImage {
    pub(super) fn from_scheduler(scheduler: &Scheduler) -> Result<Self, String> {
        serde_json::from_slice(&scheduler.encode()?).map_err(|error| error.to_string())
    }

    pub(super) fn to_scheduler(&self) -> Result<Scheduler, String> {
        let encoded = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        Scheduler::decode(&encoded)
    }

    pub(super) fn receipt(&self) -> Result<SchedulerReceipt, String> {
        self.to_scheduler()?.receipt()
    }
}

impl BindingImage {
    pub(super) fn from_spec(spec: &TaskSpec) -> Self {
        Self {
            target_id: spec.target_id.clone(),
            adapter_id: spec.adapter_id.clone(),
            auth_context_id: spec.auth_context_id.clone(),
            auth_context_handle: spec.auth_context_handle.clone(),
            coverage: spec.coverage.clone(),
            query: spec.query.clone(),
        }
    }

    pub(super) fn id(&self) -> Result<String, String> {
        Ok(format!(
            "scheduler-binding:{}",
            canonical_sha256(&("scout-scheduler-binding-v1", self))?
        ))
    }
}

impl TaskRecordImage {
    pub(super) fn columns(&self) -> TaskColumns<'_> {
        match &self.status {
            TaskStatus::Pending { not_before_ms } => TaskColumns {
                state_kind: "pending",
                ready_at_ms: *not_before_ms,
                lease_machine_id: None,
                lease_expires_at_ms: None,
            },
            TaskStatus::Leased {
                machine_id,
                expires_at_ms,
                ..
            } => TaskColumns {
                state_kind: "leased",
                ready_at_ms: 0,
                lease_machine_id: Some(machine_id),
                lease_expires_at_ms: Some(*expires_at_ms),
            },
            TaskStatus::RetryWait { not_before_ms, .. } => TaskColumns {
                state_kind: "retry_wait",
                ready_at_ms: *not_before_ms,
                lease_machine_id: None,
                lease_expires_at_ms: None,
            },
            TaskStatus::Terminal { .. } => TaskColumns {
                state_kind: "terminal",
                ready_at_ms: 0,
                lease_machine_id: None,
                lease_expires_at_ms: None,
            },
        }
    }

    pub(super) fn lease(&self) -> Option<LeaseState<'_>> {
        match &self.status {
            TaskStatus::Leased {
                machine_id,
                fence,
                expires_at_ms,
            } => Some(LeaseState {
                machine_id,
                fence: *fence,
                expires_at_ms: *expires_at_ms,
            }),
            _ => None,
        }
    }
}

pub(super) fn canonical_sha256<T: Serialize>(value: &T) -> Result<String, String> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| format!("scheduler canonical encoding failed: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}
