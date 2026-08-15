use std::collections::BTreeSet;
use std::path::Path;

use agent_core::provider::ProviderConfig;
use agent_orchestration::{
    ChangePackageDescriptor, MultiRepoPlan, MultiRepoTask, ReaderReport, ReviewDecision, TaskId,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

pub(super) fn isolated_provider_config(
    mut config: ProviderConfig,
    cwd: &Path,
    model: &str,
    writable: bool,
) -> Result<ProviderConfig, String> {
    let mut extra = match config.extra {
        Value::Object(map) => map,
        Value::Null => Map::new(),
        _ => return Err("provider extra config must be a JSON object".into()),
    };
    let permission = if writable { "allow" } else { "deny" };
    extra.insert("model".into(), Value::String(model.to_string()));
    extra.insert("isolated_writer".into(), Value::Bool(true));
    extra.insert("memories".into(), Value::Bool(false));
    extra.insert("project_knowledge".into(), Value::Bool(false));
    extra.insert("browser_enabled".into(), Value::Bool(false));
    extra.insert("mcp_servers".into(), Value::Array(Vec::new()));
    extra.insert("command_allowlist".into(), Value::Array(Vec::new()));
    extra.insert("command_denylist".into(), Value::Array(Vec::new()));
    extra.remove("remote");
    extra.insert(
        "permissions".into(),
        json!({
            "write_file": permission,
            "edit_file": permission,
            "apply_patch": permission,
            "bash": "deny",
            "bash_input": "deny",
            "bash_kill": "deny",
            "browser": "deny"
        }),
    );
    config.cwd = Some(cwd.to_string_lossy().into_owned());
    config.extra = Value::Object(extra);
    Ok(config)
}

pub(super) fn writer_prompt(task: &MultiRepoTask, plan: &MultiRepoPlan) -> String {
    let decisions = plan
        .contract_decisions
        .iter()
        .map(|decision| {
            format!(
                "{}: {} ({})",
                decision.edge_id, decision.artifact_sha256, decision.compatibility_rule
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are one bounded repository writer in a multi-repository implementation.\n\
         Implement working code; do not stop at a plan. Your filesystem is a disposable clone.\n\
         You may change exactly these repository-relative paths: {:?}.\n\
         Shell execution is unavailable. Use read_file, grep, glob, edit_file, write_file, or apply_patch.\n\
         Do not access user memory, external tools, other repositories, or paths outside this clone.\n\
         Task: {}\n\
         Pinned cross-repository decisions:\n{}\n\
         Finish only after the requested files contain the implementation. Do not claim tests you could not run.",
        task.allowed_changed_paths, task.objective, decisions
    )
}

pub(super) fn reader_prompt(task: &MultiRepoTask) -> String {
    format!(
        "You are a bounded, read-only repository reader helping a stronger implementation agent.\n\
         Inspect only this disposable repository clone. Never edit files or request broader permissions.\n\
         Find the smallest set of concrete evidence needed for this task: {}\n\
         Return exactly one JSON object and no trailing prose:\n\
         {{\"evidence_refs\":[\"relative/path:line\"],\"summary\":\"concise implementation-relevant finding\"}}",
        task.objective
    )
}

#[derive(Deserialize)]
struct ReaderReportBody {
    evidence_refs: Vec<String>,
    summary: String,
}

pub(super) fn parse_reader(text: &str, task: &MultiRepoTask) -> Result<ReaderReport, String> {
    let body: ReaderReportBody = extract_json(text)
        .ok_or_else(|| "reader did not return the required JSON receipt".to_string())?;
    Ok(ReaderReport {
        task_id: task.id.clone(),
        repository_id: task
            .repository_id
            .clone()
            .ok_or_else(|| "reader task has no repository".to_string())?,
        evidence_refs: body.evidence_refs,
        summary: body.summary,
    })
}

pub(super) fn review_prompt(task: &MultiRepoTask, packages: &[ChangePackageDescriptor]) -> String {
    let package_summary = packages
        .iter()
        .map(|package| {
            format!(
                "{} {} {:?}",
                package.task_id, package.patch_sha256, package.changed_paths
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are the independent, read-only reviewer for a freshly replayed multi-repository change.\n\
         Review correctness, cross-repository compatibility, and obvious regressions. Never edit files.\n\
         Objective: {}\nPackages:\n{}\n\
         Return exactly one JSON object and no trailing prose:\n\
         {{\"findings\":[\"relative/path:line - specific finding\"],\"decision\":\"accept|rework\",\"rework_task_ids\":[\"writer-task-id\"]}}",
        task.objective, package_summary
    )
}

#[derive(Deserialize)]
struct ReviewReport {
    #[serde(default)]
    findings: Vec<String>,
    decision: ReviewDecision,
    #[serde(default)]
    rework_task_ids: Vec<String>,
}

pub(super) struct ParsedReview {
    pub(super) decision: ReviewDecision,
    pub(super) rework_task_ids: BTreeSet<TaskId>,
    pub(super) findings: Vec<String>,
}

pub(super) fn parse_review(text: &str) -> Result<ParsedReview, String> {
    let report: ReviewReport = extract_json(text)
        .ok_or_else(|| "reviewer did not return the required JSON receipt".to_string())?;
    let rework_task_ids = report
        .rework_task_ids
        .into_iter()
        .map(TaskId::new)
        .collect::<Result<_, _>>()?;
    Ok(ParsedReview {
        decision: report.decision,
        rework_task_ids,
        findings: report.findings,
    })
}

fn extract_json<T: for<'de> Deserialize<'de>>(text: &str) -> Option<T> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Some(value);
    }
    let end = trimmed.rfind('}')?;
    trimmed[..=end]
        .char_indices()
        .rev()
        .filter(|(_, character)| *character == '{')
        .find_map(|(start, _)| serde_json::from_str(&trimmed[start..=end]).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_json_can_be_recovered_after_leading_prose() {
        let parsed = parse_review(
            "review notes\n{\"decision\":\"rework\",\"rework_task_ids\":[\"api-writer\"],\"findings\":[\"bad contract\"]}",
        )
        .unwrap();
        assert_eq!(parsed.decision, ReviewDecision::Rework);
        assert!(parsed
            .rework_task_ids
            .contains(&TaskId::new("api-writer").unwrap()));
    }

    #[test]
    fn reader_receipt_identity_comes_from_the_host_task() {
        let task = MultiRepoTask {
            id: TaskId::new("reader").unwrap(),
            role: agent_orchestration::MultiRepoTaskRole::Reader,
            repository_id: Some(agent_orchestration::RepositoryId::new("api").unwrap()),
            dependencies: BTreeSet::new(),
            objective: "inspect".into(),
            harness: "local".into(),
            harness_kind: agent_orchestration::HarnessKind::Local,
            model: "cheap".into(),
            model_tier: agent_orchestration::ModelTier::Cheap,
            budget_reservation: 1_000,
            allowed_changed_paths: BTreeSet::new(),
        };
        let report = parse_reader(
            r#"{"summary":"found call site","evidence_refs":["src/api.rs:2"]}"#,
            &task,
        )
        .unwrap();
        assert_eq!(report.task_id, task.id);
        assert_eq!(report.repository_id.as_str(), "api");

        let reader = reader_prompt(&task);
        assert!(reader.find("evidence_refs").unwrap() < reader.find("summary").unwrap());
        let review = review_prompt(&task, &[]);
        assert!(review.find("findings").unwrap() < review.find("decision").unwrap());
    }
}
