use std::collections::{BTreeMap, BTreeSet};

use scout_adapter_protocol::TargetId;
use serde::{Deserialize, Serialize};

mod core;
mod validation;

use crate::model::{
    canonical_sha256, validate_digest, validate_identifier, CompletionDisposition, LeaseClaim,
    PageCompletion, RetryClass, SchedulerTaskId, TaskRecord, TaskSpec, TaskStatus,
    TerminalDisposition,
};
use crate::{QuotaKey, ScheduleManifest};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct QuotaRuntime {
    in_flight: BTreeSet<SchedulerTaskId>,
    next_start_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scheduler {
    manifest: ScheduleManifest,
    tasks: BTreeMap<SchedulerTaskId, TaskRecord>,
    quotas: BTreeMap<QuotaKey, QuotaRuntime>,
    generation: u64,
}

impl Scheduler {
    pub fn claim(
        &mut self,
        machine_id: &str,
        eligible_targets: &BTreeSet<TargetId>,
        now_ms: u64,
        max_tasks: usize,
    ) -> Result<Vec<LeaseClaim>, String> {
        validate_identifier("scheduler machine id", machine_id, 256)?;
        if eligible_targets.is_empty() {
            return Err("scheduler claim requires at least one eligible target".into());
        }
        if max_tasks == 0 || max_tasks > 1_024 {
            return Err("scheduler claim size must be in 1..=1024".into());
        }
        let mut next = self.clone();
        let expired = next.reap_expired_inner(now_ms)?;
        let mut candidates = next
            .tasks
            .values()
            .filter(|record| {
                eligible_targets.contains(&record.spec.target_id)
                    && match record.status {
                        TaskStatus::Pending { not_before_ms }
                        | TaskStatus::RetryWait { not_before_ms, .. } => not_before_ms <= now_ms,
                        TaskStatus::Leased { .. } | TaskStatus::Terminal { .. } => false,
                    }
            })
            .map(|record| {
                let ready_at = match record.status {
                    TaskStatus::Pending { not_before_ms }
                    | TaskStatus::RetryWait { not_before_ms, .. } => not_before_ms,
                    TaskStatus::Leased { .. } | TaskStatus::Terminal { .. } => u64::MAX,
                };
                (record.spec.priority, ready_at, record.spec.task_id.clone())
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });

        let mut claims = Vec::new();
        for (_, _, task_id) in candidates {
            if claims.len() >= max_tasks {
                break;
            }
            let record = next
                .tasks
                .get(&task_id)
                .ok_or_else(|| "scheduler candidate disappeared".to_string())?;
            let quota_key = record.spec.quota_key();
            let policy = next
                .manifest
                .quota_policies
                .get(&quota_key)
                .ok_or_else(|| "scheduler task has no quota policy".to_string())?;
            let runtime = next
                .quotas
                .get(&quota_key)
                .ok_or_else(|| "scheduler quota runtime is missing".to_string())?;
            if runtime.in_flight.len() >= usize::from(policy.max_in_flight)
                || now_ms < runtime.next_start_at_ms
            {
                continue;
            }
            let expires_at_ms = now_ms
                .checked_add(policy.lease_duration_ms)
                .ok_or_else(|| "scheduler lease expiry overflow".to_string())?;
            let record = next
                .tasks
                .get_mut(&task_id)
                .ok_or_else(|| "scheduler candidate disappeared".to_string())?;
            record.attempts = record
                .attempts
                .checked_add(1)
                .ok_or_else(|| "scheduler attempt counter overflow".to_string())?;
            record.fence = record
                .fence
                .checked_add(1)
                .ok_or_else(|| "scheduler fence overflow".to_string())?;
            record.status = TaskStatus::Leased {
                machine_id: machine_id.to_owned(),
                fence: record.fence,
                expires_at_ms,
            };
            let runtime = next
                .quotas
                .get_mut(&quota_key)
                .ok_or_else(|| "scheduler quota runtime is missing".to_string())?;
            runtime.in_flight.insert(task_id.clone());
            runtime.next_start_at_ms = now_ms.saturating_add(policy.min_start_interval_ms);
            claims.push(LeaseClaim {
                task: record.spec.clone(),
                machine_id: machine_id.to_owned(),
                fence: record.fence,
                attempt: record.attempts,
                expires_at_ms,
            });
        }
        if expired > 0 || !claims.is_empty() {
            next.generation = next
                .generation
                .checked_add(1)
                .ok_or_else(|| "scheduler generation overflow".to_string())?;
            next.validate()?;
            *self = next;
        }
        Ok(claims)
    }

    pub fn heartbeat(
        &mut self,
        task_id: &SchedulerTaskId,
        machine_id: &str,
        fence: u64,
        now_ms: u64,
    ) -> Result<u64, String> {
        validate_identifier("scheduler machine id", machine_id, 256)?;
        let mut next = self.clone();
        let record = next
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| "scheduler task is unknown".to_string())?;
        let TaskStatus::Leased {
            machine_id: owner,
            fence: active_fence,
            expires_at_ms,
        } = &record.status
        else {
            return Err("scheduler task is not leased".into());
        };
        if owner != machine_id || *active_fence != fence {
            return Err("scheduler heartbeat has a stale owner or fence".into());
        }
        if now_ms > *expires_at_ms {
            return Err("scheduler lease expired before heartbeat".into());
        }
        let policy = next
            .manifest
            .quota_policies
            .get(&record.spec.quota_key())
            .ok_or_else(|| "scheduler task has no quota policy".to_string())?;
        let renewed = now_ms
            .checked_add(policy.lease_duration_ms)
            .ok_or_else(|| "scheduler lease expiry overflow".to_string())?;
        record.status = TaskStatus::Leased {
            machine_id: machine_id.to_owned(),
            fence,
            expires_at_ms: renewed,
        };
        next.generation = next
            .generation
            .checked_add(1)
            .ok_or_else(|| "scheduler generation overflow".to_string())?;
        next.validate()?;
        *self = next;
        Ok(renewed)
    }

    pub fn complete(&mut self, completion: PageCompletion) -> Result<(), String> {
        let mut next = self.clone();
        next.complete_inner(completion)?;
        next.generation = next
            .generation
            .checked_add(1)
            .ok_or_else(|| "scheduler generation overflow".to_string())?;
        next.validate()?;
        *self = next;
        Ok(())
    }

    pub fn reap_expired(&mut self, now_ms: u64) -> Result<usize, String> {
        let mut next = self.clone();
        let expired = next.reap_expired_inner(now_ms)?;
        if expired > 0 {
            next.generation = next
                .generation
                .checked_add(1)
                .ok_or_else(|| "scheduler generation overflow".to_string())?;
            next.validate()?;
            *self = next;
        }
        Ok(expired)
    }

    fn complete_inner(&mut self, completion: PageCompletion) -> Result<(), String> {
        validate_identifier("scheduler machine id", &completion.machine_id, 256)?;
        let record = self
            .tasks
            .get(&completion.task_id)
            .ok_or_else(|| "scheduler completion task is unknown".to_string())?;
        let TaskStatus::Leased {
            machine_id,
            fence,
            expires_at_ms,
        } = &record.status
        else {
            return Err("scheduler completion task is not leased".into());
        };
        if machine_id != &completion.machine_id || *fence != completion.fence {
            return Err("scheduler completion has a stale owner or fence".into());
        }
        if completion.completed_at_ms > *expires_at_ms {
            return Err("scheduler completion arrived after lease expiry".into());
        }
        let spec = record.spec.clone();
        let attempts = record.attempts;
        let quota_key = spec.quota_key();
        let policy = *self
            .manifest
            .quota_policies
            .get(&quota_key)
            .ok_or_else(|| "scheduler task has no quota policy".to_string())?;
        self.quotas
            .get_mut(&quota_key)
            .ok_or_else(|| "scheduler quota runtime is missing".to_string())?
            .in_flight
            .remove(&completion.task_id);

        match completion.disposition {
            CompletionDisposition::Retry {
                class,
                retry_after_ms,
                error_sha256,
            } => {
                if completion.receipt_id.is_some()
                    || completion.evidence_sha256.is_some()
                    || completion.continuation.is_some()
                    || !completion.expansions.is_empty()
                {
                    return Err("scheduler retry completion cannot carry terminal children".into());
                }
                validate_digest("scheduler retry error", &error_sha256)?;
                if attempts >= policy.max_attempts {
                    self.tasks
                        .get_mut(&completion.task_id)
                        .expect("task exists")
                        .status = TaskStatus::Terminal {
                        disposition: TerminalDisposition::RetryExhausted,
                        receipt_id: None,
                        evidence_sha256: error_sha256,
                    };
                } else {
                    let delay = retry_delay(policy, attempts, retry_after_ms);
                    let not_before_ms = completion.completed_at_ms.saturating_add(delay);
                    self.tasks
                        .get_mut(&completion.task_id)
                        .expect("task exists")
                        .status = TaskStatus::RetryWait {
                        not_before_ms,
                        class,
                        error_sha256,
                    };
                    if matches!(class, RetryClass::RateLimited) {
                        let quota = self
                            .quotas
                            .get_mut(&quota_key)
                            .expect("quota existence checked");
                        quota.next_start_at_ms = quota.next_start_at_ms.max(not_before_ms);
                    }
                }
                return Ok(());
            }
            CompletionDisposition::Success { final_page } => {
                let (receipt_id, evidence_sha256) =
                    terminal_evidence(&completion.receipt_id, &completion.evidence_sha256)?;
                if final_page == completion.continuation.is_some() {
                    return Err("scheduler success continuation disagrees with final_page".into());
                }
                self.tasks
                    .get_mut(&completion.task_id)
                    .expect("task exists")
                    .status = TaskStatus::Terminal {
                    disposition: TerminalDisposition::Succeeded,
                    receipt_id: Some(receipt_id),
                    evidence_sha256,
                };
            }
            CompletionDisposition::Empty => {
                let (receipt_id, evidence_sha256) =
                    terminal_evidence(&completion.receipt_id, &completion.evidence_sha256)?;
                if completion.continuation.is_some() || !completion.expansions.is_empty() {
                    return Err("empty scheduler completion cannot create children".into());
                }
                self.tasks
                    .get_mut(&completion.task_id)
                    .expect("task exists")
                    .status = TaskStatus::Terminal {
                    disposition: TerminalDisposition::Empty,
                    receipt_id: Some(receipt_id),
                    evidence_sha256,
                };
            }
            CompletionDisposition::Gap { terminal } => {
                if terminal.is_complete() || terminal == TerminalDisposition::RetryExhausted {
                    return Err("scheduler gap disposition is not an external terminal gap".into());
                }
                let (receipt_id, evidence_sha256) =
                    terminal_evidence(&completion.receipt_id, &completion.evidence_sha256)?;
                if completion.continuation.is_some() || !completion.expansions.is_empty() {
                    return Err("scheduler gap completion cannot create children".into());
                }
                self.tasks
                    .get_mut(&completion.task_id)
                    .expect("task exists")
                    .status = TaskStatus::Terminal {
                    disposition: terminal,
                    receipt_id: Some(receipt_id),
                    evidence_sha256,
                };
            }
        }

        if let Some(continuation) = completion.continuation {
            self.register_child(&completion.task_id, continuation)?;
        }
        for expansion in completion.expansions {
            self.register_child(&completion.task_id, expansion)?;
        }
        Ok(())
    }

    fn register_child(
        &mut self,
        parent_task_id: &SchedulerTaskId,
        spec: TaskSpec,
    ) -> Result<(), String> {
        spec.validate()?;
        if spec.origin.parent() != Some(parent_task_id) {
            return Err("scheduler child does not name its completing parent".into());
        }
        if let Some(existing) = self.tasks.get(&spec.task_id) {
            if existing.spec == spec {
                return Ok(());
            }
            return Err("scheduler task id collides with different child content".into());
        }
        let task_id = spec.task_id.clone();
        self.tasks.insert(
            task_id,
            TaskRecord {
                spec,
                status: TaskStatus::Pending { not_before_ms: 0 },
                attempts: 0,
                fence: 0,
            },
        );
        Ok(())
    }

    fn reap_expired_inner(&mut self, now_ms: u64) -> Result<usize, String> {
        let expired = self
            .tasks
            .iter()
            .filter_map(|(task_id, record)| match record.status {
                TaskStatus::Leased { expires_at_ms, .. } if expires_at_ms < now_ms => {
                    Some(task_id.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for task_id in &expired {
            let record = self.tasks.get(task_id).expect("expired task exists");
            let quota_key = record.spec.quota_key();
            let policy = *self
                .manifest
                .quota_policies
                .get(&quota_key)
                .ok_or_else(|| "scheduler task has no quota policy".to_string())?;
            let attempts = record.attempts;
            self.quotas
                .get_mut(&quota_key)
                .ok_or_else(|| "scheduler quota runtime is missing".to_string())?
                .in_flight
                .remove(task_id);
            let error_sha256 =
                canonical_sha256(&("scout-scheduler-lease-expired-v1", task_id, attempts))?;
            let status = if attempts >= policy.max_attempts {
                TaskStatus::Terminal {
                    disposition: TerminalDisposition::RetryExhausted,
                    receipt_id: None,
                    evidence_sha256: error_sha256,
                }
            } else {
                TaskStatus::RetryWait {
                    not_before_ms: now_ms.saturating_add(retry_delay(policy, attempts, None)),
                    class: RetryClass::TransientTransport,
                    error_sha256,
                }
            };
            self.tasks
                .get_mut(task_id)
                .expect("expired task exists")
                .status = status;
        }
        Ok(expired.len())
    }
}

fn terminal_evidence(
    receipt_id: &Option<String>,
    evidence_sha256: &Option<String>,
) -> Result<(String, String), String> {
    let receipt_id = receipt_id
        .as_ref()
        .ok_or_else(|| "scheduler terminal completion requires a receipt".to_string())?;
    if !receipt_id.starts_with("receipt:") {
        return Err("scheduler receipt id must start with receipt:".into());
    }
    let evidence_sha256 = evidence_sha256
        .as_ref()
        .ok_or_else(|| "scheduler terminal completion requires evidence".to_string())?;
    validate_digest("scheduler completion evidence", evidence_sha256)?;
    Ok((receipt_id.clone(), evidence_sha256.clone()))
}

fn retry_delay(policy: crate::QuotaPolicy, attempts: u16, retry_after_ms: Option<u64>) -> u64 {
    let exponent = u32::from(attempts.saturating_sub(1)).min(20);
    let calculated = policy
        .base_backoff_ms
        .saturating_mul(1_u64 << exponent)
        .min(policy.max_backoff_ms);
    calculated.max(retry_after_ms.unwrap_or(0).min(policy.max_backoff_ms))
}
