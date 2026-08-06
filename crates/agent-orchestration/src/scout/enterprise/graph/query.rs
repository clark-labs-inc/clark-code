use std::collections::{BTreeSet, VecDeque};

use super::super::contract::{EnterpriseEntityId, EnterpriseEntityKind};
use super::model::{EnterpriseSnapshot, MaterializedEntity};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnterpriseQuery {
    pub kind: Option<EnterpriseEntityKind>,
    pub provider_namespace: Option<String>,
    pub authority_scope: Option<String>,
    pub label_contains: Option<String>,
    pub critical: Option<bool>,
    pub after_entity_id: Option<EnterpriseEntityId>,
    pub limit: usize,
}

impl EnterpriseQuery {
    pub fn validate(&self) -> Result<(), String> {
        if self.limit == 0 || self.limit > 10_000 {
            return Err("enterprise query limit must be in 1..=10000".into());
        }
        Ok(())
    }
}

pub(super) fn entities(
    snapshot: &EnterpriseSnapshot,
    query: &EnterpriseQuery,
) -> Vec<MaterializedEntity> {
    snapshot
        .entities
        .values()
        .filter(|entity| {
            query
                .after_entity_id
                .as_ref()
                .is_none_or(|after| &entity.entity_id > after)
                && query.kind.is_none_or(|kind| entity.kind == kind)
                && query
                    .provider_namespace
                    .as_ref()
                    .is_none_or(|namespace| entity.authority.provider_namespace == *namespace)
                && query
                    .authority_scope
                    .as_ref()
                    .is_none_or(|scope| entity.authority.authority_scope == *scope)
                && query.label_contains.as_ref().is_none_or(|needle| {
                    let needle = needle.to_lowercase();
                    entity
                        .labels
                        .iter()
                        .any(|label| label.to_lowercase().contains(&needle))
                })
                && query
                    .critical
                    .is_none_or(|critical| entity.critical == critical)
        })
        .take(query.limit)
        .cloned()
        .collect()
}

pub(super) fn neighborhood(
    snapshot: &EnterpriseSnapshot,
    seed: &EnterpriseEntityId,
    depth: u8,
    max_nodes: usize,
) -> Result<Vec<MaterializedEntity>, String> {
    if !snapshot.entities.contains_key(seed) {
        return Err(format!("unknown enterprise entity {seed}"));
    }
    let mut seen = BTreeSet::from([seed.clone()]);
    let mut queue = VecDeque::from([(seed.clone(), 0_u8)]);
    while let Some((current, current_depth)) = queue.pop_front() {
        if current_depth >= depth || seen.len() >= max_nodes {
            continue;
        }
        for neighbor in snapshot.edges.values().filter_map(|edge| {
            if edge.from == current {
                Some(edge.to.clone())
            } else if edge.to == current {
                Some(edge.from.clone())
            } else {
                None
            }
        }) {
            if seen.insert(neighbor.clone()) {
                queue.push_back((neighbor, current_depth + 1));
                if seen.len() >= max_nodes {
                    break;
                }
            }
        }
    }
    Ok(seen
        .into_iter()
        .filter_map(|id| snapshot.entities.get(&id).cloned())
        .collect())
}
