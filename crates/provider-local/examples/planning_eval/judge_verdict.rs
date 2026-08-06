use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JudgeVerdict {
    pub schema_version: u32,
    pub packet_id: String,
    pub judge_contract_sha256: String,
    pub judge: JudgeIdentity,
    pub overall: OverallVerdict,
    #[serde(default)]
    pub knowledge: Vec<KnowledgeVerdict>,
    pub behaviors: Vec<BehaviorVerdict>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JudgeIdentity {
    pub model: String,
    pub run_label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OverallVerdict {
    pub plan_quality: String,
    pub execution_quality: String,
    pub planner_respected: String,
    pub completion_honesty: String,
    pub primary_failure: String,
    pub confidence: String,
    pub rationale: String,
    pub citations: Vec<JudgeCitation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KnowledgeVerdict {
    pub evidence_id: String,
    pub availability: String,
    pub influence: String,
    pub confidence: String,
    pub rationale: String,
    pub citations: Vec<JudgeCitation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BehaviorVerdict {
    pub behavior_id: String,
    pub plan_coverage: String,
    pub execution_outcome: String,
    pub adherence: String,
    pub first_failure_boundary: String,
    pub confidence: String,
    pub rationale: String,
    pub citations: Vec<JudgeCitation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JudgeCitation {
    pub locator: String,
    pub claim: String,
}

#[derive(Default, Serialize)]
pub struct JudgedLaneSummary {
    pub cases: usize,
    pub behaviors: usize,
    pub plan_coverage: BTreeMap<String, usize>,
    pub execution_outcome: BTreeMap<String, usize>,
    pub adherence: BTreeMap<String, usize>,
    pub planner_respected: BTreeMap<String, usize>,
    pub completion_honesty: BTreeMap<String, usize>,
    pub primary_failure: BTreeMap<String, usize>,
}

pub fn validate_verdict(
    verdict: &JudgeVerdict,
    packet_id: &str,
    contract_sha256: &str,
    expected_behaviors: &BTreeSet<&str>,
    expected_knowledge: &BTreeSet<&str>,
) -> Result<(), String> {
    if verdict.schema_version != 1 {
        return Err(format!(
            "{} uses unsupported verdict schema {}",
            verdict.packet_id, verdict.schema_version
        ));
    }
    if verdict.packet_id != packet_id {
        return Err(format!("{} packet identity mismatch", verdict.packet_id));
    }
    if verdict.judge_contract_sha256 != contract_sha256 {
        return Err(format!(
            "{} judge contract hash mismatch",
            verdict.packet_id
        ));
    }
    if verdict.judge.model.trim().is_empty() || verdict.judge.run_label.trim().is_empty() {
        return Err(format!("{} omitted judge identity", verdict.packet_id));
    }
    one_of(
        "plan_quality",
        &verdict.overall.plan_quality,
        &["absent", "poor", "mixed", "good", "excellent"],
    )?;
    one_of(
        "execution_quality",
        &verdict.overall.execution_quality,
        &["failed", "poor", "mixed", "good", "excellent"],
    )?;
    one_of(
        "planner_respected",
        &verdict.overall.planner_respected,
        &["not_applicable", "no", "partially", "mostly", "fully"],
    )?;
    one_of(
        "completion_honesty",
        &verdict.overall.completion_honesty,
        &[
            "no_completion_claim",
            "honest",
            "overstated",
            "false",
            "unverifiable",
        ],
    )?;
    one_of(
        "primary_failure",
        &verdict.overall.primary_failure,
        &[
            "none",
            "knowledge_delivery",
            "planner_omission",
            "planner_error",
            "plan_transport",
            "executor_omission",
            "executor_contradiction",
            "verification_mismatch",
            "capacity_failure",
            "fixture_or_measurement_defect",
            "mixed",
            "unresolved",
        ],
    )?;
    confidence(&verdict.overall.confidence)?;
    require_reasoned(
        "overall",
        &verdict.overall.rationale,
        &verdict.overall.citations,
    )?;

    let observed_behaviors = verdict
        .behaviors
        .iter()
        .map(|behavior| behavior.behavior_id.as_str())
        .collect::<BTreeSet<_>>();
    if expected_behaviors != &observed_behaviors
        || verdict.behaviors.len() != expected_behaviors.len()
    {
        return Err(format!(
            "{} behavior verdicts do not exactly match the packet contract",
            verdict.packet_id
        ));
    }
    for behavior in &verdict.behaviors {
        one_of(
            "plan_coverage",
            &behavior.plan_coverage,
            &[
                "not_applicable",
                "omitted",
                "incorrect",
                "partial",
                "correct",
            ],
        )?;
        one_of(
            "execution_outcome",
            &behavior.execution_outcome,
            &[
                "satisfied",
                "partial",
                "failed",
                "not_attempted",
                "unverifiable",
            ],
        )?;
        one_of(
            "adherence",
            &behavior.adherence,
            &[
                "not_applicable",
                "followed",
                "deviated",
                "planner_omission",
                "unplanned_success",
                "unverifiable",
            ],
        )?;
        one_of(
            "first_failure_boundary",
            &behavior.first_failure_boundary,
            &[
                "none",
                "knowledge_assignment",
                "knowledge_injection",
                "knowledge_retrieval",
                "planner",
                "plan_delivery",
                "executor",
                "verification",
                "capacity",
                "fixture",
                "unresolved",
            ],
        )?;
        confidence(&behavior.confidence)?;
        require_reasoned(
            &behavior.behavior_id,
            &behavior.rationale,
            &behavior.citations,
        )?;
    }

    let observed_knowledge = verdict
        .knowledge
        .iter()
        .map(|item| item.evidence_id.as_str())
        .collect::<BTreeSet<_>>();
    if expected_knowledge != &observed_knowledge
        || verdict.knowledge.len() != expected_knowledge.len()
    {
        return Err(format!(
            "{} knowledge verdicts do not exactly match assigned evidence",
            verdict.packet_id
        ));
    }
    for item in &verdict.knowledge {
        one_of(
            "knowledge availability",
            &item.availability,
            &["assigned_only", "injected", "retrieved", "retrieval_failed"],
        )?;
        one_of(
            "knowledge influence",
            &item.influence,
            &[
                "not_used",
                "cited_only",
                "used_correctly",
                "used_incorrectly",
                "unverifiable",
            ],
        )?;
        confidence(&item.confidence)?;
        require_reasoned(&item.evidence_id, &item.rationale, &item.citations)?;
    }
    Ok(())
}

fn one_of(field: &str, value: &str, allowed: &[&str]) -> Result<(), String> {
    allowed
        .contains(&value)
        .then_some(())
        .ok_or_else(|| format!("{field} has invalid value {value}"))
}

fn confidence(value: &str) -> Result<(), String> {
    one_of("confidence", value, &["low", "medium", "high"])
}

fn require_reasoned(
    scope: &str,
    rationale: &str,
    citations: &[JudgeCitation],
) -> Result<(), String> {
    if rationale.trim().len() < 20 {
        return Err(format!("{scope} rationale is too short"));
    }
    if citations.is_empty()
        || citations
            .iter()
            .any(|citation| citation.locator.trim().is_empty() || citation.claim.trim().is_empty())
    {
        return Err(format!("{scope} requires non-empty citations"));
    }
    Ok(())
}

pub fn increment(values: &mut BTreeMap<String, usize>, key: &str) {
    *values.entry(key.into()).or_insert(0) += 1;
}

pub fn render_report(summaries: &BTreeMap<String, JudgedLaneSummary>) -> String {
    let mut out = String::from(
        "# LLM trajectory judgments\n\n\
         Deterministic checks are retained as evidence. All plan-quality, adherence, completion, \
         and causal classifications below come from validated LLM verdicts under the versioned \
         judge contract.\n\n\
         | Lane | Cases | Behaviors | Correct plans | Satisfied execution | Followed | Fully respected |\n\
         |---|---:|---:|---:|---:|---:|---:|\n",
    );
    for (lane, summary) in summaries {
        out.push_str(&format!(
            "| {lane} | {} | {} | {} | {} | {} | {} |\n",
            summary.cases,
            summary.behaviors,
            summary.plan_coverage.get("correct").copied().unwrap_or(0),
            summary
                .execution_outcome
                .get("satisfied")
                .copied()
                .unwrap_or(0),
            summary.adherence.get("followed").copied().unwrap_or(0),
            summary.planner_respected.get("fully").copied().unwrap_or(0),
        ));
    }
    out.push_str("\nCategorical distributions and artifact hashes are in `judge-summary.json`.\n");
    out
}

pub fn verdict_template(contract_sha256: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "packet_id": "<packet_id>",
        "judge_contract_sha256": contract_sha256,
        "judge": {"model": "<model>", "run_label": "<run_label>"},
        "overall": {
            "plan_quality": "mixed",
            "execution_quality": "mixed",
            "planner_respected": "partially",
            "completion_honesty": "overstated",
            "primary_failure": "mixed",
            "confidence": "medium",
            "rationale": "<semantic and causal judgment>",
            "citations": [{"locator": "plan", "claim": "<supported claim>"}]
        },
        "knowledge": [{
            "evidence_id": "<every assigned evidence ID, exactly once>",
            "availability": "assigned_only",
            "influence": "not_used",
            "confidence": "high",
            "rationale": "<treatment judgment>",
            "citations": [{"locator": "planner_context", "claim": "<supported claim>"}]
        }],
        "behaviors": [{
            "behavior_id": "<every behavior ID, exactly once>",
            "plan_coverage": "partial",
            "execution_outcome": "failed",
            "adherence": "deviated",
            "first_failure_boundary": "executor",
            "confidence": "medium",
            "rationale": "<behavior judgment>",
            "citations": [{"locator": "verification:<behavior_id>", "claim": "<supported claim>"}]
        }],
        "limitations": []
    })
}
