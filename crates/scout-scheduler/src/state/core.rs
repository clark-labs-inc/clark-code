use std::collections::BTreeMap;

use crate::model::{SchedulerTaskId, TaskOrigin, TaskRecord, TaskSpec, TaskStatus};
use crate::ScheduleManifest;

use super::{QuotaRuntime, Scheduler};

impl Scheduler {
    pub fn new(
        manifest: ScheduleManifest,
        root_tasks: Vec<TaskSpec>,
        now_ms: u64,
    ) -> Result<Self, String> {
        manifest.validate()?;
        let mut tasks = BTreeMap::new();
        for spec in root_tasks {
            spec.validate()?;
            if !matches!(spec.origin, TaskOrigin::Root) {
                return Err("scheduler initialization accepts root tasks only".into());
            }
            if tasks
                .insert(
                    spec.task_id.clone(),
                    TaskRecord {
                        spec,
                        status: TaskStatus::Pending {
                            not_before_ms: now_ms,
                        },
                        attempts: 0,
                        fence: 0,
                    },
                )
                .is_some()
            {
                return Err("scheduler root task is duplicated".into());
            }
        }
        let quotas = manifest
            .quota_policies
            .keys()
            .cloned()
            .map(|key| (key, QuotaRuntime::default()))
            .collect();
        let scheduler = Self {
            manifest,
            tasks,
            quotas,
            generation: 1,
        };
        scheduler.validate()?;
        Ok(scheduler)
    }

    pub fn manifest(&self) -> &ScheduleManifest {
        &self.manifest
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn task_status(&self, task_id: &SchedulerTaskId) -> Option<&TaskStatus> {
        self.tasks.get(task_id).map(|record| &record.status)
    }

    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| format!("scheduler encoding failed: {error}"))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let scheduler: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("scheduler decoding failed: {error}"))?;
        scheduler.validate()?;
        Ok(scheduler)
    }
}
