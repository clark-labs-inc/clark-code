use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use scout_accumulator::{
    AccumulatorContext, Digest, PartitionedAccumulatorEditor, PartitionedAccumulatorHead,
    StoredNode, DEFAULT_PARTITION_BITS,
};
use serde::Serialize;

type NodeStore = BTreeMap<(u16, Digest), StoredNode>;

#[derive(Serialize)]
struct ScaleReceipt {
    schema_version: u16,
    status: &'static str,
    objects: usize,
    partition_bits: u8,
    nonempty_partitions: usize,
    root: Digest,
    active_nodes: usize,
    node_writes: usize,
    average_node_writes: f64,
    max_nodes_touched: usize,
    manifest_bytes: usize,
    forward_build_ms: u128,
    reverse_build_ms: u128,
    order_independent: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Scout partitioned accumulator scale evaluation failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (objects, partition_bits, output) = arguments()?;
    let context =
        AccumulatorContext::new("clark.scout.enterprise-ledger", "scale-enterprise", "event")
            .map_err(|error| error.to_string())?;
    let started = Instant::now();
    let (head, nodes, node_writes, max_nodes_touched) =
        build(context.clone(), partition_bits, 0..objects)?;
    let forward_build_ms = started.elapsed().as_millis();
    let expected_nodes = objects
        .saturating_mul(2)
        .saturating_sub(head.partitions().len());
    if head.root.count != objects as u64 || nodes.len() != expected_nodes {
        return Err("active partitioned accumulator shape has an unexpected size".into());
    }

    let started = Instant::now();
    let (reverse, _, _, _) = build(context, partition_bits, (0..objects).rev())?;
    let reverse_build_ms = started.elapsed().as_millis();
    if reverse.root != head.root || reverse.partitions() != head.partitions() {
        return Err("reverse insertion changed the partitioned accumulator root".into());
    }

    let manifest_bytes = serde_json::to_vec(&head)
        .map_err(|error| error.to_string())?
        .len();
    let receipt = ScaleReceipt {
        schema_version: 1,
        status: "passed",
        objects,
        partition_bits,
        nonempty_partitions: head.partitions().len(),
        root: head.root.digest,
        active_nodes: nodes.len(),
        node_writes,
        average_node_writes: node_writes as f64 / objects as f64,
        max_nodes_touched,
        manifest_bytes,
        forward_build_ms,
        reverse_build_ms,
        order_independent: true,
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

fn build(
    context: AccumulatorContext,
    partition_bits: u8,
    indexes: impl IntoIterator<Item = usize>,
) -> Result<(PartitionedAccumulatorHead, NodeStore, usize, usize), String> {
    let head = PartitionedAccumulatorHead::empty(context, partition_bits)
        .map_err(|error| error.to_string())?;
    let mut editor = PartitionedAccumulatorEditor::new(head).map_err(|error| error.to_string())?;
    let mut nodes = NodeStore::new();
    let mut node_writes = 0_usize;
    let mut max_nodes_touched = 0_usize;
    for index in indexes {
        let mutation = editor
            .insert(object_id(index), |partition, digest| {
                Ok(nodes.get(&(partition, digest)).cloned())
            })
            .map_err(|error| error.to_string())?;
        let partition_context = editor
            .head()
            .partition_context(mutation.partition)
            .map_err(|error| error.to_string())?;
        node_writes = node_writes.saturating_add(mutation.nodes.len());
        max_nodes_touched = max_nodes_touched.max(
            mutation
                .nodes
                .len()
                .saturating_add(mutation.obsolete_nodes.len()),
        );
        for node in &mutation.nodes {
            let digest = node
                .digest(&partition_context)
                .map_err(|error| error.to_string())?;
            if let Some(existing) = nodes.insert((mutation.partition, digest), node.clone()) {
                if existing != *node {
                    return Err("content-addressed partition node collision".into());
                }
            }
        }
        for digest in &mutation.obsolete_nodes {
            if nodes.remove(&(mutation.partition, *digest)).is_none() {
                return Err("obsolete partition path node is missing".into());
            }
        }
        if mutation.next_root != editor.head().root {
            return Err("partitioned update receipt does not match the editor head".into());
        }
    }
    Ok((editor.into_head(), nodes, node_writes, max_nodes_touched))
}

fn arguments() -> Result<(usize, u8, Option<PathBuf>), String> {
    let mut objects = 100_000_usize;
    let mut partition_bits = DEFAULT_PARTITION_BITS;
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
            "--partition-bits" => {
                partition_bits = arguments
                    .next()
                    .ok_or("--partition-bits requires a value")?
                    .parse()
                    .map_err(|_| "--partition-bits must be an integer")?;
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
    Ok((objects, partition_bits, output))
}

fn object_id(index: usize) -> String {
    format!("event:{index:064x}")
}
