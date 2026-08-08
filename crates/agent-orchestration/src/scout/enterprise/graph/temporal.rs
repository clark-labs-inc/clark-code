use std::collections::BTreeMap;

use super::control::QualifiedTopology;
use super::materialize::{apply_endpoint_classification, materialize_edges, materialize_entities};
use super::model::{MaterializedEdge, MaterializedEntity, QualifiedLifecycle};
use crate::scout::enterprise::contract::{EnterpriseEdgeId, EnterpriseEntityId, EnterpriseEvent};

pub(super) struct TemporalProjection {
    pub entities: BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    pub edges: BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    pub entity_history: BTreeMap<EnterpriseEntityId, Vec<MaterializedEntity>>,
    pub edge_history: BTreeMap<EnterpriseEdgeId, Vec<MaterializedEdge>>,
}

pub(super) fn project(
    events: &[&EnterpriseEvent],
    qualifications: &[QualifiedTopology],
) -> TemporalProjection {
    let mut entity_history = BTreeMap::<EnterpriseEntityId, Vec<MaterializedEntity>>::new();
    let mut edge_history = BTreeMap::<EnterpriseEdgeId, Vec<MaterializedEdge>>::new();
    for (index, qualification) in qualifications.iter().enumerate() {
        let as_of = events
            .iter()
            .copied()
            .filter(|event| {
                event.provenance.discovery_epoch_sequence <= qualification.discovery_epoch_sequence
            })
            .collect::<Vec<_>>();
        let mut entities = materialize_entities(&as_of);
        let mut edges = materialize_edges(&as_of);
        entities.retain(|id, _| qualification.member_entity_ids.contains(id));
        edges.retain(|id, _| qualification.member_edge_ids.contains(id));
        apply_monotone_entity_classification(&mut entities, &entity_history);
        apply_endpoint_classification(&entities, &mut edges);
        apply_monotone_edge_classification(&mut edges, &edge_history);
        let previous = index
            .checked_sub(1)
            .and_then(|prior| qualifications.get(prior));
        merge_entities(&mut entity_history, entities, qualification, previous);
        merge_edges(&mut edge_history, edges, qualification, previous);
    }
    let entities = entity_history
        .iter()
        .filter_map(|(id, versions)| {
            versions
                .last()
                .filter(|version| version.valid_to_ms.is_none())
                .map(|version| (id.clone(), version.clone()))
        })
        .collect();
    let edges = edge_history
        .iter()
        .filter_map(|(id, versions)| {
            versions
                .last()
                .filter(|version| version.valid_to_ms.is_none())
                .map(|version| (id.clone(), version.clone()))
        })
        .collect();
    TemporalProjection {
        entities,
        edges,
        entity_history,
        edge_history,
    }
}

fn merge_entities(
    history: &mut BTreeMap<EnterpriseEntityId, Vec<MaterializedEntity>>,
    next: BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    qualification: &QualifiedTopology,
    previous: Option<&QualifiedTopology>,
) {
    close_missing_entities(history, &next, qualification, previous);
    for (id, mut record) in next {
        let versions = history.entry(id).or_default();
        if versions
            .last()
            .is_some_and(|current| current.valid_to_ms.is_none() && same_entity(current, &record))
        {
            continue;
        }
        close_open_entity(
            versions,
            qualification.valid_from_ms,
            QualifiedLifecycle::Retired,
        );
        record.valid_from_ms = Some(qualification.valid_from_ms);
        record.valid_to_ms = None;
        record.qualified_pass_id = Some(qualification.confirming_pass_id.clone());
        record.lifecycle = QualifiedLifecycle::Active;
        versions.push(record);
    }
}

fn close_missing_entities(
    history: &mut BTreeMap<EnterpriseEntityId, Vec<MaterializedEntity>>,
    next: &BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    qualification: &QualifiedTopology,
    previous: Option<&QualifiedTopology>,
) {
    let continuing_scope = previous.is_some_and(|prior| {
        prior.charter_id == qualification.charter_id
            && prior.requirement_root == qualification.requirement_root
    });
    for (id, versions) in history {
        if !next.contains_key(id) {
            close_open_entity(
                versions,
                qualification.valid_from_ms,
                if continuing_scope {
                    QualifiedLifecycle::Retired
                } else {
                    QualifiedLifecycle::OutOfScope
                },
            );
        }
    }
}

fn merge_edges(
    history: &mut BTreeMap<EnterpriseEdgeId, Vec<MaterializedEdge>>,
    next: BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    qualification: &QualifiedTopology,
    previous: Option<&QualifiedTopology>,
) {
    let continuing_scope = previous.is_some_and(|prior| {
        prior.charter_id == qualification.charter_id
            && prior.requirement_root == qualification.requirement_root
    });
    for (id, versions) in history.iter_mut() {
        if !next.contains_key(id) {
            close_open_edge(
                versions,
                qualification.valid_from_ms,
                if continuing_scope {
                    QualifiedLifecycle::Retired
                } else {
                    QualifiedLifecycle::OutOfScope
                },
            );
        }
    }
    for (id, mut record) in next {
        let versions = history.entry(id).or_default();
        if versions
            .last()
            .is_some_and(|current| current.valid_to_ms.is_none() && same_edge(current, &record))
        {
            continue;
        }
        close_open_edge(
            versions,
            qualification.valid_from_ms,
            QualifiedLifecycle::Retired,
        );
        record.valid_from_ms = Some(qualification.valid_from_ms);
        record.valid_to_ms = None;
        record.qualified_pass_id = Some(qualification.confirming_pass_id.clone());
        record.lifecycle = QualifiedLifecycle::Active;
        versions.push(record);
    }
}

fn close_open_entity(
    versions: &mut [MaterializedEntity],
    valid_to_ms: u64,
    lifecycle: QualifiedLifecycle,
) {
    if let Some(current) = versions
        .last_mut()
        .filter(|record| record.valid_to_ms.is_none())
    {
        current.valid_to_ms = Some(valid_to_ms);
        current.lifecycle = lifecycle;
    }
}

fn close_open_edge(
    versions: &mut [MaterializedEdge],
    valid_to_ms: u64,
    lifecycle: QualifiedLifecycle,
) {
    if let Some(current) = versions
        .last_mut()
        .filter(|record| record.valid_to_ms.is_none())
    {
        current.valid_to_ms = Some(valid_to_ms);
        current.lifecycle = lifecycle;
    }
}

fn apply_monotone_entity_classification(
    entities: &mut BTreeMap<EnterpriseEntityId, MaterializedEntity>,
    history: &BTreeMap<EnterpriseEntityId, Vec<MaterializedEntity>>,
) {
    for (id, entity) in entities {
        if let Some(previous) = history.get(id).and_then(|versions| versions.last()) {
            entity.classification = entity.classification.join(previous.classification);
        }
    }
}

fn apply_monotone_edge_classification(
    edges: &mut BTreeMap<EnterpriseEdgeId, MaterializedEdge>,
    history: &BTreeMap<EnterpriseEdgeId, Vec<MaterializedEdge>>,
) {
    for (id, edge) in edges {
        if let Some(previous) = history.get(id).and_then(|versions| versions.last()) {
            edge.classification = edge.classification.join(previous.classification);
        }
    }
}

fn same_entity(left: &MaterializedEntity, right: &MaterializedEntity) -> bool {
    left.entity_id == right.entity_id
        && left.kind == right.kind
        && left.authority == right.authority
        && left.labels == right.labels
        && left.environments == right.environments
        && left.critical == right.critical
        && left.classification == right.classification
}

fn same_edge(left: &MaterializedEdge, right: &MaterializedEdge) -> bool {
    left.edge_id == right.edge_id
        && left.from == right.from
        && left.to == right.to
        && left.kind == right.kind
        && left.qualifier == right.qualifier
        && left.classification == right.classification
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::scout::enterprise::contract::{
        AuthorityRef, EnterpriseEntityKind, EnterpriseFact, EnterpriseId, EnterpriseProvenance,
        GraphEntityObservation,
    };

    #[test]
    fn cross_charter_scope_drop_is_out_of_scope_and_reappearance_is_disjoint() {
        let enterprise = EnterpriseId::new("temporal-test").unwrap();
        let observation = GraphEntityObservation::new(
            &enterprise,
            EnterpriseEntityKind::Service,
            AuthorityRef::new("aws", "account:test", "service:checkout").unwrap(),
            BTreeSet::from(["checkout".into()]),
            BTreeSet::from(["a".repeat(64)]),
        )
        .unwrap();
        let entity_id = observation.entity_id.clone();
        let event = EnterpriseEvent::new(
            enterprise.clone(),
            EnterpriseProvenance {
                machine_id: "machine-a".into(),
                run_id: "run-a".into(),
                adapter_instance_id: "adapter-a".into(),
                auth_context_id: "auth-a".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                source_sequence: 1,
                observed_at_ms: 10,
                source_fingerprint: "f".repeat(64),
            },
            EnterpriseFact::EntityObserved(observation),
        )
        .unwrap();
        let first = qualification("charter-a", "requirement-a", 1, 100, [&entity_id]);
        let dropped = qualification("charter-b", "requirement-b", 2, 200, std::iter::empty());
        let reappeared = qualification("charter-c", "requirement-c", 3, 300, [&entity_id]);

        let projection = project(&[&event], &[first, dropped, reappeared]);
        let versions = &projection.entity_history[&entity_id];
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].valid_from_ms, Some(100));
        assert_eq!(versions[0].valid_to_ms, Some(200));
        assert_eq!(versions[0].lifecycle, QualifiedLifecycle::OutOfScope);
        assert_eq!(versions[1].valid_from_ms, Some(300));
        assert_eq!(versions[1].valid_to_ms, None);
        assert_eq!(versions[1].lifecycle, QualifiedLifecycle::Active);
    }

    fn qualification<'a>(
        charter_id: &str,
        requirement_root: &str,
        sequence: u64,
        valid_from_ms: u64,
        entity_ids: impl IntoIterator<Item = &'a EnterpriseEntityId>,
    ) -> QualifiedTopology {
        QualifiedTopology {
            confirming_pass_id: format!("pass-{sequence}"),
            discovery_epoch_sequence: sequence,
            valid_from_ms,
            charter_id: charter_id.into(),
            requirement_root: requirement_root.into(),
            scope_root: format!("scope-{sequence}"),
            topology_root: format!("topology-{sequence}"),
            member_entity_ids: entity_ids.into_iter().cloned().collect(),
            member_edge_ids: BTreeSet::new(),
            entity_scopes: BTreeMap::new(),
            edge_scopes: BTreeMap::new(),
        }
    }
}
