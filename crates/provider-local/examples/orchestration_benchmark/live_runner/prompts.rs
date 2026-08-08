use crate::model::{ReviewFinding, TaskContract};
use crate::scenarios::Scenario;

const HANDOFF_RULES: &str = r#"Finish with exactly one JSON object and no prose after it. Use this shape:
{
  "task_id": "TASK_ID",
  "attempt_id": "ATTEMPT_ID",
  "reported_status": "reported",
  "changed_paths": ["relative/path"],
  "baseline_checkpoint": null,
  "result_checkpoint": null,
  "commands": [{"command":"...","exit_code":0,"output_artifact":null}],
  "tests": [{"name":"...","passed":true,"output_artifact":null}],
  "claims": [{"evidence_ref":"path or command","claim":"..."}],
  "unresolved": [],
  "artifact_refs": [],
  "summary": "concise result"
}
Use an empty changed_paths array for read-only work. Never claim a command or test you did not run."#;

fn contract(task: &TaskContract, attempt_id: &str) -> String {
    HANDOFF_RULES
        .replace("TASK_ID", &task.id)
        .replace("ATTEMPT_ID", attempt_id)
}

pub fn reader(scenario: &Scenario, task: &TaskContract, attempt_id: &str, cloud: bool) -> String {
    let cloud_instruction = if cloud {
        "You MUST call product_research exactly once. Give it the synthetic compatibility statement and ask it to independently challenge the proposed interpretation. Treat its response as advisory evidence and reconcile it with the local files."
    } else {
        "Do not edit files or run mutating commands. Inspect only the scoped repository evidence."
    };
    format!(
        "You are a bounded read-only coding subagent in an orchestration benchmark.\n\
         Overall task: {}\n\
         Your subtask: {}\n\
         Allowed scope: {:?}\n\
         Expected deliverable: {:?}\n\
         {cloud_instruction}\n\
         Report concrete path-level evidence useful to the writer.\n\n{}",
        scenario.prompt,
        task.instruction,
        task.scope,
        task.acceptance,
        contract(task, attempt_id),
    )
}

pub fn writer(
    scenario: &Scenario,
    task: &TaskContract,
    attempt_id: &str,
    findings: &[String],
    planned_single: bool,
    rework: &[ReviewFinding],
) -> String {
    let planning = if planned_single {
        "Before editing, form a concise internal plan and inspect every relevant contract."
    } else {
        "Inspect the relevant contracts before editing."
    };
    let evidence = if findings.is_empty() {
        "No delegated findings were supplied.".to_string()
    } else {
        format!("Delegated read-only findings:\n- {}", findings.join("\n- "))
    };
    let rework = if rework.is_empty() {
        String::new()
    } else {
        format!(
            "\nThis is a rework attempt. Resolve these reviewer findings:\n{}",
            serde_json::to_string_pretty(rework).unwrap_or_default()
        )
    };
    format!(
        "You are the sole writer in a synthetic Git repository. Complete the task and produce real working changes.\n\
         Task: {}\n\
         Allowed changed paths: {:?}\n\
         Do not alter files outside that set. Preserve pre-existing dirty user changes. Do not commit, push, access the network, or use destructive Git commands.\n\
         {planning}\n\
         {evidence}{rework}\n\
         Run the smallest useful local verification available.\n\n{}",
        scenario.prompt,
        task.scope,
        contract(task, attempt_id),
    )
}

pub fn reviewer(scenario: &Scenario, task: &TaskContract, attempt_id: &str) -> String {
    format!(
        "You are an independent read-only reviewer. Inspect the current repository changes for this task:\n{}\n\
         Check correctness, completeness, allowed-path scope, and whether the evidence matches disk state. Do not edit.\n\
         Finish with exactly one JSON object and no prose after it:\n\
         {{\"task_id\":\"{}\",\"findings\":[{{\"severity\":\"high\",\"path\":null,\"evidence_ref\":\"path or command\",\"message\":\"...\"}}],\"accepted\":true}}\n\
         Set accepted=false for any material defect. Use an empty findings array only when accepting.\n\
         Benchmark attempt id for traceability: {attempt_id}",
        scenario.prompt, task.id,
    )
}

pub fn verifier(scenario: &Scenario, task: &TaskContract, attempt_id: &str) -> String {
    format!(
        "You are the final read-only verifier for this synthetic repository task:\n{}\n\
         Inspect the resulting diff and run safe, relevant verification if available. Do not edit. Report unresolved failures honestly.\n\n{}",
        scenario.prompt,
        contract(task, attempt_id),
    )
}
