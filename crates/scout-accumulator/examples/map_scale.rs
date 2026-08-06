use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::time::Instant;

use scout_accumulator::{
    plan_map_put, plan_map_remove, prove_map_persistent, verify_map_proof, AccumulatorContext,
    Digest, MapHead, MapMutation, MapMutationOutcome, MapProofStatus, MapStoredNode,
};
use serde::Serialize;

type NodeStore = BTreeMap<Digest, MapStoredNode>;

#[derive(Serialize)]
struct Receipt {
    schema: &'static str,
    entries: usize,
    updates: usize,
    removals: usize,
    forward_build_ms: u128,
    reverse_build_ms: u128,
    mutate_ms: u128,
    cold_final_build_ms: u128,
    order_independent: bool,
    incremental_matches_cold_final: bool,
    final_count: u64,
    active_nodes: usize,
    expected_active_nodes: usize,
    max_nodes_written_per_mutation: usize,
    max_gc_candidates_per_mutation: usize,
    updated_proof: &'static str,
    removed_proof: &'static str,
    final_root: String,
    passed: bool,
}

fn main() -> Result<(), String> {
    let entries = argument(1, 100_000)?;
    let updates = argument(2, entries.min(10_000))?.min(entries);
    let removals = argument(3, entries.saturating_sub(updates).min(10_000))?
        .min(entries.saturating_sub(updates));
    if entries == 0 {
        return Err("map scale entry count must be positive".into());
    }
    let context = AccumulatorContext::new(
        "clark.scout.enterprise-projection",
        "benchmark-enterprise",
        "entity",
    )
    .map_err(|error| error.to_string())?;

    let started = Instant::now();
    let (mut head, mut nodes, mut stats) = build(&context, entries, false)?;
    let forward_build_ms = started.elapsed().as_millis();

    let started = Instant::now();
    let (reverse, _, reverse_stats) = build(&context, entries, true)?;
    let reverse_build_ms = started.elapsed().as_millis();
    stats.merge(reverse_stats);
    let order_independent = head.root == reverse.root;

    let started = Instant::now();
    for index in 0..updates {
        let object_id = object_id(index);
        let mutation = plan_map_put(
            &context,
            head,
            &object_id,
            value_digest(index, 1),
            |digest| Ok(nodes.get(&digest).cloned()),
        )
        .map_err(|error| error.to_string())?;
        if mutation.outcome != MapMutationOutcome::Updated {
            return Err("map scale update did not replace an existing value".into());
        }
        stats.observe(&mutation);
        apply_current(&context, &mut nodes, &mutation)?;
        head = mutation.next;
    }
    for index in updates..updates + removals {
        let object_id = object_id(index);
        let mutation = plan_map_remove(&context, head, &object_id, |digest| {
            Ok(nodes.get(&digest).cloned())
        })
        .map_err(|error| error.to_string())?;
        if mutation.outcome != MapMutationOutcome::Removed {
            return Err("map scale removal did not remove an existing value".into());
        }
        stats.observe(&mutation);
        apply_current(&context, &mut nodes, &mutation)?;
        head = mutation.next;
    }
    let mutate_ms = started.elapsed().as_millis();

    let started = Instant::now();
    let (cold_final, _, cold_stats) = build_final(&context, entries, updates, removals)?;
    let cold_final_build_ms = started.elapsed().as_millis();
    stats.merge(cold_stats);
    let incremental_matches_cold_final = head.root == cold_final.root;

    let updated_proof = prove_map_persistent(&context, head, object_id(0), |digest| {
        Ok(nodes.get(&digest).cloned())
    })
    .map_err(|error| error.to_string())?;
    let updated_proof =
        verify_map_proof(&head.root, &updated_proof).map_err(|error| error.to_string())?;
    let removed_proof = prove_map_persistent(&context, head, object_id(updates), |digest| {
        Ok(nodes.get(&digest).cloned())
    })
    .map_err(|error| error.to_string())?;
    let removed_proof =
        verify_map_proof(&head.root, &removed_proof).map_err(|error| error.to_string())?;
    let expected_active_nodes = head.root.count.saturating_mul(2).saturating_sub(1) as usize;
    let passed = order_independent
        && incremental_matches_cold_final
        && nodes.len() == expected_active_nodes
        && matches!(
            updated_proof,
            MapProofStatus::Present { value_digest }
                if value_digest == value_digest_for(0, 1)
        )
        && removed_proof == MapProofStatus::Absent;
    let receipt = Receipt {
        schema: "scout-authenticated-map-scale-v1",
        entries,
        updates,
        removals,
        forward_build_ms,
        reverse_build_ms,
        mutate_ms,
        cold_final_build_ms,
        order_independent,
        incremental_matches_cold_final,
        final_count: head.root.count,
        active_nodes: nodes.len(),
        expected_active_nodes,
        max_nodes_written_per_mutation: stats.max_nodes,
        max_gc_candidates_per_mutation: stats.max_gc,
        updated_proof: proof_label(updated_proof),
        removed_proof: proof_label(removed_proof),
        final_root: head.root.digest.to_hex(),
        passed,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).map_err(|error| error.to_string())?
    );
    passed
        .then_some(())
        .ok_or_else(|| "authenticated map scale qualification failed".into())
}

#[derive(Default)]
struct MutationStats {
    max_nodes: usize,
    max_gc: usize,
}

impl MutationStats {
    fn observe(&mut self, mutation: &MapMutation) {
        self.max_nodes = self.max_nodes.max(mutation.nodes.len());
        self.max_gc = self.max_gc.max(mutation.gc_candidates.len());
    }

    fn merge(&mut self, other: Self) {
        self.max_nodes = self.max_nodes.max(other.max_nodes);
        self.max_gc = self.max_gc.max(other.max_gc);
    }
}

fn build(
    context: &AccumulatorContext,
    entries: usize,
    reverse: bool,
) -> Result<(MapHead, NodeStore, MutationStats), String> {
    let mut head = MapHead::empty(context);
    let mut nodes = NodeStore::new();
    let mut stats = MutationStats::default();
    let indices: Box<dyn Iterator<Item = usize>> = if reverse {
        Box::new((0..entries).rev())
    } else {
        Box::new(0..entries)
    };
    for index in indices {
        let mutation = plan_map_put(
            context,
            head,
            object_id(index),
            value_digest(index, 0),
            |digest| Ok(nodes.get(&digest).cloned()),
        )
        .map_err(|error| error.to_string())?;
        stats.observe(&mutation);
        apply_current(context, &mut nodes, &mutation)?;
        head = mutation.next;
    }
    Ok((head, nodes, stats))
}

fn build_final(
    context: &AccumulatorContext,
    entries: usize,
    updates: usize,
    removals: usize,
) -> Result<(MapHead, NodeStore, MutationStats), String> {
    let mut head = MapHead::empty(context);
    let mut nodes = NodeStore::new();
    let mut stats = MutationStats::default();
    for index in 0..entries {
        if (updates..updates + removals).contains(&index) {
            continue;
        }
        let version = usize::from(index < updates);
        let mutation = plan_map_put(
            context,
            head,
            object_id(index),
            value_digest(index, version),
            |digest| Ok(nodes.get(&digest).cloned()),
        )
        .map_err(|error| error.to_string())?;
        stats.observe(&mutation);
        apply_current(context, &mut nodes, &mutation)?;
        head = mutation.next;
    }
    Ok((head, nodes, stats))
}

fn apply_current(
    context: &AccumulatorContext,
    nodes: &mut NodeStore,
    mutation: &MapMutation,
) -> Result<(), String> {
    let mut written = BTreeSet::new();
    for node in &mutation.nodes {
        let digest = node.digest(context).map_err(|error| error.to_string())?;
        written.insert(digest);
        if let Some(existing) = nodes.insert(digest, node.clone()) {
            if existing != *node {
                return Err("authenticated map digest collision".into());
            }
        }
    }
    for digest in &mutation.gc_candidates {
        if !written.contains(digest) {
            nodes.remove(digest);
        }
    }
    Ok(())
}

fn argument(index: usize, default: usize) -> Result<usize, String> {
    env::args()
        .nth(index)
        .map(|value| {
            value
                .parse()
                .map_err(|_| format!("argument {index} must be a nonnegative integer"))
        })
        .unwrap_or(Ok(default))
}

fn object_id(index: usize) -> String {
    format!("entity:{index:012}")
}

fn value_digest(index: usize, version: usize) -> Digest {
    value_digest_for(index, version)
}

fn value_digest_for(index: usize, version: usize) -> Digest {
    let mut bytes = [0_u8; 32];
    bytes[..8].copy_from_slice(&(index as u64).to_be_bytes());
    bytes[8..16].copy_from_slice(&(version as u64).to_be_bytes());
    Digest::from_bytes(bytes)
}

fn proof_label(status: MapProofStatus) -> &'static str {
    match status {
        MapProofStatus::Present { .. } => "present",
        MapProofStatus::Absent => "absent",
    }
}
