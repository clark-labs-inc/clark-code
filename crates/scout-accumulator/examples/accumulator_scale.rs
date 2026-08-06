use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use scout_accumulator::{
    plan_insert, prove_persistent, verify_proof, Accumulator, AccumulatorContext, AccumulatorHead,
    Digest, ProofStatus, StoredNode,
};
use serde::Serialize;

#[derive(Serialize)]
struct ScaleReceipt {
    schema_version: u16,
    status: &'static str,
    objects: usize,
    root: Digest,
    active_nodes: usize,
    node_writes: usize,
    average_node_writes: f64,
    max_nodes_touched: usize,
    max_proof_bytes: usize,
    incremental_insert_ms: u128,
    reverse_build_ms: u128,
    order_independent: bool,
    membership_and_nonmembership_verified: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Scout accumulator scale evaluation failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (objects, output) = arguments()?;
    let context =
        AccumulatorContext::new("clark.scout.enterprise-ledger", "scale-enterprise", "batch")
            .map_err(|error| error.to_string())?;
    let mut head = AccumulatorHead::empty(&context);
    let mut nodes = BTreeMap::<Digest, StoredNode>::new();
    let mut node_writes = 0_usize;
    let mut max_nodes_touched = 0_usize;
    let started = Instant::now();
    for index in 0..objects {
        let mutation = plan_insert(&context, head, object_id(index), |digest| {
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
                    return Err("content-addressed node collision".into());
                }
            }
        }
        for digest in mutation.obsolete_nodes {
            if nodes.remove(&digest).is_none() {
                return Err("obsolete path node is missing".into());
            }
        }
        head = mutation.next;
    }
    let incremental_insert_ms = started.elapsed().as_millis();
    let expected_nodes = objects.saturating_mul(2).saturating_sub(1);
    if head.root.count != objects as u64 || nodes.len() != expected_nodes {
        return Err("active accumulator shape has an unexpected size".into());
    }

    let started = Instant::now();
    let mut reverse = Accumulator::new(context.clone());
    for index in (0..objects).rev() {
        reverse
            .insert(object_id(index))
            .map_err(|error| error.to_string())?;
    }
    let reverse_build_ms = started.elapsed().as_millis();
    if reverse.root() != head.root {
        return Err("reverse insertion changed the accumulator root".into());
    }

    let mut max_proof_bytes = 0_usize;
    for (object_id, expected) in [
        (object_id(0), ProofStatus::Member),
        (object_id(objects / 2), ProofStatus::Member),
        (object_id(objects - 1), ProofStatus::Member),
        (format!("batch:{}", "f".repeat(64)), ProofStatus::NonMember),
    ] {
        let proof = prove_persistent(&context, head, object_id, |digest| {
            Ok(nodes.get(&digest).cloned())
        })
        .map_err(|error| error.to_string())?;
        if verify_proof(&head.root, &proof).map_err(|error| error.to_string())? != expected {
            return Err("proof returned the wrong membership status".into());
        }
        max_proof_bytes = max_proof_bytes.max(
            serde_json::to_vec(&proof)
                .map_err(|error| error.to_string())?
                .len(),
        );
    }
    let receipt = ScaleReceipt {
        schema_version: 1,
        status: "passed",
        objects,
        root: head.root.digest,
        active_nodes: nodes.len(),
        node_writes,
        average_node_writes: node_writes as f64 / objects as f64,
        max_nodes_touched,
        max_proof_bytes,
        incremental_insert_ms,
        reverse_build_ms,
        order_independent: true,
        membership_and_nonmembership_verified: true,
    };
    let bytes = serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?;
    if let Some(output) = output {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&output, &bytes).map_err(|error| error.to_string())?;
        println!("receipt={}", output.display());
    }
    println!("{}", String::from_utf8_lossy(&bytes));
    Ok(())
}

fn arguments() -> Result<(usize, Option<PathBuf>), String> {
    let mut objects = 100_000_usize;
    let mut output = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--objects" => {
                objects = arguments
                    .next()
                    .ok_or("--objects requires a value")?
                    .parse()
                    .map_err(|_| "--objects must be an integer")?;
            }
            "--out" => {
                output = Some(PathBuf::from(
                    arguments.next().ok_or("--out requires a path")?,
                ));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if !(1..=1_000_000).contains(&objects) {
        return Err("--objects must be in 1..=1000000".into());
    }
    Ok((objects, output))
}

fn object_id(index: usize) -> String {
    format!("batch:{index:064x}")
}
