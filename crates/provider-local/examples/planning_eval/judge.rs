use crate::context::sha256;
use crate::fixtures::scenarios;
use crate::judge_verdict::{
    increment, render_report, validate_verdict, verdict_template, JudgeVerdict, JudgedLaneSummary,
};
use crate::model::{
    CaseRecord, ContextReceipt, EvidenceRole, EvidenceSource, HandoffReceipt, RetrievalReceipt,
    RetryReceipt, TrajectoryReceipt, Verification,
};
use crate::runner::{snapshot_files, tree_digest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const CONTRACT: &str = include_str!("JUDGE_CONTRACT_V1.md");

#[derive(Clone, Debug, Serialize)]
struct JudgePacket {
    schema_version: u32,
    packet_id: String,
    judge_contract_sha256: String,
    source: PacketSource,
    task: String,
    private_reference_plan: String,
    behavior_contracts: Vec<BehaviorContract>,
    evidence_catalog: Vec<JudgeEvidence>,
    plan: Option<String>,
    handoff: HandoffReceipt,
    planner_context: NormalizedContext,
    executor_context: NormalizedContext,
    planner_trajectory: TrajectoryReceipt,
    executor_trajectory: TrajectoryReceipt,
    baseline_tree_sha256: String,
    baseline_files: BTreeMap<String, String>,
    final_tree_sha256: String,
    final_files: BTreeMap<String, String>,
    hidden_verification: Verification,
    retries: Vec<RetryReceipt>,
    provider_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct PacketSource {
    run_id: String,
    source_schema_version: u32,
    source_record_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
struct BehaviorContract {
    id: String,
    requirement: String,
}

#[derive(Clone, Debug, Serialize)]
struct JudgeEvidence {
    id: String,
    source: EvidenceSource,
    role: EvidenceRole,
    text: String,
}

#[derive(Clone, Debug, Serialize)]
struct NormalizedContext {
    assigned_evidence_ids: Vec<String>,
    injected_evidence_ids: Vec<String>,
    injected_context: String,
    context_sha256: String,
    retrievals: Vec<RetrievalReceipt>,
    legacy_receipt_correction: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct JudgeIndex {
    schema_version: u32,
    judge_contract_sha256: String,
    packets_sha256: String,
    entries: Vec<JudgeIndexEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct JudgeIndexEntry {
    packet_id: String,
    scenario: String,
    lane: String,
    repetition: usize,
}

pub fn run(input: &Path, output: &Path, verdicts: Option<&Path>) -> Result<(), String> {
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let (packets, index) = export_packets(input, output)?;
    if let Some(verdicts) = verdicts {
        ingest_verdicts(output, verdicts, &packets, &index)?;
    }
    println!(
        "{}",
        output
            .canonicalize()
            .unwrap_or_else(|_| output.to_path_buf())
            .display()
    );
    Ok(())
}

fn export_packets(
    input: &Path,
    output: &Path,
) -> Result<(BTreeMap<String, JudgePacket>, JudgeIndex), String> {
    let results_path = if input.is_dir() {
        input.join("results.jsonl")
    } else {
        input.to_path_buf()
    };
    let body = std::fs::read_to_string(&results_path)
        .map_err(|error| format!("failed to read {}: {error}", results_path.display()))?;
    let records = body
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<CaseRecord>(line)
                .map_err(|error| format!("results line {}: {error}", index + 1))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if records.is_empty() {
        return Err("judge input contains no case records".into());
    }

    let available = scenarios()
        .into_iter()
        .map(|scenario| (scenario.id, scenario))
        .collect::<BTreeMap<_, _>>();
    let contract_sha256 = sha256(CONTRACT);
    let mut packets = BTreeMap::new();
    let mut entries = Vec::new();
    let mut jsonl = String::new();
    for record in &records {
        let scenario = available
            .get(record.scenario.as_str())
            .ok_or_else(|| format!("unknown scenario in retained result: {}", record.scenario))?;
        let source_record =
            serde_json::to_string(record).map_err(|error| format!("serialize record: {error}"))?;
        let source_record_sha256 = sha256(&source_record);
        let packet_id = sha256(&format!(
            "judge-v1:{contract_sha256}:{}:{}:{}:{}:{source_record_sha256}",
            record.run_id, record.scenario, record.lane, record.repetition
        ));
        let baseline = tempfile::tempdir().map_err(|error| error.to_string())?;
        (scenario.seed)(baseline.path())?;
        let packet = JudgePacket {
            schema_version: 1,
            packet_id: packet_id.clone(),
            judge_contract_sha256: contract_sha256.clone(),
            source: PacketSource {
                run_id: record.run_id.clone(),
                source_schema_version: record.schema_version,
                source_record_sha256,
            },
            task: scenario.task.into(),
            private_reference_plan: scenario.oracle_plan.into(),
            behavior_contracts: scenario
                .semantic_plan_checks
                .iter()
                .map(|check| BehaviorContract {
                    id: check.id.into(),
                    requirement: check.expectation.into(),
                })
                .collect(),
            evidence_catalog: scenario
                .evidence
                .iter()
                .map(|item| JudgeEvidence {
                    id: item.id.into(),
                    source: item.source,
                    role: item.role,
                    text: item.text.into(),
                })
                .collect(),
            plan: record.plan.clone(),
            handoff: record.handoff.clone(),
            planner_context: normalize_context(&record.planner_context, record.schema_version),
            executor_context: normalize_context(&record.executor_context, record.schema_version),
            planner_trajectory: record.planner_trajectory.clone(),
            executor_trajectory: record.executor_trajectory.clone(),
            baseline_tree_sha256: tree_digest(baseline.path())?,
            baseline_files: snapshot_files(baseline.path())?,
            final_tree_sha256: record.executor_tree_sha256.clone(),
            final_files: record.executor_files.clone(),
            hidden_verification: record.verification.clone(),
            retries: record.retries.clone(),
            provider_error: record.error.clone(),
        };
        let serialized =
            serde_json::to_string(&packet).map_err(|error| format!("serialize packet: {error}"))?;
        jsonl.push_str(&serialized);
        jsonl.push('\n');
        entries.push(JudgeIndexEntry {
            packet_id: packet_id.clone(),
            scenario: record.scenario.clone(),
            lane: record.lane.clone(),
            repetition: record.repetition,
        });
        packets.insert(packet_id, packet);
    }
    let packets_sha256 = sha256(&jsonl);
    let index = JudgeIndex {
        schema_version: 1,
        judge_contract_sha256: contract_sha256.clone(),
        packets_sha256,
        entries,
    };
    std::fs::write(output.join("judge-packets.jsonl"), jsonl).map_err(|error| error.to_string())?;
    write_json(&output.join("judge-index.json"), &index)?;
    std::fs::write(output.join("JUDGE_INSTRUCTIONS.md"), CONTRACT)
        .map_err(|error| error.to_string())?;
    write_json(
        &output.join("verdict-template.json"),
        &verdict_template(&contract_sha256),
    )?;
    Ok((packets, index))
}

fn normalize_context(receipt: &ContextReceipt, source_schema: u32) -> NormalizedContext {
    let mut assigned = receipt.assigned_evidence_ids.clone();
    let mut injected = receipt.injected_evidence_ids.clone();
    let mut correction = None;
    if source_schema <= 4 && assigned.is_empty() && !injected.is_empty() {
        assigned = injected.clone();
        if receipt.injected_context.is_empty() && receipt.context_sha256 == sha256("") {
            injected.clear();
            correction = Some(
                "schema-v4 deferred-discovery receipt stored assigned IDs as injected; empty \
                 context hash proves that no evidence bytes were injected"
                    .into(),
            );
        }
    }
    NormalizedContext {
        assigned_evidence_ids: assigned,
        injected_evidence_ids: injected,
        injected_context: receipt.injected_context.clone(),
        context_sha256: receipt.context_sha256.clone(),
        retrievals: receipt.retrievals.clone(),
        legacy_receipt_correction: correction,
    }
}

fn ingest_verdicts(
    output: &Path,
    verdict_path: &Path,
    packets: &BTreeMap<String, JudgePacket>,
    index: &JudgeIndex,
) -> Result<(), String> {
    let body = std::fs::read_to_string(verdict_path)
        .map_err(|error| format!("failed to read {}: {error}", verdict_path.display()))?;
    let verdicts = body
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(line, value)| {
            serde_json::from_str::<JudgeVerdict>(value)
                .map_err(|error| format!("verdict line {}: {error}", line + 1))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut by_packet = BTreeMap::new();
    for verdict in verdicts {
        let packet = packets
            .get(&verdict.packet_id)
            .ok_or_else(|| format!("verdict references unknown packet {}", verdict.packet_id))?;
        let expected_behaviors = packet
            .behavior_contracts
            .iter()
            .map(|behavior| behavior.id.as_str())
            .collect::<BTreeSet<_>>();
        let expected_knowledge = packet
            .planner_context
            .assigned_evidence_ids
            .iter()
            .chain(&packet.executor_context.assigned_evidence_ids)
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        validate_verdict(
            &verdict,
            &packet.packet_id,
            &packet.judge_contract_sha256,
            &expected_behaviors,
            &expected_knowledge,
        )?;
        if by_packet
            .insert(verdict.packet_id.clone(), verdict)
            .is_some()
        {
            return Err("duplicate packet verdict".into());
        }
    }
    if by_packet.len() != packets.len() {
        let missing = packets
            .keys()
            .filter(|id| !by_packet.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "expected {} verdicts, received {}; missing: {}",
            packets.len(),
            by_packet.len(),
            missing.join(",")
        ));
    }

    let mut summaries: BTreeMap<String, JudgedLaneSummary> = BTreeMap::new();
    let mut judged_jsonl = String::new();
    for entry in &index.entries {
        let verdict = &by_packet[&entry.packet_id];
        let summary = summaries.entry(entry.lane.clone()).or_default();
        summary.cases += 1;
        summary.behaviors += verdict.behaviors.len();
        increment(
            &mut summary.planner_respected,
            &verdict.overall.planner_respected,
        );
        increment(
            &mut summary.completion_honesty,
            &verdict.overall.completion_honesty,
        );
        increment(
            &mut summary.primary_failure,
            &verdict.overall.primary_failure,
        );
        for behavior in &verdict.behaviors {
            increment(&mut summary.plan_coverage, &behavior.plan_coverage);
            increment(&mut summary.execution_outcome, &behavior.execution_outcome);
            increment(&mut summary.adherence, &behavior.adherence);
        }
        let row = serde_json::json!({"case": entry, "verdict": verdict});
        judged_jsonl.push_str(
            &serde_json::to_string(&row).map_err(|error| format!("serialize verdict: {error}"))?,
        );
        judged_jsonl.push('\n');
    }
    std::fs::write(output.join("judged-results.jsonl"), judged_jsonl)
        .map_err(|error| error.to_string())?;
    write_json(
        &output.join("judge-summary.json"),
        &serde_json::json!({
            "schema_version": 1,
            "judge_contract_sha256": index.judge_contract_sha256,
            "packets_sha256": index.packets_sha256,
            "verdicts_sha256": sha256(&body),
            "cases": packets.len(),
            "lanes": summaries,
        }),
    )?;
    std::fs::write(output.join("judge-report.md"), render_report(&summaries))
        .map_err(|error| error.to_string())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let body = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, format!("{body}\n")).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_empty_context_repairs_assigned_vs_injected() {
        let receipt = ContextReceipt {
            assigned_evidence_ids: Vec::new(),
            injected_evidence_ids: vec!["PROJECT-ONE".into()],
            injected_context: String::new(),
            context_sha256: sha256(""),
            retrievals: Vec::new(),
        };
        let normalized = normalize_context(&receipt, 4);
        assert_eq!(normalized.assigned_evidence_ids, ["PROJECT-ONE"]);
        assert!(normalized.injected_evidence_ids.is_empty());
        assert!(normalized.legacy_receipt_correction.is_some());
    }

    #[test]
    fn current_context_preserves_actual_injection() {
        let receipt = ContextReceipt {
            assigned_evidence_ids: vec!["ORG-ONE".into()],
            injected_evidence_ids: vec!["ORG-ONE".into()],
            injected_context: "capsule".into(),
            context_sha256: sha256("capsule"),
            retrievals: Vec::new(),
        };
        let normalized = normalize_context(&receipt, 5);
        assert_eq!(normalized.injected_evidence_ids, ["ORG-ONE"]);
        assert!(normalized.legacy_receipt_correction.is_none());
    }
}
