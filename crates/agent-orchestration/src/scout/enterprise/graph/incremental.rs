use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::materialize;
use super::model::{
    EnterpriseMergeReport, EnterpriseSnapshot, MaterializedCoverage, MaterializedEdge,
    MaterializedEntity, MaterializedFrontier, MaterializedSimulationContract,
};
use super::EnterpriseGraph;
use crate::scout::enterprise::contract::{
    CoverageCellId, EnterpriseBatch, EnterpriseEdgeId, EnterpriseEntityId, EnterpriseEvent,
    EnterpriseEventId, EnterpriseFact, EnterpriseId, FrontierTaskId,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ProjectionIndex {
    entities: BTreeMap<EnterpriseEntityId, BTreeSet<EnterpriseEventId>>,
    edges: BTreeMap<EnterpriseEdgeId, BTreeSet<EnterpriseEventId>>,
    coverage: BTreeMap<CoverageCellId, BTreeSet<EnterpriseEventId>>,
    frontier: BTreeMap<FrontierTaskId, BTreeSet<EnterpriseEventId>>,
    simulation: BTreeMap<EnterpriseEntityId, BTreeSet<EnterpriseEventId>>,
    retracted: BTreeSet<EnterpriseEventId>,
    max_seal_epoch_sequence: Option<u64>,
}

impl ProjectionIndex {
    pub(super) fn insert(&mut self, event: &EnterpriseEvent) {
        let event_id = event.event_id.clone();
        match &event.fact {
            EnterpriseFact::EntityObserved(value) => {
                self.entities
                    .entry(value.entity_id.clone())
                    .or_default()
                    .insert(event_id);
            }
            EnterpriseFact::EdgeObserved(value) => {
                self.edges
                    .entry(value.edge_id.clone())
                    .or_default()
                    .insert(event_id);
            }
            EnterpriseFact::CoverageObserved(value) => {
                self.coverage
                    .entry(value.cell_id.clone())
                    .or_default()
                    .insert(event_id);
            }
            EnterpriseFact::FrontierObserved(value) => {
                self.frontier
                    .entry(value.task_id.clone())
                    .or_default()
                    .insert(event_id);
            }
            EnterpriseFact::SimulationContractObserved(value) => {
                self.simulation
                    .entry(value.runtime_id.clone())
                    .or_default()
                    .insert(event_id);
            }
            EnterpriseFact::DiscoveryPassSealed(value) => {
                self.max_seal_epoch_sequence = Some(
                    self.max_seal_epoch_sequence
                        .map_or(value.discovery_epoch_sequence, |current| {
                            current.max(value.discovery_epoch_sequence)
                        }),
                );
            }
            EnterpriseFact::ObservationRetracted {
                target_event_id, ..
            } => {
                self.retracted.insert(target_event_id.clone());
            }
            EnterpriseFact::DiscoveryCharterObserved(_) => {}
        }
    }

    fn active_events<'a>(
        &'a self,
        graph: &'a EnterpriseGraph,
        event_ids: Option<&BTreeSet<EnterpriseEventId>>,
    ) -> Vec<&'a EnterpriseEvent> {
        event_ids
            .into_iter()
            .flatten()
            .filter(|event_id| !self.retracted.contains(*event_id))
            .filter_map(|event_id| graph.events.get(event_id))
            .collect()
    }

    pub(super) fn max_seal_epoch_sequence(&self) -> Option<u64> {
        self.max_seal_epoch_sequence
    }
}

/// Cursor binding an affected-key projection stream to one graph revision.
///
/// Callers keep one cursor per mutable graph. Count checks reject a stale
/// cursor before a batch is inserted, while Rust's exclusive graph borrow
/// prevents two reducers from mutating the same graph concurrently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnterpriseProjectionCursor {
    enterprise_id: EnterpriseId,
    event_count: usize,
    batch_count: usize,
    current_pass_id: Option<String>,
    max_seal_epoch_sequence: Option<u64>,
}

impl EnterpriseProjectionCursor {
    pub(super) fn from_graph(graph: &EnterpriseGraph) -> Result<Self, String> {
        let snapshot = graph.snapshot()?;
        Ok(Self::from_snapshot(graph, &snapshot))
    }

    pub(super) fn from_snapshot(graph: &EnterpriseGraph, snapshot: &EnterpriseSnapshot) -> Self {
        Self {
            enterprise_id: graph.enterprise_id.clone(),
            event_count: graph.event_count(),
            batch_count: graph.batch_count(),
            current_pass_id: snapshot.current_pass_id.clone(),
            max_seal_epoch_sequence: graph.max_seal_epoch_sequence(),
        }
    }

    pub fn event_count(&self) -> usize {
        self.event_count
    }

    pub fn batch_count(&self) -> usize {
        self.batch_count
    }

    pub fn current_pass_id(&self) -> Option<&str> {
        self.current_pass_id.as_deref()
    }

    fn validate(&self, graph: &EnterpriseGraph) -> Result<(), String> {
        if self.enterprise_id != graph.enterprise_id {
            return Err("enterprise projection cursor belongs to another enterprise".into());
        }
        if self.event_count != graph.event_count() || self.batch_count != graph.batch_count() {
            return Err("enterprise projection cursor is stale".into());
        }
        Ok(())
    }
}

/// Bounded work performed by one affected-key projection update.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseProjectionWork {
    pub inserted_events: usize,
    pub candidate_events_examined: usize,
    pub records_rebuilt: usize,
    pub full_rebuild: bool,
}

/// Exact materialization of a bounded event slice.
///
/// Persistent consumers use this after selecting the active history for a
/// small set of projection keys. Control events and retractions must be
/// handled by a full graph rebuild before calling this helper.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseProjectionSlice {
    pub entities: BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    pub edges: BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    pub coverage: BTreeMap<CoverageCellId, MaterializedCoverage>,
    pub frontier: BTreeMap<FrontierTaskId, MaterializedFrontier>,
    pub simulation_contracts: BTreeMap<EnterpriseEntityId, MaterializedSimulationContract>,
    pub conflicts: BTreeSet<super::model::EnterpriseConflict>,
}

pub fn project_event_slice(
    events: &[EnterpriseEvent],
    include_topology: bool,
) -> EnterpriseProjectionSlice {
    let event_refs = events.iter().collect::<Vec<_>>();
    let topology_events = if include_topology {
        event_refs.as_slice()
    } else {
        &[]
    };
    let mut conflicts = BTreeSet::new();
    let entities = materialize::materialize_entities(topology_events);
    let mut edges = materialize::materialize_edges(topology_events);
    materialize::apply_endpoint_classification(&entities, &mut edges);
    EnterpriseProjectionSlice {
        entities,
        edges,
        coverage: materialize::observations::coverage(&event_refs, &mut conflicts),
        frontier: materialize::observations::frontier(&event_refs, &mut conflicts),
        simulation_contracts: materialize::observations::simulation(
            topology_events,
            &mut conflicts,
        ),
        conflicts,
    }
}

/// Row-level changes produced by a batch.
///
/// `None` means the persisted row must be deleted. A control-plane change or
/// retraction returns `replacement_snapshot`; those rare transitions can
/// change authoritative membership globally and therefore cannot be reduced
/// safely to the keys named directly by the batch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnterpriseAffectedProjection {
    pub merge: EnterpriseMergeReport,
    /// True when event-root, digest, counts, conflicts, or completion metadata changed.
    ///
    /// A row-level consumer must refresh its metadata even when no materialized
    /// entity or edge changed (for example, an unsealed post-pass observation).
    pub global_metadata_changed: bool,
    pub entities: BTreeMap<EnterpriseEntityId, Option<MaterializedEntity>>,
    pub edges: BTreeMap<EnterpriseEdgeId, Option<MaterializedEdge>>,
    pub coverage: BTreeMap<CoverageCellId, Option<MaterializedCoverage>>,
    pub frontier: BTreeMap<FrontierTaskId, Option<MaterializedFrontier>>,
    pub simulation_contracts: BTreeMap<EnterpriseEntityId, Option<MaterializedSimulationContract>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_snapshot: Option<EnterpriseSnapshot>,
    pub work: EnterpriseProjectionWork,
}

impl EnterpriseAffectedProjection {
    pub fn requires_full_rebuild(&self) -> bool {
        self.replacement_snapshot.is_some()
    }

    pub fn affected_row_count(&self) -> usize {
        self.entities.len()
            + self.edges.len()
            + self.coverage.len()
            + self.frontier.len()
            + self.simulation_contracts.len()
    }
}

#[derive(Default)]
struct AffectedKeys {
    entities: BTreeSet<EnterpriseEntityId>,
    edges: BTreeSet<EnterpriseEdgeId>,
    coverage: BTreeSet<CoverageCellId>,
    frontier: BTreeSet<FrontierTaskId>,
    simulation: BTreeSet<EnterpriseEntityId>,
}

pub(super) fn apply_batch(
    graph: &mut EnterpriseGraph,
    cursor: &mut EnterpriseProjectionCursor,
    batch: EnterpriseBatch,
) -> Result<EnterpriseAffectedProjection, String> {
    cursor.validate(graph)?;
    let (merge, inserted_event_ids) = graph.insert_batch(batch)?;
    if inserted_event_ids.is_empty() {
        cursor.event_count = graph.event_count();
        cursor.batch_count = graph.batch_count();
        return Ok(empty_update(merge));
    }

    let inserted_events = inserted_event_ids
        .iter()
        .filter_map(|event_id| graph.events.get(event_id))
        .collect::<Vec<_>>();
    let crosses_control_barrier = cursor.max_seal_epoch_sequence.is_some_and(|barrier| {
        inserted_events
            .iter()
            .any(|event| event.provenance.discovery_epoch_sequence <= barrier)
    });
    let changes_control = inserted_events.iter().any(|event| {
        matches!(
            event.fact,
            EnterpriseFact::DiscoveryCharterObserved(_)
                | EnterpriseFact::DiscoveryPassSealed(_)
                | EnterpriseFact::ObservationRetracted { .. }
        )
    });
    if changes_control || crosses_control_barrier {
        let snapshot = graph.snapshot()?;
        let rebuilt_rows = snapshot.entities.len()
            + snapshot.edges.len()
            + snapshot.coverage.len()
            + snapshot.frontier.len()
            + snapshot.simulation_contracts.len();
        *cursor = EnterpriseProjectionCursor::from_snapshot(graph, &snapshot);
        return Ok(EnterpriseAffectedProjection {
            merge,
            global_metadata_changed: true,
            entities: BTreeMap::new(),
            edges: BTreeMap::new(),
            coverage: BTreeMap::new(),
            frontier: BTreeMap::new(),
            simulation_contracts: BTreeMap::new(),
            replacement_snapshot: Some(snapshot),
            work: EnterpriseProjectionWork {
                inserted_events: inserted_event_ids.len(),
                candidate_events_examined: graph.event_count(),
                records_rebuilt: rebuilt_rows,
                full_rebuild: true,
            },
        });
    }

    let include_unsealed_topology = cursor.current_pass_id.is_none();
    let keys = affected_keys(&inserted_events, include_unsealed_topology);
    let mut conflicts = BTreeSet::new();
    let mut work = EnterpriseProjectionWork {
        inserted_events: inserted_event_ids.len(),
        ..EnterpriseProjectionWork::default()
    };
    let entities = project_entities(graph, &keys.entities, &mut work);
    let edges = project_edges(graph, &keys.edges, &mut work);
    let coverage = project_coverage(graph, &keys.coverage, &mut conflicts, &mut work);
    let frontier = project_frontier(graph, &keys.frontier, &mut conflicts, &mut work);
    let simulation_contracts =
        project_simulation(graph, &keys.simulation, &mut conflicts, &mut work);
    cursor.event_count = graph.event_count();
    cursor.batch_count = graph.batch_count();

    Ok(EnterpriseAffectedProjection {
        merge,
        global_metadata_changed: true,
        entities,
        edges,
        coverage,
        frontier,
        simulation_contracts,
        replacement_snapshot: None,
        work,
    })
}

fn empty_update(merge: EnterpriseMergeReport) -> EnterpriseAffectedProjection {
    EnterpriseAffectedProjection {
        merge,
        global_metadata_changed: false,
        entities: BTreeMap::new(),
        edges: BTreeMap::new(),
        coverage: BTreeMap::new(),
        frontier: BTreeMap::new(),
        simulation_contracts: BTreeMap::new(),
        replacement_snapshot: None,
        work: EnterpriseProjectionWork::default(),
    }
}

fn affected_keys(events: &[&EnterpriseEvent], include_topology: bool) -> AffectedKeys {
    let mut keys = AffectedKeys::default();
    for event in events {
        match &event.fact {
            EnterpriseFact::EntityObserved(value) if include_topology => {
                keys.entities.insert(value.entity_id.clone());
            }
            EnterpriseFact::EdgeObserved(value) if include_topology => {
                keys.edges.insert(value.edge_id.clone());
            }
            EnterpriseFact::CoverageObserved(value) => {
                keys.coverage.insert(value.cell_id.clone());
            }
            EnterpriseFact::FrontierObserved(value) => {
                keys.frontier.insert(value.task_id.clone());
            }
            EnterpriseFact::SimulationContractObserved(value) if include_topology => {
                keys.simulation.insert(value.runtime_id.clone());
            }
            _ => {}
        }
    }
    keys
}

fn project_entities(
    graph: &EnterpriseGraph,
    keys: &BTreeSet<EnterpriseEntityId>,
    work: &mut EnterpriseProjectionWork,
) -> BTreeMap<EnterpriseEntityId, Option<MaterializedEntity>> {
    keys.iter()
        .map(|key| {
            let index = graph.projection_index();
            let events = index.active_events(graph, index.entities.get(key));
            record_work(work, events.len());
            let value = materialize::materialize_entities(&events).remove(key);
            (key.clone(), value)
        })
        .collect()
}

fn project_edges(
    graph: &EnterpriseGraph,
    keys: &BTreeSet<EnterpriseEdgeId>,
    work: &mut EnterpriseProjectionWork,
) -> BTreeMap<EnterpriseEdgeId, Option<MaterializedEdge>> {
    keys.iter()
        .map(|key| {
            let index = graph.projection_index();
            let events = index.active_events(graph, index.edges.get(key));
            record_work(work, events.len());
            let value = materialize::materialize_edges(&events).remove(key);
            (key.clone(), value)
        })
        .collect()
}

fn project_coverage(
    graph: &EnterpriseGraph,
    keys: &BTreeSet<CoverageCellId>,
    conflicts: &mut BTreeSet<super::model::EnterpriseConflict>,
    work: &mut EnterpriseProjectionWork,
) -> BTreeMap<CoverageCellId, Option<MaterializedCoverage>> {
    keys.iter()
        .map(|key| {
            let index = graph.projection_index();
            let events = index.active_events(graph, index.coverage.get(key));
            record_work(work, events.len());
            let value = materialize::observations::coverage(&events, conflicts).remove(key);
            (key.clone(), value)
        })
        .collect()
}

fn project_frontier(
    graph: &EnterpriseGraph,
    keys: &BTreeSet<FrontierTaskId>,
    conflicts: &mut BTreeSet<super::model::EnterpriseConflict>,
    work: &mut EnterpriseProjectionWork,
) -> BTreeMap<FrontierTaskId, Option<MaterializedFrontier>> {
    keys.iter()
        .map(|key| {
            let index = graph.projection_index();
            let events = index.active_events(graph, index.frontier.get(key));
            record_work(work, events.len());
            let value = materialize::observations::frontier(&events, conflicts).remove(key);
            (key.clone(), value)
        })
        .collect()
}

fn project_simulation(
    graph: &EnterpriseGraph,
    keys: &BTreeSet<EnterpriseEntityId>,
    conflicts: &mut BTreeSet<super::model::EnterpriseConflict>,
    work: &mut EnterpriseProjectionWork,
) -> BTreeMap<EnterpriseEntityId, Option<MaterializedSimulationContract>> {
    keys.iter()
        .map(|key| {
            let index = graph.projection_index();
            let events = index.active_events(graph, index.simulation.get(key));
            record_work(work, events.len());
            let value = materialize::observations::simulation(&events, conflicts).remove(key);
            (key.clone(), value)
        })
        .collect()
}

fn record_work(work: &mut EnterpriseProjectionWork, candidates: usize) {
    work.candidate_events_examined += candidates;
    work.records_rebuilt += 1;
}
