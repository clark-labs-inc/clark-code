use std::collections::BTreeMap;
use std::time::Instant;

use scout_accumulator::{
    plan_insert, prove_persistent, verify_proof, Accumulator, AccumulatorContext, AccumulatorHead,
    Digest, ProofStatus, StoredNode,
};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

pub fn incremental_accumulator(service_count: usize) -> Result<(String, Value), String> {
    let object_count = service_count.saturating_mul(10).clamp(10_000, 100_000);
    let context = AccumulatorContext::new(
        "clark.scout.enterprise-ledger",
        "benchmark-enterprise",
        "batch",
    )
    .map_err(|error| error.to_string())?;
    let mut head = AccumulatorHead::empty(&context);
    let mut nodes = BTreeMap::<Digest, StoredNode>::new();
    let mut node_writes = 0_usize;
    let mut max_nodes_touched = 0_usize;
    let started = Instant::now();
    for index in 0..object_count {
        let object_id = object_id(index);
        let mutation = plan_insert(&context, head, &object_id, |digest| {
            Ok(nodes.get(&digest).cloned())
        })
        .map_err(|error| error.to_string())?;
        node_writes = node_writes.saturating_add(mutation.nodes.len());
        max_nodes_touched = max_nodes_touched.max(
            mutation
                .nodes
                .len()
                .saturating_add(mutation.obsolete_nodes.len()),
        );
        for node in mutation.nodes {
            let digest = node.digest(&context).map_err(|error| error.to_string())?;
            if let Some(existing) = nodes.insert(digest, node.clone()) {
                if existing != node {
                    return Err("accumulator content address collided".into());
                }
            }
        }
        for digest in mutation.obsolete_nodes {
            if nodes.remove(&digest).is_none() {
                return Err("accumulator garbage collection missed an old path node".into());
            }
        }
        head = mutation.next;
    }
    let incremental_insert_ms = started.elapsed().as_millis();
    let expected_active_nodes = object_count.saturating_mul(2).saturating_sub(1);
    if head.root.count != object_count as u64 || nodes.len() != expected_active_nodes {
        return Err("incremental accumulator count or active-node bound diverged".into());
    }

    let started = Instant::now();
    let mut reverse = Accumulator::new(context.clone());
    for index in (0..object_count).rev() {
        reverse
            .insert(object_id(index))
            .map_err(|error| error.to_string())?;
    }
    let reverse_build_ms = started.elapsed().as_millis();
    if reverse.root() != head.root {
        return Err("accumulator root changed with insertion order".into());
    }

    let mut max_proof_bytes = 0_usize;
    for object_id in [
        object_id(0),
        object_id(object_count / 2),
        object_id(object_count - 1),
        format!("batch:{}", "f".repeat(64)),
    ] {
        let proof = prove_persistent(&context, head, &object_id, |digest| {
            Ok(nodes.get(&digest).cloned())
        })
        .map_err(|error| error.to_string())?;
        let status = verify_proof(&head.root, &proof).map_err(|error| error.to_string())?;
        let expected = if object_id.ends_with(&"f".repeat(64)) {
            ProofStatus::NonMember
        } else {
            ProofStatus::Member
        };
        if status != expected {
            return Err("accumulator membership proof returned the wrong status".into());
        }
        max_proof_bytes = max_proof_bytes.max(
            serde_json::to_vec(&proof)
                .map_err(|error| error.to_string())?
                .len(),
        );
    }
    let semantic_payload = json!({
        "objects": object_count,
        "root": head.root.digest,
        "count": head.root.count,
        "active_nodes": nodes.len(),
        "order_independent": true,
        "membership_and_nonmembership_verified": true,
    });
    let semantic_sha256 = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&semantic_payload).map_err(|error| error.to_string())?)
    );
    Ok((
        format!("{object_count} objects converged with logarithmic authenticated updates"),
        json!({
            "objects": object_count,
            "root": head.root.digest,
            "count": head.root.count,
            "active_nodes": nodes.len(),
            "node_writes": node_writes,
            "average_node_writes": node_writes as f64 / object_count as f64,
            "max_nodes_touched": max_nodes_touched,
            "max_proof_bytes": max_proof_bytes,
            "incremental_insert_ms": incremental_insert_ms,
            "reverse_build_ms": reverse_build_ms,
            "order_independent": true,
            "membership_and_nonmembership_verified": true,
            "semantic_sha256": semantic_sha256,
        }),
    ))
}

fn object_id(index: usize) -> String {
    format!("batch:{index:064x}")
}
