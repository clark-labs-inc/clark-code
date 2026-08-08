use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::super::contract::{
    AuthorityRef, CoverageCellId, CoverageKey, CoverageStatus, EnterpriseBatchId,
    EnterpriseClassification, EnterpriseEdgeId, EnterpriseEdgeKind, EnterpriseEntityId,
    EnterpriseEntityKind, EnterpriseEventId, EnterpriseId, FrontierKey, FrontierState,
    FrontierTaskId,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseMergeReport {
    pub batch_id: EnterpriseBatchId,
    pub received: usize,
    pub inserted: usize,
    pub duplicates: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedEntity {
    pub entity_id: EnterpriseEntityId,
    pub kind: EnterpriseEntityKind,
    pub authority: AuthorityRef,
    pub labels: BTreeSet<String>,
    pub environments: BTreeSet<String>,
    pub critical: bool,
    #[serde(default, skip_serializing_if = "classification_is_default")]
    pub classification: EnterpriseClassification,
    pub discovery_epoch_sequence: u64,
    pub evidence_digests: BTreeSet<String>,
    pub supporting_events: BTreeSet<EnterpriseEventId>,
    pub last_observed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_pass_id: Option<String>,
    #[serde(default, skip_serializing_if = "lifecycle_is_active")]
    pub lifecycle: QualifiedLifecycle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedEdge {
    pub edge_id: EnterpriseEdgeId,
    pub from: EnterpriseEntityId,
    pub to: EnterpriseEntityId,
    pub kind: EnterpriseEdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    #[serde(default, skip_serializing_if = "classification_is_default")]
    pub classification: EnterpriseClassification,
    pub discovery_epoch_sequence: u64,
    pub evidence_digests: BTreeSet<String>,
    pub supporting_events: BTreeSet<EnterpriseEventId>,
    pub last_observed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_pass_id: Option<String>,
    #[serde(default, skip_serializing_if = "lifecycle_is_active")]
    pub lifecycle: QualifiedLifecycle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualifiedLifecycle {
    #[default]
    Active,
    Retired,
    OutOfScope,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedCoverage {
    pub cell_id: CoverageCellId,
    pub key: CoverageKey,
    pub discovery_epoch_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<CoverageStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enumerated_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enumerated_edge_count: Option<u64>,
    pub evidence_digests: BTreeSet<String>,
    pub supporting_events: BTreeSet<EnterpriseEventId>,
    pub conflicted: bool,
}

impl MaterializedCoverage {
    pub fn is_complete(&self) -> bool {
        !self.conflicted
            && self.status.is_some_and(CoverageStatus::is_complete)
            && self.next_cursor.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedFrontier {
    pub task_id: FrontierTaskId,
    pub key: FrontierKey,
    pub discovery_epoch_sequence: u64,
    pub transition_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<FrontierState>,
    pub discovered_entity_ids: BTreeSet<EnterpriseEntityId>,
    pub discovered_edge_ids: BTreeSet<EnterpriseEdgeId>,
    pub evidence_digests: BTreeSet<String>,
    pub supporting_events: BTreeSet<EnterpriseEventId>,
    pub conflicted: bool,
}

impl MaterializedFrontier {
    pub fn is_complete(&self) -> bool {
        !self.conflicted
            && matches!(
                self.state,
                Some(FrontierState::PageComplete { .. })
                    | Some(FrontierState::Terminal {
                        status: CoverageStatus::Supported | CoverageStatus::Empty,
                        ..
                    })
            )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedCharter {
    pub charter_id: String,
    pub revision: u64,
    pub max_age_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    pub discovery_epoch_sequence: u64,
    pub required_coverage: BTreeSet<CoverageKey>,
    pub critical_journey_ids: BTreeSet<EnterpriseEntityId>,
    pub critical_runtime_ids: BTreeSet<EnterpriseEntityId>,
    pub evidence_digests: BTreeSet<String>,
    pub supporting_events: BTreeSet<EnterpriseEventId>,
    pub conflicted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedDiscoveryPass {
    pub pass_id: String,
    pub charter_id: String,
    pub discovery_epoch: String,
    pub discovery_epoch_sequence: u64,
    pub sealed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_pass_id: Option<String>,
    pub requirement_root: String,
    pub scope_root: String,
    pub topology_root: String,
    pub verified: bool,
    pub evidence_digests: BTreeSet<String>,
    pub supporting_events: BTreeSet<EnterpriseEventId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedSimulationContract {
    pub runtime_id: EnterpriseEntityId,
    pub discovery_epoch_sequence: u64,
    pub complete: bool,
    pub evidence_digests: BTreeSet<String>,
    pub supporting_events: BTreeSet<EnterpriseEventId>,
    pub conflicted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnterpriseConflict {
    SourceEquivocation {
        source_position: String,
        event_ids: BTreeSet<EnterpriseEventId>,
    },
    OrphanRetraction {
        retraction_event_id: EnterpriseEventId,
        target_event_id: EnterpriseEventId,
    },
    RetractionOfRetraction {
        retraction_event_id: EnterpriseEventId,
        target_event_id: EnterpriseEventId,
    },
    DanglingEdge {
        edge_id: EnterpriseEdgeId,
        missing_entity_id: EnterpriseEntityId,
    },
    CoverageDisagreement {
        cell_id: CoverageCellId,
        event_ids: BTreeSet<EnterpriseEventId>,
    },
    FrontierDisagreement {
        task_id: FrontierTaskId,
        event_ids: BTreeSet<EnterpriseEventId>,
    },
    SimulationContractDisagreement {
        runtime_id: EnterpriseEntityId,
        event_ids: BTreeSet<EnterpriseEventId>,
    },
    CharterDisagreement {
        event_ids: BTreeSet<EnterpriseEventId>,
    },
    DiscoveryPassInvalid {
        pass_id: String,
    },
    DiscoveryPassFork {
        discovery_epoch_sequence: u64,
        pass_ids: BTreeSet<String>,
    },
    DiscoveryPassNonMonotonic {
        first_pass_id: String,
        confirming_pass_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseSnapshot {
    pub enterprise_id: EnterpriseId,
    pub event_root: String,
    pub graph_digest: String,
    pub event_count: usize,
    pub retracted_event_count: usize,
    pub entities: BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    pub edges: BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entity_history: BTreeMap<EnterpriseEntityId, Vec<MaterializedEntity>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub edge_history: BTreeMap<EnterpriseEdgeId, Vec<MaterializedEdge>>,
    pub coverage: BTreeMap<CoverageCellId, MaterializedCoverage>,
    pub frontier: BTreeMap<FrontierTaskId, MaterializedFrontier>,
    pub simulation_contracts: BTreeMap<EnterpriseEntityId, MaterializedSimulationContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charter: Option<MaterializedCharter>,
    pub discovery_passes: BTreeMap<String, MaterializedDiscoveryPass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_pass_id: Option<String>,
    pub fixed_point: bool,
    pub control_blockers: Vec<String>,
    pub conflicts: BTreeSet<EnterpriseConflict>,
}

fn classification_is_default(value: &EnterpriseClassification) -> bool {
    *value == EnterpriseClassification::Internal
}

fn lifecycle_is_active(value: &QualifiedLifecycle) -> bool {
    *value == QualifiedLifecycle::Active
}

impl EnterpriseSnapshot {
    pub fn completion(&self) -> EnterpriseCompletion {
        let observed_at_ms = self
            .current_pass_id
            .as_ref()
            .and_then(|pass_id| self.discovery_passes.get(pass_id))
            .map_or(0, |pass| pass.sealed_at_ms);
        self.completion_at(observed_at_ms)
    }

    pub fn completion_at(&self, evaluated_at_ms: u64) -> EnterpriseCompletion {
        let mut blockers = self.control_blockers.clone();
        if self.entities.is_empty() {
            blockers.push("enterprise graph contains no entities".into());
        }
        if !self.conflicts.is_empty() {
            blockers.push(format!(
                "enterprise graph contains {} unresolved conflicts",
                self.conflicts.len()
            ));
        }
        if let (Some(charter), Some(pass_id)) = (&self.charter, &self.current_pass_id) {
            if let Some(pass) = self.discovery_passes.get(pass_id) {
                if evaluated_at_ms.saturating_sub(pass.sealed_at_ms) > charter.max_age_ms {
                    blockers.push(format!(
                        "discovery pass {pass_id} is older than the charter freshness policy"
                    ));
                }
            }
        }
        self.add_journey_blockers(&mut blockers);
        self.add_simulation_blockers(&mut blockers);
        EnterpriseCompletion {
            complete: blockers.is_empty(),
            blockers,
        }
    }

    fn add_simulation_blockers(&self, blockers: &mut Vec<String>) {
        let Some(charter) = &self.charter else {
            return;
        };
        let runtime_edges = self
            .edges
            .values()
            .map(|edge| {
                let runtime_id = match edge.kind {
                    EnterpriseEdgeKind::SourceFor => &edge.to,
                    _ => &edge.from,
                };
                (runtime_id.clone(), edge.kind)
            })
            .collect::<BTreeSet<_>>();
        for runtime_id in &charter.critical_runtime_ids {
            let Some(entity) = self.entities.get(runtime_id) else {
                blockers.push(format!("charter critical runtime {runtime_id} is missing"));
                continue;
            };
            if !matches!(
                entity.kind,
                EnterpriseEntityKind::Service
                    | EnterpriseEntityKind::Function
                    | EnterpriseEntityKind::Job
                    | EnterpriseEntityKind::Api
            ) {
                blockers.push(format!(
                    "charter critical runtime {runtime_id} is not a runtime entity"
                ));
            }
            let complete_contract = self
                .simulation_contracts
                .get(&entity.entity_id)
                .is_some_and(|contract| contract.complete && !contract.conflicted);
            if !complete_contract {
                blockers.push(format!(
                    "critical runtime {} lacks a complete simulation contract",
                    entity.entity_id
                ));
            }
            for kind in [
                EnterpriseEdgeKind::SourceFor,
                EnterpriseEdgeKind::DeploysTo,
                EnterpriseEdgeKind::AuthenticatesVia,
                EnterpriseEdgeKind::OwnedBy,
                EnterpriseEdgeKind::MonitoredBy,
            ] {
                if !runtime_edges.contains(&(entity.entity_id.clone(), kind)) {
                    blockers.push(format!(
                        "critical runtime {} lacks a {kind:?} edge",
                        entity.entity_id
                    ));
                }
            }
        }
    }

    fn add_journey_blockers(&self, blockers: &mut Vec<String>) {
        let Some(charter) = &self.charter else {
            return;
        };
        for journey_id in &charter.critical_journey_ids {
            let Some(journey) = self.entities.get(journey_id) else {
                blockers.push(format!("charter critical journey {journey_id} is missing"));
                continue;
            };
            if journey.kind != EnterpriseEntityKind::Journey {
                blockers.push(format!(
                    "charter critical journey {journey_id} is not a journey entity"
                ));
                continue;
            }
            let has_actor = self.edges.values().any(|edge| {
                edge.kind == EnterpriseEdgeKind::EntersThrough
                    && &edge.to == journey_id
                    && self
                        .entities
                        .get(&edge.from)
                        .is_some_and(|entity| entity.kind == EnterpriseEntityKind::Actor)
            });
            if !has_actor {
                blockers.push(format!(
                    "critical journey {journey_id} lacks an Actor -> Journey entry edge"
                ));
            }
            let runtimes = self
                .edges
                .values()
                .filter(|edge| {
                    edge.kind == EnterpriseEdgeKind::Implements && &edge.from == journey_id
                })
                .filter_map(|edge| self.entities.get(&edge.to))
                .filter(|entity| charter.critical_runtime_ids.contains(&entity.entity_id))
                .collect::<Vec<_>>();
            if runtimes.is_empty() {
                blockers.push(format!(
                    "critical journey {journey_id} lacks a pinned runtime implementation"
                ));
                continue;
            }
            let has_effect = runtimes.iter().any(|runtime| {
                self.edges.values().any(|edge| {
                    edge.kind == EnterpriseEdgeKind::Writes
                        && edge.from == runtime.entity_id
                        && self.entities.get(&edge.to).is_some_and(|target| {
                            matches!(
                                target.kind,
                                EnterpriseEntityKind::Database
                                    | EnterpriseEntityKind::Dataset
                                    | EnterpriseEntityKind::Cache
                                    | EnterpriseEntityKind::ObjectStore
                                    | EnterpriseEntityKind::Queue
                                    | EnterpriseEntityKind::Topic
                            )
                        })
                })
            });
            if !has_effect {
                blockers.push(format!(
                    "critical journey {journey_id} lacks a runtime state-effect path"
                ));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseCompletion {
    pub complete: bool,
    pub blockers: Vec<String>,
}
