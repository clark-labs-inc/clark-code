use std::collections::{BTreeMap, BTreeSet};

use super::super::contract::{
    CoverageKey, DiscoveryCharterObservation, DiscoveryPassSealObservation, EnterpriseEdgeId,
    EnterpriseEntityId, EnterpriseEvent, EnterpriseFact, EnterpriseId,
};
use super::model::{EnterpriseConflict, MaterializedCharter, MaterializedDiscoveryPass};

mod pass;
mod qualification;

use pass::{pass_id, project_pass};

#[derive(Clone, Default)]
pub(super) struct PassMembership {
    pub entities: BTreeSet<EnterpriseEntityId>,
    pub edges: BTreeSet<EnterpriseEdgeId>,
    pub entity_scopes:
        BTreeMap<EnterpriseEntityId, BTreeSet<super::super::contract::CoverageCellId>>,
    pub edge_scopes: BTreeMap<EnterpriseEdgeId, BTreeSet<super::super::contract::CoverageCellId>>,
}

pub(super) struct ControlMaterialization {
    pub charter: Option<MaterializedCharter>,
    pub passes: BTreeMap<String, MaterializedDiscoveryPass>,
    pub current_pass_id: Option<String>,
    pub fixed_point: bool,
    pub member_entity_ids: BTreeSet<EnterpriseEntityId>,
    pub member_edge_ids: BTreeSet<EnterpriseEdgeId>,
    pub qualified_topologies: Vec<QualifiedTopology>,
    pub blockers: Vec<String>,
}

#[derive(Clone)]
pub(super) struct QualifiedTopology {
    pub confirming_pass_id: String,
    pub discovery_epoch_sequence: u64,
    pub valid_from_ms: u64,
    pub charter_id: String,
    pub requirement_root: String,
    pub scope_root: String,
    pub topology_root: String,
    pub member_entity_ids: BTreeSet<EnterpriseEntityId>,
    pub member_edge_ids: BTreeSet<EnterpriseEdgeId>,
    pub entity_scopes:
        BTreeMap<EnterpriseEntityId, BTreeSet<super::super::contract::CoverageCellId>>,
    pub edge_scopes: BTreeMap<EnterpriseEdgeId, BTreeSet<super::super::contract::CoverageCellId>>,
}

pub(super) fn materialize(
    enterprise_id: &EnterpriseId,
    events: &[&EnterpriseEvent],
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) -> Result<ControlMaterialization, String> {
    let (charter, charters) = materialize_charters(events, conflicts)?;
    let Some(charter) = charter else {
        return Ok(ControlMaterialization {
            charter: None,
            passes: BTreeMap::new(),
            current_pass_id: None,
            fixed_point: false,
            member_entity_ids: BTreeSet::new(),
            member_edge_ids: BTreeSet::new(),
            qualified_topologies: Vec::new(),
            blockers: vec!["enterprise has no coordinator-issued discovery charter".into()],
        });
    };

    let mut passes = BTreeMap::new();
    let mut memberships = BTreeMap::new();
    for event in events {
        let EnterpriseFact::DiscoveryPassSealed(seal) = &event.fact else {
            continue;
        };
        let Some(pass_charter) = charters.get(&seal.charter_id) else {
            continue;
        };
        let projection = project_pass(enterprise_id, events, pass_charter, seal)?;
        let expected_pass_id = pass_id(enterprise_id, seal, &projection)?;
        let verified = projection.blockers.is_empty()
            && seal.pass_id == expected_pass_id
            && seal.requirement_root == projection.requirement_root
            && seal.scope_root == projection.scope_root
            && seal.topology_root == projection.topology_root;
        if !verified {
            conflicts.insert(EnterpriseConflict::DiscoveryPassInvalid {
                pass_id: seal.pass_id.clone(),
            });
        }
        let record =
            passes
                .entry(seal.pass_id.clone())
                .or_insert_with(|| MaterializedDiscoveryPass {
                    pass_id: seal.pass_id.clone(),
                    charter_id: seal.charter_id.clone(),
                    discovery_epoch: seal.discovery_epoch.clone(),
                    discovery_epoch_sequence: seal.discovery_epoch_sequence,
                    sealed_at_ms: event.provenance.observed_at_ms,
                    previous_pass_id: seal.previous_pass_id.clone(),
                    requirement_root: seal.requirement_root.clone(),
                    scope_root: seal.scope_root.clone(),
                    topology_root: seal.topology_root.clone(),
                    verified,
                    evidence_digests: BTreeSet::new(),
                    supporting_events: BTreeSet::new(),
                });
        if record.discovery_epoch != seal.discovery_epoch
            || record.discovery_epoch_sequence != seal.discovery_epoch_sequence
            || record.previous_pass_id != seal.previous_pass_id
            || record.requirement_root != seal.requirement_root
            || record.scope_root != seal.scope_root
            || record.topology_root != seal.topology_root
        {
            conflicts.insert(EnterpriseConflict::DiscoveryPassInvalid {
                pass_id: seal.pass_id.clone(),
            });
            record.verified = false;
        }
        record
            .evidence_digests
            .extend(seal.evidence_digests.iter().cloned());
        record.sealed_at_ms = record.sealed_at_ms.min(event.provenance.observed_at_ms);
        record.supporting_events.insert(event.event_id.clone());
        if verified {
            memberships.insert(
                seal.pass_id.clone(),
                PassMembership {
                    entities: projection.member_entity_ids,
                    edges: projection.member_edge_ids,
                    entity_scopes: projection.entity_scopes,
                    edge_scopes: projection.edge_scopes,
                },
            );
        }
    }

    let mut verified_by_sequence = BTreeMap::<u64, BTreeSet<String>>::new();
    for pass in passes.values().filter(|pass| pass.verified) {
        verified_by_sequence
            .entry(pass.discovery_epoch_sequence)
            .or_default()
            .insert(pass.pass_id.clone());
    }
    for (discovery_epoch_sequence, pass_ids) in &verified_by_sequence {
        if pass_ids.len() > 1 {
            conflicts.insert(EnterpriseConflict::DiscoveryPassFork {
                discovery_epoch_sequence: *discovery_epoch_sequence,
                pass_ids: pass_ids.clone(),
            });
        }
    }
    let selection = qualification::select(
        &passes,
        &memberships,
        &verified_by_sequence,
        latest_attempt_sequence(events),
        conflicts,
    );

    Ok(ControlMaterialization {
        charter: Some(charter),
        passes,
        current_pass_id: selection.current_pass_id,
        fixed_point: selection.fixed_point,
        member_entity_ids: selection.member_entity_ids,
        member_edge_ids: selection.member_edge_ids,
        qualified_topologies: selection.qualified_topologies,
        blockers: selection.blockers,
    })
}

pub(super) fn draft_seal(
    enterprise_id: &EnterpriseId,
    events: &[&EnterpriseEvent],
    charter_id: &str,
    discovery_epoch: &str,
    discovery_epoch_sequence: u64,
    previous_pass_id: Option<String>,
    evidence_digests: BTreeSet<String>,
) -> Result<DiscoveryPassSealObservation, String> {
    let mut conflicts = BTreeSet::new();
    let charter = materialize_charter(events, &mut conflicts)?
        .ok_or_else(|| "cannot seal a discovery pass without a charter".to_string())?;
    if charter.charter_id != charter_id {
        return Err("discovery pass charter is not the current enterprise charter".into());
    }
    if !conflicts.is_empty() {
        return Err("cannot seal a discovery pass while the charter is conflicted".into());
    }
    let placeholder = DiscoveryPassSealObservation {
        pass_id: format!("pass:{}", "0".repeat(64)),
        charter_id: charter_id.into(),
        discovery_epoch: discovery_epoch.into(),
        discovery_epoch_sequence,
        previous_pass_id,
        requirement_root: "0".repeat(64),
        scope_root: "0".repeat(64),
        topology_root: "0".repeat(64),
        evidence_digests,
    };
    let projection = project_pass(enterprise_id, events, &charter, &placeholder)?;
    if !projection.blockers.is_empty() {
        return Err(format!(
            "cannot seal incomplete discovery pass: {}",
            projection.blockers.join("; ")
        ));
    }
    let mut seal = placeholder;
    seal.requirement_root = projection.requirement_root.clone();
    seal.scope_root = projection.scope_root.clone();
    seal.topology_root = projection.topology_root.clone();
    seal.pass_id = pass_id(enterprise_id, &seal, &projection)?;
    seal.validate()?;
    Ok(seal)
}

fn materialize_charter(
    events: &[&EnterpriseEvent],
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) -> Result<Option<MaterializedCharter>, String> {
    Ok(materialize_charters(events, conflicts)?.0)
}

fn materialize_charters(
    events: &[&EnterpriseEvent],
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) -> Result<
    (
        Option<MaterializedCharter>,
        BTreeMap<String, MaterializedCharter>,
    ),
    String,
> {
    let observations = events
        .iter()
        .filter(|event| matches!(event.fact, EnterpriseFact::DiscoveryCharterObserved(_)))
        .copied()
        .collect::<Vec<_>>();
    validate_charter_lineage(&observations, conflicts);
    let Some(current_epoch) = observations
        .iter()
        .map(|event| event.provenance.discovery_epoch_sequence)
        .max()
    else {
        return Ok((None, BTreeMap::new()));
    };
    let mut observations_by_id = BTreeMap::<String, Vec<&EnterpriseEvent>>::new();
    for event in &observations {
        let EnterpriseFact::DiscoveryCharterObserved(value) = &event.fact else {
            continue;
        };
        observations_by_id
            .entry(value.charter_id.clone())
            .or_default()
            .push(*event);
    }
    let mut charters = BTreeMap::new();
    for (charter_id, observations) in observations_by_id {
        let charter = materialize_charter_observations(&observations, conflicts)?;
        charters.insert(charter_id, charter);
    }
    let current_events = observations
        .iter()
        .filter(|event| event.provenance.discovery_epoch_sequence == current_epoch)
        .collect::<Vec<_>>();
    let current_ids = current_events
        .iter()
        .filter_map(|event| match &event.fact {
            EnterpriseFact::DiscoveryCharterObserved(value) => Some(value.charter_id.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if current_ids.len() != 1 {
        conflicts.insert(EnterpriseConflict::CharterDisagreement {
            event_ids: current_events
                .iter()
                .map(|event| event.event_id.clone())
                .collect(),
        });
    }
    let current = current_ids
        .iter()
        .next()
        .and_then(|charter_id| charters.get(charter_id))
        .cloned();
    Ok((current, charters))
}

fn materialize_charter_observations(
    observations: &[&EnterpriseEvent],
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) -> Result<MaterializedCharter, String> {
    let epoch = observations
        .iter()
        .map(|event| event.provenance.discovery_epoch_sequence)
        .max()
        .ok_or_else(|| "cannot materialize an empty enterprise charter group".to_string())?;
    let latest = observations
        .iter()
        .copied()
        .filter(|event| event.provenance.discovery_epoch_sequence == epoch)
        .collect::<Vec<_>>();
    let semantic = latest
        .iter()
        .filter_map(|event| match &event.fact {
            EnterpriseFact::DiscoveryCharterObserved(value) => Some(charter_semantic_key(value)),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let conflicted = semantic.len() != 1;
    let event_ids = latest
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<BTreeSet<_>>();
    if conflicted {
        conflicts.insert(EnterpriseConflict::CharterDisagreement {
            event_ids: event_ids.clone(),
        });
    }
    let first = latest
        .first()
        .and_then(|event| match &event.fact {
            EnterpriseFact::DiscoveryCharterObserved(value) => Some(value),
            _ => None,
        })
        .ok_or_else(|| "enterprise charter group contains no charter observation".to_string())?;
    let mut evidence_digests = BTreeSet::new();
    for event in latest {
        if let EnterpriseFact::DiscoveryCharterObserved(value) = &event.fact {
            evidence_digests.extend(value.evidence_digests.iter().cloned());
        }
    }
    Ok(MaterializedCharter {
        charter_id: first.charter_id.clone(),
        revision: first.revision,
        max_age_ms: first.max_age_ms,
        supersedes: first.supersedes.clone(),
        discovery_epoch_sequence: epoch,
        required_coverage: first.required_coverage.clone(),
        critical_journey_ids: first.critical_journey_ids.clone(),
        critical_runtime_ids: first.critical_runtime_ids.clone(),
        evidence_digests,
        supporting_events: event_ids,
        conflicted,
    })
}

fn validate_charter_lineage(
    observations: &[&EnterpriseEvent],
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) {
    let mut semantics_by_id = BTreeMap::new();
    let mut ids_by_revision = BTreeMap::<u64, BTreeSet<String>>::new();
    let mut event_ids_by_revision = BTreeMap::<u64, BTreeSet<_>>::new();
    for event in observations {
        let EnterpriseFact::DiscoveryCharterObserved(value) = &event.fact else {
            continue;
        };
        semantics_by_id
            .entry(value.charter_id.clone())
            .or_insert_with(BTreeSet::new)
            .insert(charter_semantic_key(value));
        ids_by_revision
            .entry(value.revision)
            .or_default()
            .insert(value.charter_id.clone());
        event_ids_by_revision
            .entry(value.revision)
            .or_default()
            .insert(event.event_id.clone());
    }
    for (charter_id, semantics) in semantics_by_id {
        if semantics.len() > 1 {
            let event_ids = observations
                .iter()
                .filter_map(|event| match &event.fact {
                    EnterpriseFact::DiscoveryCharterObserved(value)
                        if value.charter_id == charter_id =>
                    {
                        Some(event.event_id.clone())
                    }
                    _ => None,
                })
                .collect();
            conflicts.insert(EnterpriseConflict::CharterDisagreement { event_ids });
        }
    }
    for (revision, charter_ids) in &ids_by_revision {
        if charter_ids.len() > 1 {
            conflicts.insert(EnterpriseConflict::CharterDisagreement {
                event_ids: event_ids_by_revision
                    .get(revision)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
    }
    for event in observations {
        let EnterpriseFact::DiscoveryCharterObserved(value) = &event.fact else {
            continue;
        };
        if value.revision <= 1 {
            continue;
        }
        let valid_predecessor = ids_by_revision
            .get(&(value.revision - 1))
            .is_some_and(|ids| ids.len() == 1 && value.supersedes.as_ref() == ids.iter().next());
        if !valid_predecessor {
            conflicts.insert(EnterpriseConflict::CharterDisagreement {
                event_ids: BTreeSet::from([event.event_id.clone()]),
            });
        }
    }
}

#[allow(clippy::type_complexity)]
fn charter_semantic_key(
    value: &DiscoveryCharterObservation,
) -> (
    String,
    u64,
    u64,
    Option<String>,
    BTreeSet<CoverageKey>,
    BTreeSet<EnterpriseEntityId>,
    BTreeSet<EnterpriseEntityId>,
) {
    (
        value.charter_id.clone(),
        value.revision,
        value.max_age_ms,
        value.supersedes.clone(),
        value.required_coverage.clone(),
        value.critical_journey_ids.clone(),
        value.critical_runtime_ids.clone(),
    )
}

fn latest_attempt_sequence(events: &[&EnterpriseEvent]) -> u64 {
    events
        .iter()
        .map(|event| event.provenance.discovery_epoch_sequence)
        .max()
        .unwrap_or_default()
}
