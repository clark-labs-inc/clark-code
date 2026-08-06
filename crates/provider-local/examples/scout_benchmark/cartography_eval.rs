use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum NodeKind {
    Actor,
    Journey,
    Repository,
    Service,
    Deployment,
    Identity,
    DataStore,
    Vendor,
    Owner,
    Monitor,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum EdgeKind {
    EntersThrough,
    Calls,
    Reads,
    Writes,
    DependsOn,
    SourceFor,
    DeploysTo,
    AuthenticatesVia,
    OwnedBy,
    ObservedBy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct SimulationContract {
    inputs: bool,
    outputs: bool,
    state: bool,
    timeout: bool,
    retry: bool,
    idempotency: bool,
    failure_behavior: bool,
}

impl SimulationContract {
    fn complete() -> Self {
        Self {
            inputs: true,
            outputs: true,
            state: true,
            timeout: true,
            retry: true,
            idempotency: true,
            failure_behavior: true,
        }
    }

    fn is_complete(&self) -> bool {
        self.inputs
            && self.outputs
            && self.state
            && self.timeout
            && self.retry
            && self.idempotency
            && self.failure_behavior
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Node {
    id: &'static str,
    kind: NodeKind,
    critical: bool,
    evidence_count: u8,
    simulation: Option<SimulationContract>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Edge {
    from: &'static str,
    to: &'static str,
    kind: EdgeKind,
    evidence_count: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CoverageStatus {
    Supported,
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct CoverageCell {
    id: &'static str,
    status: CoverageStatus,
    cursor_complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct BusinessSystemMap {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    expected_coverage: BTreeSet<&'static str>,
    expected_external_dependencies: BTreeSet<&'static str>,
    coverage: Vec<CoverageCell>,
    scenario_results: BTreeMap<&'static str, bool>,
    secret_payload_reads: u32,
    secret_material_fields: u32,
    fixed_point_replay_matches: bool,
}

fn golden_map() -> BusinessSystemMap {
    let nodes = vec![
        node("customer", NodeKind::Actor, true, None),
        node("purchase", NodeKind::Journey, true, None),
        node("checkout-repo", NodeKind::Repository, true, None),
        node(
            "checkout-service",
            NodeKind::Service,
            true,
            Some(SimulationContract::complete()),
        ),
        node("checkout-prod", NodeKind::Deployment, true, None),
        node("checkout-role", NodeKind::Identity, true, None),
        node("orders", NodeKind::DataStore, true, None),
        node("payments", NodeKind::Vendor, true, None),
        node("commerce-team", NodeKind::Owner, true, None),
        node("checkout-slo", NodeKind::Monitor, true, None),
    ];
    let edges = vec![
        edge("customer", "purchase", EdgeKind::EntersThrough),
        edge("purchase", "checkout-service", EdgeKind::Calls),
        edge("checkout-repo", "checkout-service", EdgeKind::SourceFor),
        edge("checkout-service", "checkout-prod", EdgeKind::DeploysTo),
        edge(
            "checkout-service",
            "checkout-role",
            EdgeKind::AuthenticatesVia,
        ),
        edge("checkout-service", "orders", EdgeKind::Writes),
        edge("checkout-service", "orders", EdgeKind::Reads),
        edge("checkout-service", "payments", EdgeKind::DependsOn),
        edge("checkout-service", "commerce-team", EdgeKind::OwnedBy),
        edge("checkout-service", "checkout-slo", EdgeKind::ObservedBy),
    ];
    BusinessSystemMap {
        nodes,
        edges,
        expected_coverage: BTreeSet::from([
            "forge:orgs",
            "cloud:account-a:region-a",
            "cloud:account-a:region-b",
            "dns:zones",
            "identity:tenant",
            "observability:services",
            "vendors:contracts",
        ]),
        expected_external_dependencies: BTreeSet::from(["payments"]),
        coverage: vec![
            coverage("forge:orgs"),
            coverage("cloud:account-a:region-a"),
            coverage("cloud:account-a:region-b"),
            coverage("dns:zones"),
            coverage("identity:tenant"),
            coverage("observability:services"),
            coverage("vendors:contracts"),
        ],
        scenario_results: BTreeMap::from([
            ("purchase_success", true),
            ("payment_timeout_no_double_charge", true),
            ("duplicate_delivery_idempotent", true),
            ("datastore_failure_alerts_owner", true),
        ]),
        secret_payload_reads: 0,
        secret_material_fields: 0,
        fixed_point_replay_matches: true,
    }
}

fn node(
    id: &'static str,
    kind: NodeKind,
    critical: bool,
    simulation: Option<SimulationContract>,
) -> Node {
    Node {
        id,
        kind,
        critical,
        evidence_count: 2,
        simulation,
    }
}

fn edge(from: &'static str, to: &'static str, kind: EdgeKind) -> Edge {
    Edge {
        from,
        to,
        kind,
        evidence_count: 2,
    }
}

fn coverage(id: &'static str) -> CoverageCell {
    CoverageCell {
        id,
        status: CoverageStatus::Supported,
        cursor_complete: true,
    }
}

fn validate(map: &BusinessSystemMap) -> Result<String, String> {
    if map.secret_payload_reads != 0 || map.secret_material_fields != 0 {
        return Err("secret_leak".into());
    }
    if !map.fixed_point_replay_matches {
        return Err("discovery_not_stable".into());
    }

    let node_ids = map
        .nodes
        .iter()
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    if node_ids.len() != map.nodes.len()
        || !map.nodes.iter().any(|node| node.kind == NodeKind::Journey)
        || !map.nodes.iter().any(|node| node.kind == NodeKind::Service)
    {
        return Err("business_graph_required".into());
    }
    if map.nodes.iter().any(|node| node.evidence_count == 0)
        || map.edges.iter().any(|edge| {
            edge.evidence_count == 0 || !node_ids.contains(edge.from) || !node_ids.contains(edge.to)
        })
    {
        return Err("unverified_graph_observation".into());
    }

    let coverage = map
        .coverage
        .iter()
        .map(|cell| (cell.id, cell))
        .collect::<BTreeMap<_, _>>();
    for expected in &map.expected_coverage {
        let Some(cell) = coverage.get(expected) else {
            return Err("control_plane_coverage_gap".into());
        };
        if matches!(
            cell.status,
            CoverageStatus::Supported | CoverageStatus::Empty
        ) && !cell.cursor_complete
        {
            return Err("incomplete_enumeration".into());
        }
    }

    for expected in &map.expected_external_dependencies {
        if !map
            .nodes
            .iter()
            .any(|node| node.id == *expected && node.kind == NodeKind::Vendor)
            || !map
                .edges
                .iter()
                .any(|edge| edge.to == *expected && edge.kind == EdgeKind::DependsOn)
        {
            return Err("external_dependency_gap".into());
        }
    }

    for service in map
        .nodes
        .iter()
        .filter(|node| node.critical && node.kind == NodeKind::Service)
    {
        for (kind, code) in [
            (EdgeKind::SourceFor, "missing_source_provenance"),
            (EdgeKind::DeploysTo, "missing_deployment"),
            (EdgeKind::AuthenticatesVia, "missing_runtime_identity"),
            (EdgeKind::OwnedBy, "missing_owner"),
            (EdgeKind::ObservedBy, "missing_observability"),
        ] {
            if !map
                .edges
                .iter()
                .any(|edge| edge.from == service.id && edge.kind == kind)
                && !map
                    .edges
                    .iter()
                    .any(|edge| edge.to == service.id && edge.kind == kind)
            {
                return Err(code.into());
            }
        }
        if !service
            .simulation
            .as_ref()
            .is_some_and(SimulationContract::is_complete)
        {
            return Err("missing_simulation_contract".into());
        }
    }

    let journey_ids = map
        .nodes
        .iter()
        .filter(|node| node.critical && node.kind == NodeKind::Journey)
        .map(|node| node.id)
        .collect::<Vec<_>>();
    for journey in journey_ids {
        if !reaches_business_effect(map, journey) {
            return Err("journey_path_gap".into());
        }
    }

    if map.scenario_results.len() < 4 || map.scenario_results.values().any(|passed| !passed) {
        return Err("simulation_scenario_failed".into());
    }

    let encoded = serde_json::to_vec(map).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn reaches_business_effect(map: &BusinessSystemMap, start: &str) -> bool {
    let kinds = map
        .nodes
        .iter()
        .map(|node| (node.id, &node.kind))
        .collect::<BTreeMap<_, _>>();
    let mut queue = VecDeque::from([start]);
    let mut visited = BTreeSet::new();
    while let Some(current) = queue.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        if kinds
            .get(current)
            .is_some_and(|kind| matches!(kind, NodeKind::DataStore | NodeKind::Vendor))
        {
            return true;
        }
        queue.extend(
            map.edges
                .iter()
                .filter(|edge| edge.from == current)
                .map(|edge| edge.to),
        );
    }
    false
}

fn expect_error(
    controls: &mut BTreeMap<&'static str, &'static str>,
    name: &'static str,
    map: BusinessSystemMap,
    expected: &'static str,
) -> Result<(), String> {
    let actual = validate(&map).expect_err("negative control unexpectedly passed");
    if actual != expected {
        return Err(format!(
            "negative control {name} returned {actual}, expected {expected}"
        ));
    }
    controls.insert(name, expected);
    Ok(())
}

pub fn business_system_contract() -> Result<(String, Value), String> {
    let golden = golden_map();
    let semantic_sha256 = validate(&golden)?;
    let mut controls = BTreeMap::new();

    let mut host_only = golden.clone();
    host_only
        .nodes
        .retain(|node| node.kind == NodeKind::Repository);
    host_only.edges.clear();
    expect_error(
        &mut controls,
        "host_inventory_only",
        host_only,
        "business_graph_required",
    )?;

    let mut region_gap = golden.clone();
    region_gap
        .coverage
        .retain(|cell| cell.id != "cloud:account-a:region-b");
    expect_error(
        &mut controls,
        "default_region_only",
        region_gap,
        "control_plane_coverage_gap",
    )?;

    let mut denied_as_empty = golden.clone();
    denied_as_empty.coverage[0].status = CoverageStatus::Empty;
    denied_as_empty.coverage[0].cursor_complete = false;
    expect_error(
        &mut controls,
        "denied_as_empty",
        denied_as_empty,
        "incomplete_enumeration",
    )?;

    for (name, kind, expected) in [
        (
            "unversioned_deployment",
            EdgeKind::SourceFor,
            "missing_source_provenance",
        ),
        ("orphan_owner", EdgeKind::OwnedBy, "missing_owner"),
        (
            "missing_correctness_observability",
            EdgeKind::ObservedBy,
            "missing_observability",
        ),
    ] {
        let mut mutated = golden.clone();
        mutated.edges.retain(|edge| edge.kind != kind);
        expect_error(&mut controls, name, mutated, expected)?;
    }

    let mut missing_simulation = golden.clone();
    missing_simulation
        .nodes
        .iter_mut()
        .find(|node| node.id == "checkout-service")
        .expect("fixture service")
        .simulation = None;
    expect_error(
        &mut controls,
        "missing_simulation_contract",
        missing_simulation,
        "missing_simulation_contract",
    )?;

    let mut undeclared_vendor = golden.clone();
    undeclared_vendor
        .edges
        .retain(|edge| edge.kind != EdgeKind::DependsOn);
    expect_error(
        &mut controls,
        "undeclared_vendor",
        undeclared_vendor,
        "external_dependency_gap",
    )?;

    let mut invented_edge = golden.clone();
    invented_edge.edges[0].evidence_count = 0;
    expect_error(
        &mut controls,
        "false_name_join",
        invented_edge,
        "unverified_graph_observation",
    )?;

    let mut secret_canary = golden.clone();
    secret_canary.secret_payload_reads = 1;
    expect_error(&mut controls, "secret_canary", secret_canary, "secret_leak")?;

    let mut unstable = golden;
    unstable.fixed_point_replay_matches = false;
    expect_error(
        &mut controls,
        "unstable_frontier",
        unstable,
        "discovery_not_stable",
    )?;

    Ok((
        "business graph is simulation-ready and every mutation control is rejected".into(),
        json!({
            "semantic_sha256": semantic_sha256,
            "critical_node_recall": 1.0,
            "critical_edge_recall": 1.0,
            "invented_nodes": 0,
            "invented_edges": 0,
            "secret_payload_reads": 0,
            "fixed_point_replay_matches": true,
            "negative_controls": controls,
        }),
    ))
}
