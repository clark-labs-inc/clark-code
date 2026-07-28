use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    canonical_sha256, validate_digest, validate_identifier, SchedulerReceipt, SchedulerTaskId,
    TaskOrigin, TaskRecord, TaskStatus, TerminalDisposition,
};

use super::Scheduler;

impl Scheduler {
    pub fn receipt(&self) -> Result<SchedulerReceipt, String> {
        self.validate()?;
        let mut pending = 0;
        let mut leased = 0;
        let mut retry_wait = 0;
        let mut terminal = 0;
        let mut complete_terminal = 0;
        for record in self.tasks.values() {
            match record.status {
                TaskStatus::Pending { .. } => pending += 1,
                TaskStatus::Leased { .. } => leased += 1,
                TaskStatus::RetryWait { .. } => retry_wait += 1,
                TaskStatus::Terminal { disposition, .. } => {
                    terminal += 1;
                    if disposition.is_complete() {
                        complete_terminal += 1;
                    }
                }
            }
        }
        let state_sha256 = canonical_sha256(&(
            "scout-scheduler-state-v1",
            &self.manifest,
            self.generation,
            &self.tasks,
            &self.quotas,
        ))?;
        Ok(SchedulerReceipt {
            manifest_id: self.manifest.manifest_id.clone(),
            generation: self.generation,
            tasks: self.tasks.len(),
            pending,
            leased,
            retry_wait,
            terminal,
            complete_terminal,
            gap_terminal: terminal - complete_terminal,
            sealed: terminal == self.tasks.len(),
            complete: terminal == self.tasks.len() && terminal == complete_terminal,
            state_sha256,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        self.manifest.validate()?;
        if self.generation == 0 {
            return Err("scheduler generation must be positive".into());
        }
        let root_ids = self
            .tasks
            .values()
            .filter(|record| matches!(record.spec.origin, TaskOrigin::Root))
            .map(|record| record.spec.task_id.clone())
            .collect::<BTreeSet<_>>();
        if root_ids != self.manifest.root_task_ids {
            return Err("scheduler roots disagree with the manifest".into());
        }
        let mut expected_in_flight = self
            .manifest
            .quota_policies
            .keys()
            .cloned()
            .map(|key| (key, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        let mut expansion_counts = BTreeMap::<(SchedulerTaskId, String), u32>::new();
        for (task_id, record) in &self.tasks {
            record.spec.validate()?;
            if task_id != &record.spec.task_id {
                return Err("scheduler task map key disagrees with its specification".into());
            }
            if record.spec.enterprise_id != self.manifest.enterprise_id
                || record.spec.charter_id != self.manifest.charter_id
                || record.spec.discovery_epoch != self.manifest.discovery_epoch
            {
                return Err("scheduler task is outside its manifest boundary".into());
            }
            let quota_key = record.spec.quota_key();
            if !self.manifest.quota_policies.contains_key(&quota_key) {
                return Err("scheduler task has no manifest quota policy".into());
            }
            self.validate_origin(record, &mut expansion_counts)?;
            match &record.status {
                TaskStatus::Pending { .. } => {}
                TaskStatus::Leased {
                    machine_id,
                    fence,
                    expires_at_ms,
                } => {
                    validate_identifier("scheduler machine id", machine_id, 256)?;
                    if *fence == 0 || *fence != record.fence || *expires_at_ms == 0 {
                        return Err("scheduler lease fence or expiry is invalid".into());
                    }
                    if record.attempts == 0 {
                        return Err("scheduler leased task has no attempt".into());
                    }
                    expected_in_flight
                        .get_mut(&quota_key)
                        .expect("quota existence checked")
                        .insert(task_id.clone());
                }
                TaskStatus::RetryWait { error_sha256, .. } => {
                    validate_digest("scheduler retry error", error_sha256)?;
                    if record.attempts == 0 {
                        return Err("scheduler retry task has no attempt".into());
                    }
                }
                TaskStatus::Terminal {
                    disposition,
                    receipt_id,
                    evidence_sha256,
                } => {
                    validate_digest("scheduler terminal evidence", evidence_sha256)?;
                    if disposition.is_complete() && receipt_id.is_none() {
                        return Err("complete scheduler terminal state requires a receipt".into());
                    }
                    if record.attempts == 0 {
                        return Err("scheduler terminal task has no attempt".into());
                    }
                }
            }
        }
        if self.quotas.len() != self.manifest.quota_policies.len() {
            return Err("scheduler quota runtime disagrees with the manifest".into());
        }
        for (key, runtime) in &self.quotas {
            let policy = self
                .manifest
                .quota_policies
                .get(key)
                .ok_or_else(|| "scheduler has an undeclared quota runtime".to_string())?;
            if runtime.in_flight
                != *expected_in_flight
                    .get(key)
                    .ok_or_else(|| "scheduler quota expectation is missing".to_string())?
            {
                return Err("scheduler quota in-flight set disagrees with leases".into());
            }
            if runtime.in_flight.len() > usize::from(policy.max_in_flight) {
                return Err("scheduler quota exceeds max_in_flight".into());
            }
        }
        Ok(())
    }

    fn validate_origin(
        &self,
        record: &TaskRecord,
        expansion_counts: &mut BTreeMap<(SchedulerTaskId, String), u32>,
    ) -> Result<(), String> {
        let Some(parent_id) = record.spec.origin.parent() else {
            return Ok(());
        };
        let parent = self
            .tasks
            .get(parent_id)
            .ok_or_else(|| "scheduler child references an unknown parent".to_string())?;
        if !matches!(
            parent.status,
            TaskStatus::Terminal {
                disposition: TerminalDisposition::Succeeded,
                ..
            }
        ) {
            return Err("scheduler child parent is not terminal-successful".into());
        }
        match &record.spec.origin {
            TaskOrigin::Continuation { .. } => {
                if record.spec.enterprise_id != parent.spec.enterprise_id
                    || record.spec.charter_id != parent.spec.charter_id
                    || record.spec.discovery_epoch != parent.spec.discovery_epoch
                    || record.spec.target_id != parent.spec.target_id
                    || record.spec.adapter_id != parent.spec.adapter_id
                    || record.spec.auth_context_id != parent.spec.auth_context_id
                    || record.spec.auth_context_handle != parent.spec.auth_context_handle
                    || record.spec.coverage != parent.spec.coverage
                    || record.spec.query != parent.spec.query
                    || record.spec.page_ordinal != parent.spec.page_ordinal.saturating_add(1)
                {
                    return Err("scheduler continuation changes its bound route".into());
                }
            }
            TaskOrigin::Expansion { rule_id, .. } => {
                let rule = self
                    .manifest
                    .expansion_rules
                    .get(rule_id)
                    .ok_or_else(|| "scheduler expansion uses an undeclared rule".to_string())?;
                if parent.spec.route_kind() != rule.parent
                    || record.spec.route_kind() != rule.child
                    || (rule.same_target && record.spec.target_id != parent.spec.target_id)
                {
                    return Err("scheduler expansion violates its manifest rule".into());
                }
                let count = expansion_counts
                    .entry((parent_id.clone(), rule_id.clone()))
                    .or_default();
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| "scheduler expansion count overflow".to_string())?;
                if *count > rule.max_children_per_parent {
                    return Err("scheduler expansion exceeds its manifest child limit".into());
                }
            }
            TaskOrigin::Root => unreachable!("parent was present"),
        }
        Ok(())
    }
}
