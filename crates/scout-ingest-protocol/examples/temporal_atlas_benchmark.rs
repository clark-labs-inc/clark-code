use std::collections::BTreeSet;
use std::time::Instant;

use scout_ingest_protocol::cartography::{
    GraphObjectKind, GraphSnapshotRef, PublishSimulationOverlay, SimulationCoverageState,
    SimulationMembership, SimulationObjectRef, SimulationOverlayStatus, SimulationResultState,
    MAX_SIMULATION_MEMBERSHIPS_PER_PUBLISH,
};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service_count = std::env::args()
        .nth(1)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(10_000);
    let expected_memberships = service_count
        .checked_mul(6)
        .ok_or("membership count overflow")?;
    if expected_memberships > MAX_SIMULATION_MEMBERSHIPS_PER_PUBLISH {
        return Err(format!(
            "{service_count} services require {expected_memberships} memberships, above the contract limit {MAX_SIMULATION_MEMBERSHIPS_PER_PUBLISH}"
        )
        .into());
    }

    let build_started = Instant::now();
    let organization_id = Uuid::from_u128(1);
    let workspace_id = Uuid::from_u128(2);
    let mut memberships = Vec::with_capacity(expected_memberships);
    for service in 0..service_count {
        for lane in 0..6 {
            let digest = hex_lower(&Sha256::digest(format!("service:{service}:lane:{lane}")));
            memberships.push(SimulationMembership {
                object: SimulationObjectRef {
                    object_kind: if lane == 5 {
                        GraphObjectKind::Edge
                    } else {
                        GraphObjectKind::Entity
                    },
                    object_id: format!("{}:{digest}", if lane == 5 { "edge" } else { "entity" }),
                },
                coverage: if lane < 4 {
                    SimulationCoverageState::Covered
                } else if lane == 4 {
                    SimulationCoverageState::Partial
                } else {
                    SimulationCoverageState::OutsideContract
                },
                result: SimulationResultState::NotRun,
                confidence_basis_points: if lane < 4 { 10_000 } else { 7_500 },
                rationale: format!("synthetic deterministic lane {lane}"),
                evidence_event_ids: BTreeSet::new(),
            });
        }
    }
    let overlay = PublishSimulationOverlay {
        stable_key: "benchmark.enterprise-temporal-atlas".into(),
        name: "Enterprise Temporal Atlas benchmark".into(),
        status: SimulationOverlayStatus::Ready,
        snapshot: GraphSnapshotRef {
            organization_id,
            workspace_id,
            effective_at_ms: 1_700_000_000_000,
            known_at_ms: 1_700_000_001_000,
            filter_sha256: "ab".repeat(32),
        },
        memberships,
        summary: json!({
            "service_count": service_count,
            "membership_count": expected_memberships,
        }),
    };
    let build_ms = build_started.elapsed().as_millis();

    let serialize_started = Instant::now();
    let bytes = serde_json::to_vec(&overlay)?;
    let serialize_ms = serialize_started.elapsed().as_millis();
    let semantic_sha256 = hex_lower(&Sha256::digest(&bytes));

    let deserialize_started = Instant::now();
    let decoded: PublishSimulationOverlay = serde_json::from_slice(&bytes)?;
    let deserialize_ms = deserialize_started.elapsed().as_millis();
    if decoded != overlay {
        return Err("simulation overlay wire round-trip changed content".into());
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "clark-temporal-atlas-benchmark-v1",
            "service_count": service_count,
            "membership_count": expected_memberships,
            "serialized_bytes": bytes.len(),
            "semantic_sha256": semantic_sha256,
            "build_ms": build_ms,
            "serialize_ms": serialize_ms,
            "deserialize_ms": deserialize_ms,
            "platform": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "values_observed": false,
            "live_model_calls": 0,
        }))?
    );
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
