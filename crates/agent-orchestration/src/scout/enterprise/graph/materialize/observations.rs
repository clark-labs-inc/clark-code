use std::collections::{BTreeMap, BTreeSet};

use super::super::super::contract::{
    CoverageCellId, EnterpriseEntityId, EnterpriseEvent, EnterpriseFact, FrontierTaskId,
    SimulationContractObservation,
};
use super::super::model::{
    EnterpriseConflict, MaterializedCoverage, MaterializedFrontier, MaterializedSimulationContract,
};

pub(in crate::scout::enterprise::graph) fn coverage(
    events: &[&EnterpriseEvent],
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) -> BTreeMap<CoverageCellId, MaterializedCoverage> {
    let mut grouped = BTreeMap::<CoverageCellId, Vec<&EnterpriseEvent>>::new();
    for event in events {
        if let EnterpriseFact::CoverageObserved(value) = &event.fact {
            grouped
                .entry(value.cell_id.clone())
                .or_default()
                .push(event);
        }
    }
    grouped
        .into_iter()
        .map(|(cell_id, observations)| {
            let epoch = observations
                .iter()
                .map(|event| event.provenance.discovery_epoch_sequence)
                .max()
                .unwrap_or_default();
            let latest = observations
                .into_iter()
                .filter(|event| event.provenance.discovery_epoch_sequence == epoch)
                .collect::<Vec<_>>();
            let values = latest
                .iter()
                .filter_map(|event| match &event.fact {
                    EnterpriseFact::CoverageObserved(value) => Some((
                        value.status,
                        value.next_cursor.clone(),
                        value.enumerated_count,
                        value.enumerated_edge_count,
                    )),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            let conflicted = values.len() != 1;
            let event_ids = latest
                .iter()
                .map(|event| event.event_id.clone())
                .collect::<BTreeSet<_>>();
            if conflicted {
                conflicts.insert(EnterpriseConflict::CoverageDisagreement {
                    cell_id: cell_id.clone(),
                    event_ids: event_ids.clone(),
                });
            }
            let first = latest
                .first()
                .and_then(|event| match &event.fact {
                    EnterpriseFact::CoverageObserved(value) => Some(value),
                    _ => None,
                })
                .expect("coverage group contains coverage observations");
            let mut evidence_digests = BTreeSet::new();
            for event in &latest {
                if let EnterpriseFact::CoverageObserved(value) = &event.fact {
                    evidence_digests.extend(value.evidence_digests.iter().cloned());
                }
            }
            let value = (!conflicted).then(|| values.into_iter().next().unwrap());
            (
                cell_id.clone(),
                MaterializedCoverage {
                    cell_id,
                    key: first.key.clone(),
                    discovery_epoch_sequence: epoch,
                    status: value.as_ref().map(|item| item.0),
                    next_cursor: value.as_ref().and_then(|item| item.1.clone()),
                    enumerated_count: value.as_ref().map(|item| item.2),
                    enumerated_edge_count: value.as_ref().map(|item| item.3),
                    evidence_digests,
                    supporting_events: event_ids,
                    conflicted,
                },
            )
        })
        .collect()
}

pub(in crate::scout::enterprise::graph) fn frontier(
    events: &[&EnterpriseEvent],
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) -> BTreeMap<FrontierTaskId, MaterializedFrontier> {
    let mut grouped = BTreeMap::<FrontierTaskId, Vec<&EnterpriseEvent>>::new();
    for event in events {
        if let EnterpriseFact::FrontierObserved(value) = &event.fact {
            grouped
                .entry(value.task_id.clone())
                .or_default()
                .push(event);
        }
    }
    grouped
        .into_iter()
        .map(|(task_id, observations)| {
            let epoch = observations
                .iter()
                .map(|event| event.provenance.discovery_epoch_sequence)
                .max()
                .unwrap_or_default();
            let latest = observations
                .into_iter()
                .filter(|event| event.provenance.discovery_epoch_sequence == epoch)
                .collect::<Vec<_>>();
            let transition_sequence = latest
                .iter()
                .filter_map(|event| match &event.fact {
                    EnterpriseFact::FrontierObserved(value) => Some(value.transition_sequence),
                    _ => None,
                })
                .max()
                .unwrap_or_default();
            let latest = latest
                .into_iter()
                .filter(|event| {
                    matches!(
                        &event.fact,
                        EnterpriseFact::FrontierObserved(value)
                            if value.transition_sequence == transition_sequence
                    )
                })
                .collect::<Vec<_>>();
            let semantics = latest
                .iter()
                .filter_map(|event| match &event.fact {
                    EnterpriseFact::FrontierObserved(value) => Some((
                        value.state.clone(),
                        value.parent_task_id.clone(),
                        value.discovered_entity_ids.clone(),
                        value.discovered_edge_ids.clone(),
                    )),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            let conflicted = semantics.len() != 1;
            let event_ids = latest
                .iter()
                .map(|event| event.event_id.clone())
                .collect::<BTreeSet<_>>();
            if conflicted {
                conflicts.insert(EnterpriseConflict::FrontierDisagreement {
                    task_id: task_id.clone(),
                    event_ids: event_ids.clone(),
                });
            }
            let first = latest
                .first()
                .and_then(|event| match &event.fact {
                    EnterpriseFact::FrontierObserved(value) => Some(value),
                    _ => None,
                })
                .expect("frontier group contains frontier observations");
            let state = (!conflicted).then(|| {
                semantics
                    .into_iter()
                    .next()
                    .expect("one frontier semantic")
                    .0
            });
            let mut discovered_entity_ids = BTreeSet::new();
            let mut discovered_edge_ids = BTreeSet::new();
            let mut evidence_digests = BTreeSet::new();
            for event in &latest {
                if let EnterpriseFact::FrontierObserved(value) = &event.fact {
                    discovered_entity_ids.extend(value.discovered_entity_ids.iter().cloned());
                    discovered_edge_ids.extend(value.discovered_edge_ids.iter().cloned());
                    evidence_digests.extend(value.evidence_digests.iter().cloned());
                }
            }
            (
                task_id.clone(),
                MaterializedFrontier {
                    task_id,
                    key: first.key.clone(),
                    discovery_epoch_sequence: epoch,
                    transition_sequence,
                    state,
                    discovered_entity_ids,
                    discovered_edge_ids,
                    evidence_digests,
                    supporting_events: event_ids,
                    conflicted,
                },
            )
        })
        .collect()
}

pub(in crate::scout::enterprise::graph) fn simulation(
    events: &[&EnterpriseEvent],
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) -> BTreeMap<EnterpriseEntityId, MaterializedSimulationContract> {
    let mut grouped = BTreeMap::<EnterpriseEntityId, Vec<&EnterpriseEvent>>::new();
    for event in events {
        if let EnterpriseFact::SimulationContractObserved(value) = &event.fact {
            grouped
                .entry(value.runtime_id.clone())
                .or_default()
                .push(event);
        }
    }
    grouped
        .into_iter()
        .map(|(runtime_id, observations)| {
            let epoch = observations
                .iter()
                .map(|event| event.provenance.discovery_epoch_sequence)
                .max()
                .unwrap_or_default();
            let latest = observations
                .into_iter()
                .filter(|event| event.provenance.discovery_epoch_sequence == epoch)
                .collect::<Vec<_>>();
            let values = latest
                .iter()
                .filter_map(|event| match &event.fact {
                    EnterpriseFact::SimulationContractObserved(value) => {
                        Some(simulation_key(value))
                    }
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            let conflicted = values.len() != 1;
            let event_ids = latest
                .iter()
                .map(|event| event.event_id.clone())
                .collect::<BTreeSet<_>>();
            if conflicted {
                conflicts.insert(EnterpriseConflict::SimulationContractDisagreement {
                    runtime_id: runtime_id.clone(),
                    event_ids: event_ids.clone(),
                });
            }
            let mut evidence_digests = BTreeSet::new();
            for event in &latest {
                if let EnterpriseFact::SimulationContractObserved(value) = &event.fact {
                    evidence_digests.extend(value.evidence_digests.iter().cloned());
                }
            }
            let complete = !conflicted
                && latest.iter().any(|event| {
                    matches!(
                        &event.fact,
                        EnterpriseFact::SimulationContractObserved(value) if value.is_complete()
                    )
                });
            (
                runtime_id.clone(),
                MaterializedSimulationContract {
                    runtime_id,
                    discovery_epoch_sequence: epoch,
                    complete,
                    evidence_digests,
                    supporting_events: event_ids,
                    conflicted,
                },
            )
        })
        .collect()
}

fn simulation_key(value: &SimulationContractObservation) -> [bool; 9] {
    [
        value.inputs,
        value.outputs,
        value.state_effects,
        value.timeouts,
        value.retries,
        value.idempotency,
        value.failure_behavior,
        value.observability,
        value.recovery,
    ]
}
