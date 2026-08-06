use std::collections::BTreeMap;

use serde::Serialize;

use super::contract::canonical_digest;
use super::contract::{
    EnterpriseBatch, EnterpriseBatchId, EnterpriseEvent, EnterpriseEventId, EnterpriseId,
    VerifiedEnterpriseBatch,
};

mod control;
mod incremental;
mod materialize;
mod model;
mod query;
mod temporal;

pub use incremental::{
    project_event_slice, EnterpriseAffectedProjection, EnterpriseProjectionCursor,
    EnterpriseProjectionSlice, EnterpriseProjectionWork,
};
pub use model::{
    EnterpriseCompletion, EnterpriseConflict, EnterpriseMergeReport, EnterpriseSnapshot,
    MaterializedCharter, MaterializedCoverage, MaterializedDiscoveryPass, MaterializedEdge,
    MaterializedEntity, MaterializedFrontier, MaterializedSimulationContract, QualifiedLifecycle,
};
pub use query::EnterpriseQuery;

#[derive(Clone, Debug)]
pub struct EnterpriseGraph {
    enterprise_id: EnterpriseId,
    events: BTreeMap<EnterpriseEventId, EnterpriseEvent>,
    batch_event_counts: BTreeMap<EnterpriseBatchId, usize>,
    projection_index: Option<incremental::ProjectionIndex>,
}

impl PartialEq for EnterpriseGraph {
    fn eq(&self, other: &Self) -> bool {
        self.enterprise_id == other.enterprise_id
            && self.events == other.events
            && self.batch_event_counts == other.batch_event_counts
    }
}

impl Eq for EnterpriseGraph {}

impl EnterpriseGraph {
    pub fn new(enterprise_id: EnterpriseId) -> Self {
        Self {
            enterprise_id,
            events: BTreeMap::new(),
            batch_event_counts: BTreeMap::new(),
            projection_index: None,
        }
    }

    pub fn enterprise_id(&self) -> &EnterpriseId {
        &self.enterprise_id
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn batch_count(&self) -> usize {
        self.batch_event_counts.len()
    }

    pub fn events(&self) -> impl Iterator<Item = &EnterpriseEvent> {
        self.events.values()
    }

    pub fn from_batches(
        enterprise_id: EnterpriseId,
        batches: impl IntoIterator<Item = EnterpriseBatch>,
    ) -> Result<Self, String> {
        let mut graph = Self::new(enterprise_id);
        for batch in batches {
            graph.apply_batch(batch)?;
        }
        Ok(graph)
    }

    pub fn apply_batch(&mut self, batch: EnterpriseBatch) -> Result<EnterpriseMergeReport, String> {
        self.insert_batch(batch).map(|(report, _)| report)
    }

    fn insert_batch(
        &mut self,
        batch: EnterpriseBatch,
    ) -> Result<(EnterpriseMergeReport, Vec<EnterpriseEventId>), String> {
        batch.validate()?;
        if batch.enterprise_id != self.enterprise_id {
            return Err("cannot merge a batch from another enterprise".into());
        }
        let received = batch.events.len();
        if let Some(existing_event_count) = self.batch_event_counts.get(&batch.batch_id) {
            return Ok((
                EnterpriseMergeReport {
                    batch_id: batch.batch_id,
                    received,
                    inserted: 0,
                    duplicates: *existing_event_count,
                },
                Vec::new(),
            ));
        }

        for event in &batch.events {
            if self
                .events
                .get(&event.event_id)
                .is_some_and(|existing| existing != event)
            {
                return Err("enterprise event-id collision".into());
            }
        }
        let batch_id = batch.batch_id;
        let mut inserted = 0;
        let mut duplicates = 0;
        let mut inserted_event_ids = Vec::new();
        for event in batch.events {
            let event_id = event.event_id.clone();
            if self.events.contains_key(&event_id) {
                duplicates += 1;
            } else {
                if let Some(projection_index) = &mut self.projection_index {
                    projection_index.insert(&event);
                }
                inserted_event_ids.push(event_id.clone());
                self.events.insert(event_id, event);
                inserted += 1;
            }
        }
        self.batch_event_counts.insert(batch_id.clone(), received);
        Ok((
            EnterpriseMergeReport {
                batch_id,
                received,
                inserted,
                duplicates,
            },
            inserted_event_ids,
        ))
    }

    pub fn projection_cursor(&self) -> Result<EnterpriseProjectionCursor, String> {
        EnterpriseProjectionCursor::from_graph(self)
    }

    pub fn projection_cursor_from_snapshot(
        &self,
        snapshot: &EnterpriseSnapshot,
    ) -> Result<EnterpriseProjectionCursor, String> {
        if snapshot.enterprise_id != self.enterprise_id
            || snapshot.event_count != self.event_count()
            || snapshot.event_root != self.event_root()?
        {
            return Err("enterprise projection snapshot does not match graph revision".into());
        }
        Ok(EnterpriseProjectionCursor::from_snapshot(self, snapshot))
    }

    pub fn apply_batch_affected(
        &mut self,
        cursor: &mut EnterpriseProjectionCursor,
        batch: EnterpriseBatch,
    ) -> Result<EnterpriseAffectedProjection, String> {
        self.ensure_projection_index();
        incremental::apply_batch(self, cursor, batch)
    }

    pub fn apply_verified_batch_affected(
        &mut self,
        cursor: &mut EnterpriseProjectionCursor,
        batch: &VerifiedEnterpriseBatch,
    ) -> Result<EnterpriseAffectedProjection, String> {
        self.apply_batch_affected(cursor, batch.batch().clone())
    }

    pub fn merge(&mut self, other: &Self) -> Result<EnterpriseMergeReport, String> {
        if other.enterprise_id != self.enterprise_id {
            return Err("cannot merge graphs from different enterprises".into());
        }
        if other.events.is_empty() {
            return Ok(EnterpriseMergeReport {
                batch_id: EnterpriseBatchId::new(format!("batch:{}", self.event_root()?))?,
                received: 0,
                inserted: 0,
                duplicates: 0,
            });
        }
        let batch =
            EnterpriseBatch::new(self.enterprise_id.clone(), other.events.values().cloned())?;
        self.apply_batch(batch)
    }

    pub fn event_root(&self) -> Result<String, String> {
        enterprise_event_root(&self.enterprise_id, self.events.keys())
    }

    pub fn snapshot(&self) -> Result<EnterpriseSnapshot, String> {
        materialize::snapshot(self)
    }

    pub fn draft_discovery_pass_seal(
        &self,
        charter_id: &str,
        discovery_epoch: &str,
        discovery_epoch_sequence: u64,
        previous_pass_id: Option<String>,
        evidence_digests: std::collections::BTreeSet<String>,
    ) -> Result<super::contract::DiscoveryPassSealObservation, String> {
        let mut conflicts = std::collections::BTreeSet::new();
        let retracted = materialize::retracted_events(self.raw_events(), &mut conflicts);
        if !conflicts.is_empty() {
            return Err("cannot seal a discovery pass while retractions conflict".into());
        }
        let active = self
            .raw_events()
            .values()
            .filter(|event| !retracted.contains(&event.event_id))
            .collect::<Vec<_>>();
        control::draft_seal(
            &self.enterprise_id,
            &active,
            charter_id,
            discovery_epoch,
            discovery_epoch_sequence,
            previous_pass_id,
            evidence_digests,
        )
    }

    pub fn query_entities(
        &self,
        query: &EnterpriseQuery,
    ) -> Result<Vec<MaterializedEntity>, String> {
        query.validate()?;
        let snapshot = self.snapshot()?;
        Ok(query::entities(&snapshot, query))
    }

    pub fn neighborhood(
        &self,
        seed: &super::contract::EnterpriseEntityId,
        depth: u8,
        max_nodes: usize,
    ) -> Result<Vec<MaterializedEntity>, String> {
        if depth > 8 {
            return Err("enterprise neighborhood depth cannot exceed 8".into());
        }
        if max_nodes == 0 || max_nodes > 10_000 {
            return Err("enterprise neighborhood max_nodes must be in 1..=10000".into());
        }
        let snapshot = self.snapshot()?;
        query::neighborhood(&snapshot, seed, depth, max_nodes)
    }

    pub(super) fn raw_events(&self) -> &BTreeMap<EnterpriseEventId, EnterpriseEvent> {
        &self.events
    }

    fn projection_index(&self) -> &incremental::ProjectionIndex {
        self.projection_index
            .as_ref()
            .expect("affected projection initializes its event index")
    }

    fn max_seal_epoch_sequence(&self) -> Option<u64> {
        self.projection_index
            .as_ref()
            .and_then(incremental::ProjectionIndex::max_seal_epoch_sequence)
            .or_else(|| {
                self.events
                    .values()
                    .filter_map(|event| match &event.fact {
                        super::contract::EnterpriseFact::DiscoveryPassSealed(value) => {
                            Some(value.discovery_epoch_sequence)
                        }
                        _ => None,
                    })
                    .max()
            })
    }

    fn ensure_projection_index(&mut self) {
        if self.projection_index.is_some() {
            return;
        }
        let mut index = incremental::ProjectionIndex::default();
        for event in self.events.values() {
            index.insert(event);
        }
        self.projection_index = Some(index);
    }
}

pub fn enterprise_event_root<'a>(
    enterprise_id: &EnterpriseId,
    event_ids: impl IntoIterator<Item = &'a EnterpriseEventId>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct EventRoot<'a> {
        schema: &'static str,
        enterprise_id: &'a EnterpriseId,
        event_ids: Vec<&'a EnterpriseEventId>,
    }
    canonical_digest(&EventRoot {
        schema: "scout-enterprise-event-root-v2",
        enterprise_id,
        event_ids: event_ids.into_iter().collect(),
    })
}

impl EnterpriseSnapshot {
    pub fn refresh_graph_digest(&mut self) -> Result<(), String> {
        self.graph_digest = materialize::snapshot_digest_from_snapshot(self)?;
        Ok(())
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn lazy_projection_index_is_not_part_of_graph_identity() {
        let mut graph = EnterpriseGraph::new(EnterpriseId::new("enterprise:test").unwrap());
        let without_index = graph.clone();

        assert!(graph.projection_index.is_none());
        graph.ensure_projection_index();

        assert!(graph.projection_index.is_some());
        assert_eq!(graph, without_index);
    }
}
