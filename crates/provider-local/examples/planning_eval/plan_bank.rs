use crate::model::{
    ContextReceipt, KnowledgeDelivery, RetrievalTreatmentReceipt, RetryReceipt, RouteReceipt,
    TrajectoryReceipt, UsageReceipt,
};
use crate::model::{EvidenceSource, Scenario};
use crate::retry::{receipt, retryable_error, wait_with_progress, PHASE_DELAYS};
use crate::route::LiveConfig;
use crate::runner::{
    build_planner_prompt, connect_provider, drive_connected, phase_retry, tree_digest, DriveOptions,
};
use agent_core::domain::{ProposedPlan, ProposedPlanStatus};
use agent_core::provider::CollaborationMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BankSourceSet {
    None,
    All,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct PlanBankKey {
    pub scenario: String,
    pub repetition: usize,
    pub source_set: BankSourceSet,
    #[serde(default)]
    pub knowledge_delivery: KnowledgeDelivery,
    pub planning_profile: String,
    pub route_model: String,
    pub fixture_sha256: String,
    pub task_prompt_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanBankEntry {
    pub schema_version: u32,
    pub key: PlanBankKey,
    pub proposal: ProposedPlan,
    pub proposal_sha256: String,
    #[serde(default)]
    pub planning_contract: String,
    #[serde(default)]
    pub task_prompt: String,
    pub planner_context: ContextReceipt,
    #[serde(default)]
    pub project_memory_files: BTreeMap<String, String>,
    pub planner_usage: UsageReceipt,
    pub planner_trajectory: TrajectoryReceipt,
    pub planner_retries: Vec<RetryReceipt>,
    #[serde(default)]
    pub retrieval_treatment: RetrievalTreatmentReceipt,
    pub planner_read_only: bool,
    pub error: Option<String>,
}

impl PlanBankEntry {
    pub fn bank_id(&self) -> String {
        let identity = serde_json::json!({
            "key": self.key,
            "proposal_id": self.proposal.id,
            "proposal_revision": self.proposal.revision,
            "proposal_sha256": self.proposal_sha256,
        });
        crate::context::sha256(&identity.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.schema_version, 1..=4) {
            return Err(format!(
                "unsupported plan-bank schema {}",
                self.schema_version
            ));
        }
        let actual = crate::context::sha256(&self.proposal.markdown);
        if actual != self.proposal_sha256 {
            return Err(format!(
                "plan-bank proposal hash mismatch for {}",
                self.key.scenario
            ));
        }
        if self.schema_version >= 2
            && crate::context::sha256(&self.task_prompt) != self.key.task_prompt_sha256
        {
            return Err(format!(
                "plan-bank task prompt hash mismatch for {}",
                self.key.scenario
            ));
        }
        if self.schema_version >= 2
            && self.planning_contract
                != provider_local::planning_prompt_contract_for_eval(&self.key.planning_profile)
        {
            return Err(format!(
                "plan-bank planning contract mismatch for {}",
                self.key.scenario
            ));
        }
        if self.schema_version >= 2 {
            match (self.key.source_set, self.key.knowledge_delivery) {
                (BankSourceSet::None, _) if !self.project_memory_files.is_empty() => {
                    return Err("plan-bank none treatment retained project memory".into());
                }
                (BankSourceSet::All, KnowledgeDelivery::PrefetchedCapsule)
                    if !self.project_memory_files.is_empty() =>
                {
                    return Err("prefetched plan-bank treatment retained project memory".into());
                }
                (BankSourceSet::All, delivery)
                    if delivery != KnowledgeDelivery::PrefetchedCapsule
                        && self.project_memory_files.is_empty() =>
                {
                    return Err("plan-bank all treatment omitted project memory".into());
                }
                _ => {}
            }
        }
        if self.schema_version >= 3
            && self.retrieval_treatment.knowledge_delivery != self.key.knowledge_delivery
        {
            return Err(format!(
                "plan-bank delivery receipt mismatch for {}",
                self.key.scenario
            ));
        }
        if !self.planner_read_only {
            return Err(format!(
                "plan-bank planner mutated its workspace for {}",
                self.key.scenario
            ));
        }
        if self.error.is_some() {
            return Err(format!(
                "plan-bank entry retained a planner error for {}",
                self.key.scenario
            ));
        }
        Ok(())
    }
}

pub struct PlanBank {
    path: PathBuf,
    entries: BTreeMap<PlanBankKey, PlanBankEntry>,
}

impl PlanBank {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let mut entries = BTreeMap::new();
        if let Ok(body) = std::fs::read_to_string(&path) {
            for (index, line) in body.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let entry: PlanBankEntry = serde_json::from_str(line)
                    .map_err(|error| format!("{} line {}: {error}", path.display(), index + 1))?;
                entry.validate()?;
                if let Some(previous) = entries.insert(entry.key.clone(), entry.clone()) {
                    if previous.proposal_sha256 != entry.proposal_sha256
                        || previous.proposal.id != entry.proposal.id
                        || previous.proposal.revision != entry.proposal.revision
                    {
                        return Err(format!(
                            "conflicting duplicate plan-bank key at {} line {}",
                            path.display(),
                            index + 1
                        ));
                    }
                }
            }
        }
        Ok(Self { path, entries })
    }

    pub fn get(&self, key: &PlanBankKey) -> Option<&PlanBankEntry> {
        self.entries.get(key)
    }

    pub fn find(
        &self,
        scenario: &str,
        repetition: usize,
        source_set: BankSourceSet,
        knowledge_delivery: KnowledgeDelivery,
        profile: &str,
        route_model: &str,
    ) -> Result<&PlanBankEntry, String> {
        let matches = self
            .entries
            .values()
            .filter(|entry| {
                entry.key.scenario == scenario
                    && entry.key.repetition == repetition
                    && entry.key.source_set == source_set
                    && entry.key.knowledge_delivery == knowledge_delivery
                    && entry.key.planning_profile == profile
                    && entry.key.route_model == route_model
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [entry] => Ok(*entry),
            [] => Err(format!(
                "missing plan-bank entry for {scenario} repetition {repetition} {source_set:?} {knowledge_delivery:?}"
            )),
            _ => Err(format!(
                "ambiguous plan-bank entries for {scenario} repetition {repetition} {source_set:?} {knowledge_delivery:?}"
            )),
        }
    }

    pub fn insert(&mut self, entry: PlanBankEntry) -> Result<(), String> {
        entry.validate()?;
        if let Some(previous) = self.entries.get(&entry.key) {
            if previous.proposal_sha256 == entry.proposal_sha256
                && previous.proposal.id == entry.proposal.id
                && previous.proposal.revision == entry.proposal.revision
            {
                return Ok(());
            }
            return Err(format!(
                "refusing conflicting plan-bank entry for {} repetition {}",
                entry.key.scenario, entry.key.repetition
            ));
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| error.to_string())?;
        serde_json::to_writer(&mut file, &entry).map_err(|error| error.to_string())?;
        writeln!(file).map_err(|error| error.to_string())?;
        file.flush().map_err(|error| error.to_string())?;
        self.entries.insert(entry.key.clone(), entry);
        Ok(())
    }

    pub fn ensure_offline_reference(
        &mut self,
        scenario: &Scenario,
        repetition: usize,
        source_set: BankSourceSet,
        knowledge_delivery: KnowledgeDelivery,
        profile: &str,
    ) -> Result<(), String> {
        let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
        (scenario.seed)(workspace.path())?;
        let fixture_sha256 = crate::runner::tree_digest(workspace.path())?;
        let sources = match source_set {
            BankSourceSet::None => Vec::new(),
            BankSourceSet::All => vec![
                EvidenceSource::Project,
                EvidenceSource::Org,
                EvidenceSource::Scout,
            ],
        };
        let evidence = crate::context::select_evidence(scenario, &sources);
        if knowledge_delivery != KnowledgeDelivery::PrefetchedCapsule {
            crate::context::seed_project_memory(workspace.path(), &evidence)?;
        }
        let project_memory_files = crate::context::snapshot_project_memory(workspace.path())?;
        let (packet, planner_context) = match knowledge_delivery {
            KnowledgeDelivery::PrefetchedCapsule => {
                crate::context::prefetched_planner_packet(&evidence)
            }
            _ => crate::context::context_packet(&evidence),
        };
        let task_prompt =
            crate::runner::build_planner_prompt(scenario, &packet, knowledge_delivery);
        let key = PlanBankKey {
            scenario: scenario.id.into(),
            repetition,
            source_set,
            knowledge_delivery,
            planning_profile: profile.into(),
            route_model: "deterministic-reference".into(),
            fixture_sha256,
            task_prompt_sha256: crate::context::sha256(&task_prompt),
        };
        if self.get(&key).is_some() {
            return Ok(());
        }
        let source_name = match source_set {
            BankSourceSet::None => "none",
            BankSourceSet::All => "all",
        };
        let delivery_name = knowledge_delivery_name(knowledge_delivery);
        let proposal = ProposedPlan {
            id: format!(
                "planning-eval-offline-bank-{}-{}-{source_name}-{delivery_name}",
                scenario.id, repetition
            ),
            revision: 1,
            markdown: scenario.oracle_plan.into(),
            status: ProposedPlanStatus::AwaitingDecision,
            global_reminders: Vec::new(),
            execution_contract: Vec::new(),
        };
        self.insert(PlanBankEntry {
            schema_version: 4,
            proposal_sha256: crate::context::sha256(&proposal.markdown),
            planning_contract: provider_local::planning_prompt_contract_for_eval(profile),
            task_prompt,
            key,
            proposal,
            planner_context,
            project_memory_files,
            planner_usage: UsageReceipt::default(),
            planner_trajectory: TrajectoryReceipt::default(),
            planner_retries: Vec::new(),
            retrieval_treatment: RetrievalTreatmentReceipt {
                knowledge_delivery,
                ..Default::default()
            },
            planner_read_only: true,
            error: None,
        })
    }

    pub async fn generate_live_entry(
        scenario: &Scenario,
        repetition: usize,
        source_set: BankSourceSet,
        knowledge_delivery: KnowledgeDelivery,
        route: &RouteReceipt,
        config: &LiveConfig,
    ) -> Result<PlanBankEntry, String> {
        let sources = match source_set {
            BankSourceSet::None => Vec::new(),
            BankSourceSet::All => vec![
                EvidenceSource::Project,
                EvidenceSource::Org,
                EvidenceSource::Scout,
            ],
        };
        let planner_evidence = crate::context::select_evidence(scenario, &sources);
        let (planner_packet, mut planner_context) = match knowledge_delivery {
            KnowledgeDelivery::PrefetchedCapsule => {
                crate::context::prefetched_planner_packet(&planner_evidence)
            }
            _ => crate::context::context_packet(&planner_evidence),
        };
        let task_prompt = build_planner_prompt(scenario, &planner_packet, knowledge_delivery);
        let fixture = tempfile::tempdir().map_err(|error| error.to_string())?;
        (scenario.seed)(fixture.path())?;
        let fixture_sha256 = tree_digest(fixture.path())?;
        let mut retries = Vec::new();
        let mut final_run = None;
        let mut project_memory_files = BTreeMap::new();
        let mut planner_read_only = true;

        for attempt in 1..=3 {
            let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
            (scenario.seed)(workspace.path())?;
            if knowledge_delivery != KnowledgeDelivery::PrefetchedCapsule {
                crate::context::seed_project_memory(workspace.path(), &planner_evidence)?;
            }
            let before = tree_digest(workspace.path())?;
            let gateway_evidence = if knowledge_delivery == KnowledgeDelivery::PrefetchedCapsule {
                Vec::new()
            } else {
                planner_evidence.clone()
            };
            let gateway = crate::gateway::Gateway::start(
                &config.base_url,
                &config.api_key,
                &gateway_evidence,
            )
            .await?;
            let options = DriveOptions {
                mode: Some(CollaborationMode::Plan),
                writable: true,
                memories: knowledge_delivery != KnowledgeDelivery::PrefetchedCapsule,
                base_url: &gateway.base_url,
                planner_tools: knowledge_delivery != KnowledgeDelivery::PrefetchedCapsule,
                preactivated_tools: preactivated_tools(knowledge_delivery, &sources),
            };
            let mut connected = connect_provider(workspace.path(), &options, config, None).await?;
            let run = drive_connected(&mut connected, &task_prompt, false).await?;
            let mutated = before != tree_digest(workspace.path())?;
            planner_read_only &= !mutated;
            let capacity_retry = run.error.as_deref().filter(|error| {
                retryable_error(error) && run.proposal.is_none() && !mutated && attempt < 3
            });
            if let Some(reason) = capacity_retry {
                let delay = PHASE_DELAYS[attempt - 1];
                let waited = wait_with_progress("plan_bank_planner", delay).await;
                retries.push(phase_retry(
                    "plan_bank_planner",
                    attempt,
                    reason,
                    delay,
                    waited,
                    false,
                    mutated,
                ));
                continue;
            }
            if run.error.is_none() && run.proposal.is_none() && !mutated && attempt < 3 {
                let reason = "model completed without the required hidden proposed_plan artifact";
                eprintln!("plan_bank_planner: protocol retry after attempt {attempt}: {reason}");
                let mut retry = receipt(
                    "plan_bank_planner",
                    attempt,
                    "protocol",
                    reason,
                    std::time::Duration::ZERO,
                    0,
                );
                retry.model_output_observed = true;
                retries.push(retry);
                continue;
            }
            planner_context.retrievals.extend(gateway.receipts());
            project_memory_files = crate::context::snapshot_project_memory(workspace.path())?;
            final_run = Some(run);
            break;
        }

        let run = final_run.ok_or("plan-bank planner exhausted clean retries")?;
        if let Some(error) = run.error.as_deref() {
            return Err(format!("plan-bank planner failed: {error}"));
        }
        if !planner_read_only {
            return Err("plan-bank planner mutated its workspace".into());
        }
        let proposal = run
            .proposal
            .ok_or("plan-bank planner completed without a proposed plan artifact")?;
        let retrieval_treatment = crate::retrieval::retrieval_treatment_for_sources(
            &sources,
            knowledge_delivery,
            &planner_context,
            &run.trajectory,
        );
        let entry = PlanBankEntry {
            schema_version: 4,
            key: PlanBankKey {
                scenario: scenario.id.into(),
                repetition,
                source_set,
                knowledge_delivery,
                planning_profile: config.profile.clone(),
                route_model: route.effective_model.clone(),
                fixture_sha256,
                task_prompt_sha256: crate::context::sha256(&task_prompt),
            },
            proposal_sha256: crate::context::sha256(&proposal.markdown),
            planning_contract: provider_local::planning_prompt_contract_for_eval(&config.profile),
            task_prompt,
            proposal,
            planner_context,
            project_memory_files,
            planner_usage: run.usage,
            planner_trajectory: run.trajectory,
            planner_retries: retries,
            retrieval_treatment,
            planner_read_only,
            error: None,
        };
        entry.validate()?;
        Ok(entry)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn total_planner_tokens(&self) -> u64 {
        self.entries
            .values()
            .map(|entry| entry.planner_usage.input_tokens + entry.planner_usage.output_tokens)
            .sum()
    }

    pub fn total_planner_cost_usd(&self) -> f64 {
        self.entries
            .values()
            .map(|entry| entry.planner_usage.cost_usd)
            .sum()
    }
}

fn knowledge_delivery_name(delivery: KnowledgeDelivery) -> &'static str {
    match delivery {
        KnowledgeDelivery::ForcedPreflight => "forced-preflight",
        KnowledgeDelivery::DeferredDiscovery => "deferred-discovery",
        KnowledgeDelivery::PreactivatedTools => "preactivated-tools",
        KnowledgeDelivery::PrefetchedCapsule => "prefetched-capsule",
    }
}

fn preactivated_tools(delivery: KnowledgeDelivery, sources: &[EvidenceSource]) -> Vec<String> {
    if delivery != KnowledgeDelivery::PreactivatedTools {
        return Vec::new();
    }
    let mut tools = Vec::new();
    if sources.contains(&EvidenceSource::Project) {
        tools.push("memory".to_string());
    }
    if sources.contains(&EvidenceSource::Org) {
        tools.push("organization_knowledge".to_string());
    }
    if sources.contains(&EvidenceSource::Scout) {
        tools.extend([
            "scout_enterprise".to_string(),
            "scout_enterprise_query".to_string(),
        ]);
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(markdown: &str) -> PlanBankEntry {
        PlanBankEntry {
            schema_version: 3,
            key: PlanBankKey {
                scenario: "scenario".into(),
                repetition: 1,
                source_set: BankSourceSet::All,
                knowledge_delivery: KnowledgeDelivery::DeferredDiscovery,
                planning_profile: "concise".into(),
                route_model: "qwen-3.7-flash".into(),
                fixture_sha256: "fixture".into(),
                task_prompt_sha256: crate::context::sha256("task"),
            },
            proposal: ProposedPlan {
                id: "plan-1".into(),
                revision: 1,
                markdown: markdown.into(),
                status: ProposedPlanStatus::AwaitingDecision,
                global_reminders: Vec::new(),
                execution_contract: Vec::new(),
            },
            proposal_sha256: crate::context::sha256(markdown),
            planning_contract: provider_local::planning_prompt_contract_for_eval("concise"),
            task_prompt: "task".into(),
            planner_context: ContextReceipt {
                assigned_evidence_ids: Vec::new(),
                injected_evidence_ids: Vec::new(),
                injected_context: String::new(),
                context_sha256: crate::context::sha256(""),
                retrievals: Vec::new(),
            },
            project_memory_files: BTreeMap::from([(
                ".clark/memory/MEMORY.md".into(),
                "# Project Memory".into(),
            )]),
            planner_usage: UsageReceipt::default(),
            planner_trajectory: TrajectoryReceipt::default(),
            planner_retries: Vec::new(),
            retrieval_treatment: RetrievalTreatmentReceipt {
                knowledge_delivery: KnowledgeDelivery::DeferredDiscovery,
                ..Default::default()
            },
            planner_read_only: true,
            error: None,
        }
    }

    #[test]
    fn append_only_bank_round_trips_and_rejects_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plan-bank.jsonl");
        let mut bank = PlanBank::open(&path).unwrap();
        let first = entry("frozen plan");
        let key = first.key.clone();
        bank.insert(first.clone()).unwrap();
        bank.insert(first).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
        assert_eq!(
            PlanBank::open(&path)
                .unwrap()
                .get(&key)
                .unwrap()
                .proposal
                .markdown,
            "frozen plan"
        );
        assert!(bank.insert(entry("different plan")).is_err());
    }

    #[test]
    fn bank_rejects_hash_or_mutation_receipt_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let mut bad_hash = entry("plan");
        bad_hash.proposal_sha256 = "wrong".into();
        assert!(PlanBank::open(temp.path().join("bank.jsonl"))
            .unwrap()
            .insert(bad_hash)
            .is_err());
        let mut mutated = entry("plan");
        mutated.planner_read_only = false;
        assert!(PlanBank::open(temp.path().join("bank-2.jsonl"))
            .unwrap()
            .insert(mutated)
            .is_err());
    }

    #[test]
    fn delivery_mechanisms_have_distinct_immutable_bank_keys() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plan-bank.jsonl");
        let mut bank = PlanBank::open(&path).unwrap();
        let deferred = entry("same frozen plan");
        let mut preactivated = deferred.clone();
        preactivated.key.knowledge_delivery = KnowledgeDelivery::PreactivatedTools;
        preactivated.retrieval_treatment.knowledge_delivery = KnowledgeDelivery::PreactivatedTools;
        bank.insert(deferred).unwrap();
        bank.insert(preactivated).unwrap();
        assert_eq!(bank.len(), 2);
    }
}
