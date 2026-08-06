use std::collections::{BTreeMap, BTreeSet};

use super::{PassMembership, QualifiedTopology};
use crate::scout::enterprise::contract::{EnterpriseEdgeId, EnterpriseEntityId};
use crate::scout::enterprise::graph::{EnterpriseConflict, MaterializedDiscoveryPass};

pub(super) struct QualificationSelection {
    pub current_pass_id: Option<String>,
    pub fixed_point: bool,
    pub member_entity_ids: BTreeSet<EnterpriseEntityId>,
    pub member_edge_ids: BTreeSet<EnterpriseEdgeId>,
    pub qualified_topologies: Vec<QualifiedTopology>,
    pub blockers: Vec<String>,
}

pub(super) fn select(
    passes: &BTreeMap<String, MaterializedDiscoveryPass>,
    memberships: &BTreeMap<String, PassMembership>,
    verified_by_sequence: &BTreeMap<u64, BTreeSet<String>>,
    latest_attempt: u64,
    conflicts: &mut BTreeSet<EnterpriseConflict>,
) -> QualificationSelection {
    let unique = |pass: &MaterializedDiscoveryPass| {
        pass.verified
            && verified_by_sequence
                .get(&pass.discovery_epoch_sequence)
                .is_some_and(|ids| ids.len() == 1 && ids.contains(&pass.pass_id))
    };
    let mut candidates = passes
        .values()
        .filter(|pass| unique(pass))
        .filter_map(|confirming| {
            let first = confirming
                .previous_pass_id
                .as_ref()
                .and_then(|pass_id| passes.get(pass_id))?;
            (unique(first)
                && first.discovery_epoch_sequence < confirming.discovery_epoch_sequence
                && first.charter_id == confirming.charter_id
                && first.requirement_root == confirming.requirement_root
                && first.scope_root == confirming.scope_root
                && first.topology_root == confirming.topology_root)
                .then_some((first, confirming))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        (left.1.discovery_epoch_sequence, &left.1.pass_id)
            .cmp(&(right.1.discovery_epoch_sequence, &right.1.pass_id))
    });

    let mut qualified = Vec::<QualifiedTopology>::new();
    let mut conflict_blocked_candidate = false;
    for (first, confirming) in candidates {
        let after_sequence = qualified
            .last()
            .map_or(0, |current| current.discovery_epoch_sequence);
        if has_relevant_control_conflict(
            conflicts,
            passes,
            after_sequence,
            confirming.discovery_epoch_sequence,
        ) {
            conflict_blocked_candidate = true;
            continue;
        }
        if first.sealed_at_ms > confirming.sealed_at_ms
            || qualified
                .last()
                .is_some_and(|current| first.sealed_at_ms <= current.valid_from_ms)
        {
            conflicts.insert(EnterpriseConflict::DiscoveryPassNonMonotonic {
                first_pass_id: first.pass_id.clone(),
                confirming_pass_id: confirming.pass_id.clone(),
            });
            continue;
        }
        let Some(membership) = memberships.get(&confirming.pass_id) else {
            continue;
        };
        let same_version = qualified.last().is_some_and(|current| {
            current.charter_id == confirming.charter_id
                && current.requirement_root == confirming.requirement_root
                && current.scope_root == confirming.scope_root
                && current.topology_root == confirming.topology_root
        });
        if same_version {
            let current = qualified.last_mut().expect("checked last topology");
            current.confirming_pass_id = confirming.pass_id.clone();
            current.discovery_epoch_sequence = confirming.discovery_epoch_sequence;
            current.member_entity_ids = membership.entities.clone();
            current.member_edge_ids = membership.edges.clone();
            current.entity_scopes = membership.entity_scopes.clone();
            current.edge_scopes = membership.edge_scopes.clone();
            continue;
        }
        qualified.push(QualifiedTopology {
            confirming_pass_id: confirming.pass_id.clone(),
            discovery_epoch_sequence: confirming.discovery_epoch_sequence,
            valid_from_ms: first.sealed_at_ms,
            charter_id: confirming.charter_id.clone(),
            requirement_root: confirming.requirement_root.clone(),
            scope_root: confirming.scope_root.clone(),
            topology_root: confirming.topology_root.clone(),
            member_entity_ids: membership.entities.clone(),
            member_edge_ids: membership.edges.clone(),
            entity_scopes: membership.entity_scopes.clone(),
            edge_scopes: membership.edge_scopes.clone(),
        });
    }

    let mut blockers = Vec::new();
    let Some(current) = qualified.last() else {
        blockers.push(
            "enterprise has no two-pass verified linked fixed-point topology qualification".into(),
        );
        return QualificationSelection {
            current_pass_id: None,
            fixed_point: false,
            member_entity_ids: BTreeSet::new(),
            member_edge_ids: BTreeSet::new(),
            qualified_topologies: qualified,
            blockers,
        };
    };
    let fixed_point = latest_attempt <= current.discovery_epoch_sequence;
    if !fixed_point {
        blockers.push(format!(
            "discovery epoch {latest_attempt} is newer than the latest verified pass {}; qualified topology remains unchanged",
            current.confirming_pass_id
        ));
    }
    if conflict_blocked_candidate {
        blockers.push(
            "a relevant invalid or forked discovery pass blocks newer topology qualification"
                .into(),
        );
    }
    QualificationSelection {
        current_pass_id: Some(current.confirming_pass_id.clone()),
        fixed_point,
        member_entity_ids: current.member_entity_ids.clone(),
        member_edge_ids: current.member_edge_ids.clone(),
        qualified_topologies: qualified,
        blockers,
    }
}

fn has_relevant_control_conflict(
    conflicts: &BTreeSet<EnterpriseConflict>,
    passes: &BTreeMap<String, MaterializedDiscoveryPass>,
    after_sequence: u64,
    through_sequence: u64,
) -> bool {
    conflicts.iter().any(|conflict| {
        let sequence = match conflict {
            EnterpriseConflict::DiscoveryPassInvalid { pass_id } => passes
                .get(pass_id)
                .map(|pass| pass.discovery_epoch_sequence),
            EnterpriseConflict::DiscoveryPassFork {
                discovery_epoch_sequence,
                ..
            } => Some(*discovery_epoch_sequence),
            EnterpriseConflict::DiscoveryPassNonMonotonic {
                first_pass_id,
                confirming_pass_id,
            } => passes
                .get(confirming_pass_id)
                .or_else(|| passes.get(first_pass_id))
                .map(|pass| pass.discovery_epoch_sequence),
            _ => None,
        };
        sequence.is_some_and(|sequence| after_sequence < sequence && sequence <= through_sequence)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    type QualificationFixture = (
        BTreeMap<String, MaterializedDiscoveryPass>,
        BTreeMap<String, PassMembership>,
        BTreeMap<u64, BTreeSet<String>>,
    );

    #[test]
    fn invalid_intermediate_attempt_freezes_later_qualified_topology() {
        let (mut passes, memberships, verified) = baseline_and_changed_pairs();
        passes.insert(
            "invalid".into(),
            pass("invalid", 3, Some("a2"), false, "bad"),
        );
        let mut conflicts = BTreeSet::from([EnterpriseConflict::DiscoveryPassInvalid {
            pass_id: "invalid".into(),
        }]);

        let selected = select(&passes, &memberships, &verified, 5, &mut conflicts);

        assert_eq!(selected.current_pass_id.as_deref(), Some("a2"));
        assert_eq!(selected.qualified_topologies.len(), 1);
        assert_eq!(selected.qualified_topologies[0].topology_root, "a");
        assert!(selected
            .blockers
            .iter()
            .any(|blocker| blocker.contains("invalid or forked")));
    }

    #[test]
    fn verified_fork_freezes_later_qualified_topology() {
        let (mut passes, memberships, mut verified) = baseline_and_changed_pairs();
        passes.insert(
            "fork-left".into(),
            pass("fork-left", 3, Some("a2"), true, "a"),
        );
        passes.insert("fork-right".into(), pass("fork-right", 3, None, true, "a"));
        verified.insert(3, BTreeSet::from(["fork-left".into(), "fork-right".into()]));
        let mut conflicts = BTreeSet::from([EnterpriseConflict::DiscoveryPassFork {
            discovery_epoch_sequence: 3,
            pass_ids: verified[&3].clone(),
        }]);

        let selected = select(&passes, &memberships, &verified, 5, &mut conflicts);

        assert_eq!(selected.current_pass_id.as_deref(), Some("a2"));
        assert_eq!(selected.qualified_topologies.len(), 1);
        assert_eq!(selected.qualified_topologies[0].topology_root, "a");
    }

    fn baseline_and_changed_pairs() -> QualificationFixture {
        let passes = [
            pass("a1", 1, None, true, "a"),
            pass("a2", 2, Some("a1"), true, "a"),
            pass("b1", 4, Some("a2"), true, "b"),
            pass("b2", 5, Some("b1"), true, "b"),
        ]
        .into_iter()
        .map(|pass| (pass.pass_id.clone(), pass))
        .collect::<BTreeMap<_, _>>();
        let memberships = passes
            .keys()
            .map(|pass_id| (pass_id.clone(), PassMembership::default()))
            .collect();
        let verified = passes
            .values()
            .map(|pass| {
                (
                    pass.discovery_epoch_sequence,
                    BTreeSet::from([pass.pass_id.clone()]),
                )
            })
            .collect();
        (passes, memberships, verified)
    }

    fn pass(
        pass_id: &str,
        sequence: u64,
        previous: Option<&str>,
        verified: bool,
        topology: &str,
    ) -> MaterializedDiscoveryPass {
        MaterializedDiscoveryPass {
            pass_id: pass_id.into(),
            charter_id: "charter".into(),
            discovery_epoch: format!("epoch-{sequence}"),
            discovery_epoch_sequence: sequence,
            sealed_at_ms: sequence * 100,
            previous_pass_id: previous.map(str::to_owned),
            requirement_root: "requirement".into(),
            scope_root: "scope".into(),
            topology_root: topology.into(),
            verified,
            evidence_digests: BTreeSet::new(),
            supporting_events: BTreeSet::new(),
        }
    }
}
