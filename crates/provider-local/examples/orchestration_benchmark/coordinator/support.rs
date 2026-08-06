use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_core::domain::RunUsage;

use super::*;
use crate::model::{RunMetrics, TriggerMetrics};

pub(crate) fn planner_task(scenario: &Scenario) -> TaskContract {
    TaskContract {
        id: "plan".into(),
        logical_path: "/root/plan".into(),
        mode: TaskMode::ReadOnly,
        instruction: format!("Plan without editing: {}", scenario.prompt),
        dependencies: vec![],
        scope: scenario
            .initial_files
            .iter()
            .map(|file| file.path.clone())
            .collect(),
        acceptance: vec!["plan identifies scope and verification".into()],
        permission_ceiling: PermissionCeiling::ReadOnly,
        preferred_model_tier: "strong".into(),
    }
}

pub(crate) fn reader_task(reader: &scenarios::ReaderTask) -> TaskContract {
    TaskContract {
        id: format!("read-{}", reader.id),
        logical_path: format!("/root/read_{}", reader.id.replace('-', "_")),
        mode: TaskMode::ReadOnly,
        instruction: reader.instruction.clone(),
        dependencies: reader.dependencies.clone(),
        scope: reader.scope.clone(),
        acceptance: vec![format!("report evidence for {}", reader.expected_finding)],
        permission_ceiling: PermissionCeiling::ReadOnly,
        preferred_model_tier: if reader.cheap_model_eligible {
            "cheap".into()
        } else {
            "strong".into()
        },
    }
}

pub(crate) fn writer_task(scenario: &Scenario, prior: &[TaskContract]) -> TaskContract {
    TaskContract {
        id: "implement".into(),
        logical_path: "/root/implement".into(),
        mode: TaskMode::Write,
        instruction: scenario.prompt.clone(),
        dependencies: prior
            .iter()
            .filter(|task| task.mode == TaskMode::ReadOnly)
            .map(|task| task.id.clone())
            .collect(),
        scope: scenario.allowed_changed_paths.clone(),
        acceptance: scenario
            .hidden_checks
            .iter()
            .map(|check| format!("hidden rubric: {check:?}"))
            .collect(),
        permission_ceiling: PermissionCeiling::WorkspaceWrite,
        preferred_model_tier: "strong".into(),
    }
}

pub(crate) fn review_task(scenario: &Scenario, writer: &TaskContract) -> TaskContract {
    TaskContract {
        id: "review".into(),
        logical_path: "/root/review".into(),
        mode: TaskMode::Review,
        instruction: format!("Review the implementation for: {}", scenario.prompt),
        dependencies: vec![writer.id.clone()],
        scope: writer.scope.clone(),
        acceptance: vec!["return an evidence-backed accept or reject verdict".into()],
        permission_ceiling: PermissionCeiling::ReadOnly,
        preferred_model_tier: "strong".into(),
    }
}

pub(crate) fn verify_task(scenario: &Scenario, writer: &TaskContract) -> TaskContract {
    TaskContract {
        id: "verify".into(),
        logical_path: "/root/verify".into(),
        mode: TaskMode::Verify,
        instruction: format!("Run final verification for: {}", scenario.prompt),
        dependencies: vec![writer.id.clone()],
        scope: writer.scope.clone(),
        acceptance: vec!["authoritative hidden rubric passes".into()],
        permission_ceiling: PermissionCeiling::ReadOnly,
        preferred_model_tier: "strong".into(),
    }
}

pub(crate) fn trigger_metrics(
    scenario: &Scenario,
    lane: &LaneSpec,
    actual: bool,
) -> TriggerMetrics {
    let expected = scenario.expected_delegate;
    let reader_scope_union: BTreeSet<_> = scenario
        .reader_tasks
        .iter()
        .flat_map(|reader| reader.scope.iter().cloned())
        .collect();
    let relevant_files: BTreeSet<_> = scenario
        .initial_files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    let boundary_score = if !actual {
        (!expected) as u8 as f64
    } else {
        reader_scope_union.intersection(&relevant_files).count() as f64
            / reader_scope_union.len().max(1) as f64
    };
    let dependency_score = if scenario.family == "false_parallelism" {
        scenario
            .reader_tasks
            .iter()
            .all(|reader| !reader.dependencies.is_empty()) as u8 as f64
    } else {
        1.0
    };
    let cheap_eligible = scenario
        .reader_tasks
        .iter()
        .filter(|reader| reader.cheap_model_eligible)
        .count();
    let cheap_model_assignment_score = if cheap_eligible == 0
        || matches!(lane.kind, LaneKind::CheapSubagents | LaneKind::ClarkCloud)
    {
        1.0
    } else {
        0.0
    };
    let cloud_agent_assignment_score = match (scenario.cloud_agent_eligible, lane.cloud_agents) {
        (true, true) | (false, false) => 1.0,
        _ => 0.0,
    };
    TriggerMetrics {
        expected_delegate: expected,
        actual_delegate: actual,
        false_positive: actual && !expected,
        false_negative: !actual
            && expected
            && !matches!(lane.kind, LaneKind::Single | LaneKind::PlannedSingle),
        boundary_score,
        dependency_score,
        cheap_model_assignment_score,
        cloud_agent_assignment_score,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn metrics(
    attempts: &[AttemptRecord],
    correctness: f64,
    changed_path_precision: f64,
    recovered_failures: u32,
    unrecovered_failures: u32,
    review_catches: u32,
    review_false_vetoes: u32,
    duration_ms: u64,
    lane: &LaneSpec,
    actual_delegate: bool,
) -> RunMetrics {
    let usage = attempts
        .iter()
        .fold(RunUsage::default(), |mut total, attempt| {
            total.input_tokens += attempt.usage.input_tokens;
            total.output_tokens += attempt.usage.output_tokens;
            total.cost_usd =
                Some(total.cost_usd.unwrap_or(0.0) + attempt.usage.cost_usd.unwrap_or(0.0));
            total
        });
    let agent_millis = attempts.iter().map(|attempt| attempt.duration_ms).sum();
    let max_parallel_agents = if actual_delegate {
        lane.max_concurrency.min(attempts.len()).max(1)
    } else {
        1
    };
    let utilization = if duration_ms == 0 {
        0.0
    } else {
        (agent_millis as f64 / (duration_ms * max_parallel_agents as u64) as f64).min(1.0)
    };
    let lifecycle_trace_failures = attempts
        .iter()
        .filter(|attempt| attempt.provider != "acp" && !attempt.lifecycle_trace_replayable)
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    RunMetrics {
        correctness,
        changed_path_precision,
        recovered_failures,
        unrecovered_failures,
        review_catches,
        review_false_vetoes,
        interventions: 0,
        duration_ms,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        non_cached_input_tokens: 0,
        non_cached_input_available: false,
        cost_usd: usage.cost_usd.unwrap_or(0.0),
        agent_millis,
        redundant_reads: duplicate_tool_calls(attempts, "read_file"),
        root_executions: attempts
            .iter()
            .filter(|attempt| attempt.execution.is_some())
            .count()
            .try_into()
            .unwrap_or(u32::MAX),
        root_attempts: attempts
            .iter()
            .filter_map(|attempt| attempt.execution.as_ref())
            .map(|execution| execution.attempts)
            .sum(),
        root_recoveries: attempts
            .iter()
            .filter_map(|attempt| attempt.execution.as_ref())
            .map(|execution| execution.recoveries)
            .sum(),
        lifecycle_trace_failures,
        duplicate_tool_receipts: attempts
            .iter()
            .map(|attempt| attempt.duplicate_tool_receipts)
            .sum(),
        cloud_agent_calls: tool_call_count(attempts, "clark_research"),
        unmetered_external_calls: tool_call_count(attempts, "clark_research"),
        max_parallel_agents,
        utilization,
    }
}

fn duplicate_tool_calls(attempts: &[AttemptRecord], tool: &str) -> u32 {
    tool_call_count(attempts, tool).saturating_sub(1)
}

fn tool_call_count(attempts: &[AttemptRecord], tool: &str) -> u32 {
    attempts
        .iter()
        .map(|attempt| {
            attempt
                .tool_calls
                .iter()
                .filter(|name| name == &tool)
                .count()
        })
        .sum::<usize>()
        .try_into()
        .unwrap_or(u32::MAX)
}

pub(crate) fn final_task_statuses(
    control: &ControlPlane,
    tasks: &[TaskContract],
    correctness: f64,
    writer_succeeded: bool,
) -> BTreeMap<String, TaskStatus> {
    let mut statuses = control.snapshot().task_statuses;
    for task in tasks {
        let status = if task.mode == TaskMode::Write {
            if writer_succeeded && correctness >= 1.0 {
                TaskStatus::Accepted
            } else {
                TaskStatus::Failed
            }
        } else {
            TaskStatus::Verified
        };
        statuses.insert(task.id.clone(), status);
    }
    statuses
}

pub(crate) async fn checkpoint(seeded: &SeededRepository) -> Option<String> {
    provider_local::create_checkpoint(&provider_local::LocalExecutor, &seeded.root)
        .await
        .ok()
        .flatten()
}

pub(crate) fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}
