use std::collections::{BTreeSet, HashSet};

use serde::Serialize;
use uuid::Uuid;

const MIN_DEEP_PASSES: usize = 3;
const REQUIRED_ZERO_NOVELTY_PASSES: usize = 2;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityDeepTaskReceipt {
    pub task_id: String,
    pub attempt: u32,
    pub claim_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityDeepPassReceipt {
    pub orchestration_id: String,
    pub focus: String,
    pub tasks: Vec<SecurityDeepTaskReceipt>,
    pub candidate_ids: Option<Vec<String>>,
    pub novel_candidate_ids: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityDeepStatus {
    pub run_id: String,
    pub scan_id: String,
    pub inventory_id: String,
    pub minimum_passes: usize,
    pub required_zero_novelty_passes: usize,
    pub passes: Vec<SecurityDeepPassReceipt>,
    pub saturated: bool,
}

#[derive(Default)]
pub(crate) struct SecurityDeepLedger {
    active: Option<DeepRun>,
}

struct DeepRun {
    run_id: String,
    scan_id: String,
    inventory_id: String,
    passes: Vec<SecurityDeepPassReceipt>,
}

impl SecurityDeepLedger {
    pub(crate) fn begin(
        &mut self,
        scan_id: &str,
        inventory_id: &str,
    ) -> Result<SecurityDeepStatus, String> {
        if scan_id.trim().is_empty() {
            return Err("scan_id must not be empty".into());
        }
        if inventory_id.trim().is_empty() {
            return Err("inventory id must not be empty".into());
        }
        if let Some(active) = &self.active {
            if active.scan_id == scan_id && active.inventory_id == inventory_id {
                return Ok(self.status().expect("active run has status"));
            }
            return Err(format!(
                "deep run `{}` is already active for scan `{}`",
                active.run_id, active.scan_id
            ));
        }
        self.active = Some(DeepRun {
            run_id: format!("security-deep-{}", Uuid::new_v4()),
            scan_id: scan_id.to_string(),
            inventory_id: inventory_id.to_string(),
            passes: Vec::new(),
        });
        Ok(self.status().expect("new run has status"))
    }

    pub(crate) fn record_orchestration(
        &mut self,
        orchestration_id: &str,
        focus: &str,
        mut tasks: Vec<SecurityDeepTaskReceipt>,
    ) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if active
            .passes
            .iter()
            .any(|pass| pass.orchestration_id == orchestration_id)
        {
            return;
        }
        tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        active.passes.push(SecurityDeepPassReceipt {
            orchestration_id: orchestration_id.to_string(),
            focus: focus.trim().to_string(),
            tasks,
            candidate_ids: None,
            novel_candidate_ids: None,
        });
    }

    pub(crate) fn checkpoint(
        &mut self,
        run_id: &str,
        orchestration_id: &str,
        candidate_ids: Vec<String>,
    ) -> Result<SecurityDeepStatus, String> {
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| "no deep security run is active".to_string())?;
        if active.run_id != run_id {
            return Err("deep_run_id does not match the active run".into());
        }
        let pass_index = active
            .passes
            .iter()
            .position(|pass| pass.orchestration_id == orchestration_id)
            .ok_or_else(|| {
                "orchestration_id is not an accepted read-only delegation receipt".to_string()
            })?;
        if active.passes[pass_index].candidate_ids.is_some() {
            return Err("the deep pass is already checkpointed".into());
        }
        let candidate_ids = normalize_candidate_ids(candidate_ids)?;
        let prior = active.passes[..pass_index]
            .iter()
            .flat_map(|pass| pass.candidate_ids.iter().flatten())
            .cloned()
            .collect::<BTreeSet<_>>();
        let novel = candidate_ids
            .iter()
            .filter(|candidate| !prior.contains(*candidate))
            .cloned()
            .collect::<Vec<_>>();
        active.passes[pass_index].candidate_ids = Some(candidate_ids);
        active.passes[pass_index].novel_candidate_ids = Some(novel);
        Ok(self.status().expect("active run has status"))
    }

    pub(crate) fn status(&self) -> Option<SecurityDeepStatus> {
        let active = self.active.as_ref()?;
        Some(SecurityDeepStatus {
            run_id: active.run_id.clone(),
            scan_id: active.scan_id.clone(),
            inventory_id: active.inventory_id.clone(),
            minimum_passes: MIN_DEEP_PASSES,
            required_zero_novelty_passes: REQUIRED_ZERO_NOVELTY_PASSES,
            passes: active.passes.clone(),
            saturated: saturated(&active.passes),
        })
    }

    pub(super) fn validate(
        &self,
        run_id: &str,
        scan_id: &str,
        inventory_id: &str,
        candidate_ids: &BTreeSet<String>,
    ) -> Result<usize, String> {
        let status = self
            .status()
            .ok_or_else(|| "no deep security run is active".to_string())?;
        if status.run_id != run_id
            || status.scan_id != scan_id
            || status.inventory_id != inventory_id
        {
            return Err("deep run does not match the scan or repository snapshot".into());
        }
        if status.passes.len() < MIN_DEEP_PASSES {
            return Err(format!(
                "deep scan requires at least {MIN_DEEP_PASSES} accepted independent passes"
            ));
        }
        if status
            .passes
            .iter()
            .any(|pass| pass.tasks.is_empty() || pass.candidate_ids.is_none())
        {
            return Err("every deep pass needs accepted tasks and a candidate checkpoint".into());
        }
        let normalized_focus = status
            .passes
            .iter()
            .map(|pass| normalize_focus(&pass.focus))
            .collect::<HashSet<_>>();
        if normalized_focus.len() != status.passes.len() || normalized_focus.contains("") {
            return Err("deep passes require distinct non-empty discovery focuses".into());
        }
        if !status.saturated {
            return Err(format!(
                "deep scan has not reached {REQUIRED_ZERO_NOVELTY_PASSES} consecutive zero-novelty passes"
            ));
        }
        let observed = status
            .passes
            .iter()
            .flat_map(|pass| pass.candidate_ids.iter().flatten())
            .cloned()
            .collect::<BTreeSet<_>>();
        if &observed != candidate_ids {
            let missing = observed
                .difference(candidate_ids)
                .take(5)
                .cloned()
                .collect::<Vec<_>>();
            let unregistered = candidate_ids
                .difference(&observed)
                .take(5)
                .cloned()
                .collect::<Vec<_>>();
            return Err(format!(
                "deep candidate reduction does not match the final bundle; missing={missing:?}, unregistered={unregistered:?}"
            ));
        }
        Ok(status.passes.len())
    }
}

fn normalize_candidate_ids(candidate_ids: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = BTreeSet::new();
    for candidate in candidate_ids {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            return Err("candidate_ids must not contain empty values".into());
        }
        if !normalized.insert(candidate.to_string()) {
            return Err(format!(
                "duplicate candidate id `{candidate}` in deep checkpoint"
            ));
        }
    }
    Ok(normalized.into_iter().collect())
}

fn normalize_focus(focus: &str) -> String {
    focus
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn saturated(passes: &[SecurityDeepPassReceipt]) -> bool {
    passes.len() >= MIN_DEEP_PASSES
        && passes
            .iter()
            .rev()
            .take(REQUIRED_ZERO_NOVELTY_PASSES)
            .all(|pass| pass.novel_candidate_ids.as_ref().is_some_and(Vec::is_empty))
}
