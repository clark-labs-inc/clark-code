use crate::model::{Evidence, EvidenceRole, EvidenceSource, RetrievalReceipt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

mod proxy;

use proxy::{forward, read_request, Request};

const ORG_ID: &str = "11111111-1111-4111-8111-111111111111";
const WORKSPACE_ID: &str = "22222222-2222-4222-8222-222222222222";
const MACHINE_ID: &str = "33333333-3333-4333-8333-333333333333";

pub struct Gateway {
    pub base_url: String,
    receipts: Arc<Mutex<Vec<RetrievalReceipt>>>,
    task: JoinHandle<()>,
}

impl Gateway {
    pub async fn start(
        upstream_base_url: &str,
        api_key: &str,
        evidence: &[&Evidence],
    ) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let client = clark_http::build_client(clark_http::ClientOptions {
            request_timeout: Some(std::time::Duration::from_secs(7 * 60)),
            ..Default::default()
        })
        .map_err(|error| format!("planning eval gateway client failed: {error}"))?;
        let state = Arc::new(State {
            upstream_base_url: upstream_base_url.to_string(),
            api_key: api_key.to_string(),
            evidence: evidence.iter().map(|item| (*item).clone()).collect(),
            receipts: Arc::new(Mutex::new(Vec::new())),
            client,
        });
        let receipts = state.receipts.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(error) = serve(stream, state).await {
                        eprintln!("planning_eval gateway request failed: {error}");
                    }
                });
            }
        });
        Ok(Self {
            base_url: format!("http://{address}/v1"),
            receipts,
            task,
        })
    }

    pub fn receipts(&self) -> Vec<RetrievalReceipt> {
        self.receipts.lock().expect("gateway receipt lock").clone()
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct State {
    upstream_base_url: String,
    api_key: String,
    evidence: Vec<Evidence>,
    receipts: Arc<Mutex<Vec<RetrievalReceipt>>>,
    client: reqwest::Client,
}

async fn serve(mut stream: TcpStream, state: Arc<State>) -> Result<(), String> {
    let request = read_request(&mut stream).await?;
    let started = Instant::now();
    let path = request.target.split('?').next().unwrap_or(&request.target);
    let (status, content_type, response_headers, body) = match path {
        "/v1/memories" => (
            200,
            "application/json",
            Vec::new(),
            serde_json::to_vec(&json!({"data": []})).unwrap(),
        ),
        "/v1/organization-knowledge/search" => {
            let query = query_value(&request.target, "query").unwrap_or_default();
            let ids = state
                .evidence
                .iter()
                .filter(|item| item.source == EvidenceSource::Org)
                .map(|item| item.id.to_string())
                .collect::<Vec<_>>();
            let body = serde_json::to_vec(&organization_response(&state.evidence, &query)).unwrap();
            record(
                &state,
                started,
                RetrievalRecord {
                    source: "org",
                    operation: "organization_knowledge.search",
                    query: Some(query),
                    returned_evidence_ids: ids,
                    request: &request,
                    response_status: 200,
                    response_body: &body,
                },
            );
            (200, "application/json", Vec::new(), body)
        }
        "/v1/system-cartography/machines/enroll" => {
            let request_body: Value =
                serde_json::from_slice(&request.body).map_err(|error| error.to_string())?;
            let public_key = request_body["public_key"]
                .as_str()
                .ok_or("Scout enrollment omitted public_key")?;
            let public_bytes = decode_hex(public_key)?;
            let signer_id = format!("signer:{:x}", Sha256::digest(public_bytes));
            let body = serde_json::to_vec(&json!({
                "id": MACHINE_ID,
                "organization_id": request_body["organization_id"],
                "workspace_id": request_body["workspace_id"],
                "signer_id": signer_id,
                "public_key": public_key.to_ascii_lowercase(),
                "platform": request_body["platform"],
                "architecture": request_body["architecture"],
                "coordinator_public_key": "ab".repeat(32)
            }))
            .unwrap();
            record(
                &state,
                started,
                RetrievalRecord {
                    source: "scout",
                    operation: "machines.enroll",
                    query: None,
                    returned_evidence_ids: Vec::new(),
                    request: &request,
                    response_status: 200,
                    response_body: &body,
                },
            );
            (200, "application/json", Vec::new(), body)
        }
        "/v1/system-cartography/snapshots/query" => {
            let request_body: Value =
                serde_json::from_slice(&request.body).map_err(|error| error.to_string())?;
            let ids = state
                .evidence
                .iter()
                .filter(|item| item.source == EvidenceSource::Scout)
                .map(|item| item.id.to_string())
                .collect::<Vec<_>>();
            let body =
                serde_json::to_vec(&snapshot_response(&state.evidence, &request_body)).unwrap();
            record(
                &state,
                started,
                RetrievalRecord {
                    source: "scout",
                    operation: "snapshots.query",
                    query: Some(request_body.to_string()),
                    returned_evidence_ids: ids,
                    request: &request,
                    response_status: 200,
                    response_body: &body,
                },
            );
            (200, "application/json", Vec::new(), body)
        }
        repository_path
            if repository_path.starts_with("/v1/code/repositories/")
                && repository_path.ends_with("/context") =>
        {
            let query = query_value(&request.target, "q").unwrap_or_default();
            let body = serde_json::to_vec(&json!({
                "fingerprint": "benchmark",
                "canonical_remote": "https://example.invalid/benchmark.git",
                "current_branch": "main",
                "default_branch": "main",
                "commits": [
                    {
                        "oid": "1a".repeat(20),
                        "author_name": "Migration Working Group",
                        "committed_at": "2026-06-14T18:20:00Z",
                        "subject": "Preserve compatibility at producer boundaries",
                        "body": "Rollouts keep the legacy entrypoint until every observed consumer is dual-read capable."
                    },
                    {
                        "oid": "2b".repeat(20),
                        "author_name": "Reliability",
                        "committed_at": "2026-07-02T09:10:00Z",
                        "subject": "Make rollback order executable",
                        "body": "Operational ordering belongs in versioned configuration and must be asserted by tests."
                    }
                ]
            }))
            .unwrap();
            record(
                &state,
                started,
                RetrievalRecord {
                    source: "repository",
                    operation: "code.repositories.context",
                    query: Some(query),
                    returned_evidence_ids: vec![
                        "REPO-COMMIT-COMPAT".into(),
                        "REPO-COMMIT-ROLLBACK".into(),
                    ],
                    request: &request,
                    response_status: 200,
                    response_body: &body,
                },
            );
            (200, "application/json", Vec::new(), body)
        }
        _ => {
            let response = forward(
                &request,
                &state.client,
                &state.upstream_base_url,
                &state.api_key,
            )
            .await?;
            (
                response.status,
                response.content_type,
                response.headers,
                response.body,
            )
        }
    };
    let reason = match status {
        200..=299 => "OK",
        429 => "Too Many Requests",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    let mut header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in response_headers {
        header.push_str(&format!("{name}: {value}\r\n"));
    }
    header.push_str("\r\n");
    stream
        .write_all(header.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(&body)
        .await
        .map_err(|error| error.to_string())
}

fn organization_response(evidence: &[Evidence], query: &str) -> Value {
    let hits = evidence
        .iter()
        .filter(|item| item.source == EvidenceSource::Org)
        .map(|item| {
            json!({
                "claim_id": item.id,
                "subject": "benchmark-system",
                "predicate": "governing_decision",
                "object": item.text,
                "fact_kind": "decision",
                "confidence": if item.role == EvidenceRole::Required { 0.98 } else { 0.72 },
                "status": if item.role == EvidenceRole::Stale { "superseded" } else { "current" },
                "valid_from": if item.role == EvidenceRole::Stale { "2024-01-01T00:00:00Z" } else { "2026-06-01T00:00:00Z" },
                "valid_to": if item.role == EvidenceRole::Stale { Value::String("2025-12-31T23:59:59Z".into()) } else { Value::Null },
                "observed_at": "2026-07-20T12:00:00Z",
                "source_kind": "decision_record",
                "source_display_name": "Clark benchmark organization memory",
                "evidence_locator": format!("org://decisions/{}", item.id),
                "evidence_excerpt": item.text
            })
        })
        .collect::<Vec<_>>();
    json!({
        "query": query,
        "organizations": [{"organization_id": ORG_ID, "query": query, "hits": hits}]
    })
}

fn snapshot_response(evidence: &[Evidence], query: &Value) -> Value {
    let effective = query["effective_at_ms"]
        .as_u64()
        .unwrap_or(1_784_000_000_000);
    let known = query["known_at_ms"].as_u64().unwrap_or(1_784_000_100_000);
    let mut sequence = 1_i64;
    let mut entries = topology_entries(evidence, effective, known, &mut sequence);
    entries.extend(evidence
        .iter()
        .filter(|item| item.source == EvidenceSource::Scout)
        .map(|item| {
            let current = sequence;
            sequence += 1;
            json!({
                "object_kind": "claim",
                "object_id": format!("claim:{}", &format!("{:x}", Sha256::digest(item.id.as_bytes()))),
                "run_id": "44444444-4444-4444-8444-444444444444",
                "machine_id": MACHINE_ID,
                "accepted_at_ms": known,
                "event": event(item.id, item.text, current, effective, "claim")
            })
        })
        .collect::<Vec<_>>());
    let coverage = if evidence.iter().any(|item| item.id == "SCOUT-AUDIT-GRAPH") {
        ("staging-eu", "unreachable", false)
    } else {
        ("production-inventory", "supported", true)
    };
    entries.push(json!({
        "object_kind": "coverage",
        "object_id": format!("coverage:{}", coverage.0),
        "run_id": "44444444-4444-4444-8444-444444444444",
        "machine_id": MACHINE_ID,
        "accepted_at_ms": known,
        "event": coverage_event(coverage.0, coverage.1, coverage.2, sequence, effective)
    }));
    json!({
        "organization_id": ORG_ID,
        "workspace_id": WORKSPACE_ID,
        "effective_at_ms": effective,
        "known_at_ms": known,
        "entries": entries,
        "next_cursor": Value::Null
    })
}

fn topology_entries(
    evidence: &[Evidence],
    effective: u64,
    known: u64,
    sequence: &mut i64,
) -> Vec<Value> {
    let scout_id = evidence
        .iter()
        .find(|item| item.source == EvidenceSource::Scout && item.role == EvidenceRole::Required)
        .map(|item| item.id)
        .unwrap_or("SCOUT-PREF-GRAPH");
    let topology = match scout_id {
        "SCOUT-AUDIT-GRAPH" => (
            ["audit-api", "audit-worker", "regional-storage", "audit-web"],
            [
                ("audit-api", "audit-worker", "enqueues_to"),
                ("audit-worker", "regional-storage", "writes_to"),
                ("audit-web", "audit-api", "polls"),
            ],
        ),
        "SCOUT-EVENT-GRAPH" => (
            ["event-producer", "billing", "analytics", "notifications"],
            [
                ("event-producer", "billing", "delivers_to"),
                ("event-producer", "analytics", "delivers_to"),
                ("event-producer", "notifications", "delivers_to"),
            ],
        ),
        "SCOUT-OAUTH-GRAPH" => (
            [
                "auth-signer",
                "gateway-verifier",
                "worker-verifier",
                "auth-metrics",
            ],
            [
                ("auth-signer", "gateway-verifier", "issues_for"),
                ("auth-signer", "worker-verifier", "issues_for"),
                ("gateway-verifier", "auth-metrics", "reports_to"),
            ],
        ),
        "SCOUT-WEBHOOK-GRAPH" => (
            [
                "webhook-receiver",
                "idempotency-ledger",
                "billing",
                "notifications",
            ],
            [
                ("webhook-receiver", "idempotency-ledger", "claims_in"),
                ("webhook-receiver", "billing", "delivers_to"),
                ("webhook-receiver", "notifications", "delivers_to"),
            ],
        ),
        "SCOUT-SEARCH-GRAPH" => (
            [
                "search-writer",
                "search-backfill",
                "api-reader",
                "search-indexes",
            ],
            [
                ("search-writer", "search-indexes", "writes_to"),
                ("search-backfill", "search-indexes", "copies_to"),
                ("api-reader", "search-indexes", "reads_from"),
            ],
        ),
        "SCOUT-CACHE-GRAPH" => (
            [
                "policy-publisher",
                "gateway-subscriber",
                "worker-subscriber",
                "policy-metrics",
            ],
            [
                ("policy-publisher", "gateway-subscriber", "delivers_to"),
                ("policy-publisher", "worker-subscriber", "delivers_to"),
                ("gateway-subscriber", "policy-metrics", "reports_to"),
            ],
        ),
        "SCOUT-RETENTION-GRAPH" => (
            [
                "retention-classifier",
                "legal-hold-api",
                "delete-guard",
                "artifact-sweeper",
            ],
            [
                ("retention-classifier", "legal-hold-api", "classifies_for"),
                ("legal-hold-api", "delete-guard", "protects"),
                ("delete-guard", "artifact-sweeper", "gates"),
            ],
        ),
        "SCOUT-SYNC-GRAPH" => (
            [
                "mobile-queue",
                "sync-api",
                "conflict-resolver",
                "sync-metrics",
            ],
            [
                ("mobile-queue", "sync-api", "syncs_to"),
                ("sync-api", "conflict-resolver", "resolves_with"),
                ("conflict-resolver", "sync-metrics", "reports_to"),
            ],
        ),
        "SCOUT-FLAG-GRAPH" => (
            ["flag-admin", "flag-evaluator", "flag-sdk", "flag-metrics"],
            [
                ("flag-admin", "flag-evaluator", "writes_to"),
                ("flag-sdk", "flag-evaluator", "reads_from"),
                ("flag-evaluator", "flag-metrics", "reports_to"),
            ],
        ),
        "SCOUT-SHARD-GRAPH" => (
            [
                "tenant-api",
                "shard-router",
                "rebalance-worker",
                "shard-metrics",
            ],
            [
                ("tenant-api", "shard-router", "reads_through"),
                ("rebalance-worker", "shard-router", "updates"),
                ("shard-router", "shard-metrics", "reports_to"),
            ],
        ),
        "SCOUT-TEMPLATE-GRAPH" => (
            ["template-registry", "template-renderer", "email", "push"],
            [
                ("template-registry", "template-renderer", "serves"),
                ("template-renderer", "email", "renders_for"),
                ("template-renderer", "push", "renders_for"),
            ],
        ),
        _ => (
            ["preference-core", "cloud", "desktop", "mobile"],
            [
                ("preference-core", "cloud", "persists_to"),
                ("preference-core", "desktop", "migrates_on"),
                ("preference-core", "mobile", "configures"),
            ],
        ),
    };
    let mut entries = Vec::new();
    for id in topology.0 {
        let current = *sequence;
        *sequence += 1;
        entries.push(graph_entry(
            "entity",
            id,
            entity_event(id, current, effective),
            known,
        ));
    }
    for (source, target, kind) in topology.1 {
        let current = *sequence;
        *sequence += 1;
        let id = format!("{source}:{kind}:{target}");
        entries.push(graph_entry(
            "edge",
            &id,
            edge_event(source, target, kind, current, effective),
            known,
        ));
    }
    entries
}

fn graph_entry(kind: &str, id: &str, event: Value, known: u64) -> Value {
    json!({
        "object_kind": kind,
        "object_id": format!("{kind}:{:x}", Sha256::digest(id.as_bytes())),
        "run_id": "44444444-4444-4444-8444-444444444444",
        "machine_id": MACHINE_ID,
        "accepted_at_ms": known,
        "event": event
    })
}

fn entity_event(id: &str, sequence: i64, at: u64) -> Value {
    let mut value = event(
        &format!("SCOUT-ENTITY-{id}"),
        "Stable deployed service identity",
        sequence,
        at,
        "entity",
    );
    value["fact"]["subject"] = json!({
        "type": "entity",
        "entity": entity_identity(id)
    });
    value["fact"]["attributes"] = json!({
        "environment": "production",
        "lifecycle": "active",
        "effective_at_ms": at,
        "known_at_ms": at + 100_000
    });
    value
}

fn edge_event(source: &str, target: &str, kind: &str, sequence: i64, at: u64) -> Value {
    let mut value = event(
        &format!("SCOUT-EDGE-{source}-{target}"),
        "Observed production dependency",
        sequence,
        at,
        "edge",
    );
    value["fact"]["subject"] = json!({
        "type": "edge",
        "edge": {
            "edge_kind": kind,
            "source": entity_identity(source),
            "target": entity_identity(target),
            "qualifier": "production"
        }
    });
    value["fact"]["attributes"] = json!({
        "observation_count": 12,
        "effective_at_ms": at,
        "known_at_ms": at + 100_000
    });
    value
}

fn entity_identity(id: &str) -> Value {
    json!({
        "entity_kind": "runtime.service",
        "provider_namespace": "benchmark",
        "authority_scope": "organization/production",
        "provider_native_id": id
    })
}

fn event(id: &str, text: &str, sequence: i64, at: u64, kind: &str) -> Value {
    json!({
        "event_id": format!("event:{}", "ab".repeat(32)),
        "source_id": "55555555-5555-4555-8555-555555555555",
        "task_id": "66666666-6666-4666-8666-666666666666",
        "fence": 1,
        "source_sequence": sequence,
        "observed_at_ms": at,
        "classification": "internal",
        "evidence": {"evidence_id": id, "bucket": "benchmark", "key": format!("scout/{id}.json"), "sha256": "cd".repeat(32), "size_bytes": text.len(), "version_id": "v1"},
        "fact": {
            "subject": {"type":"claim","claim":{"claim_kind":"deployment_observation","target":{"type":"entity","entity":{"entity_kind":"service","provider_namespace":"benchmark","authority_scope":"production","provider_native_id":"system"}},"predicate":kind}},
            "attributes": {"evidence_id":id,"statement":text,"effective_at_ms":at,"known_at_ms":at+100000,"confidence":0.96},
            "evidence_digests": ["cd".repeat(32)]
        }
    })
}

fn coverage_event(key: &str, disposition: &str, complete: bool, sequence: i64, at: u64) -> Value {
    let mut value = event(
        "SCOUT-COVERAGE",
        "Authoritative coverage boundary",
        sequence,
        at,
        "coverage",
    );
    value["fact"]["subject"] = json!({
        "type":"coverage",
        "coverage_key":key,
        "disposition":disposition,
        "complete":complete,
        "continuation_handle":Value::Null
    });
    value
}

fn query_value(target: &str, key: &str) -> Option<String> {
    let url = reqwest::Url::parse(&format!("http://benchmark{target}")).ok()?;
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

struct RetrievalRecord<'a> {
    source: &'a str,
    operation: &'a str,
    query: Option<String>,
    returned_evidence_ids: Vec<String>,
    request: &'a Request,
    response_status: u16,
    response_body: &'a [u8],
}

fn record(state: &State, started: Instant, record: RetrievalRecord<'_>) {
    let request_body = String::from_utf8_lossy(&record.request.body).into_owned();
    let response_body_text = String::from_utf8_lossy(record.response_body).into_owned();
    state
        .receipts
        .lock()
        .expect("gateway receipt lock")
        .push(RetrievalReceipt {
            source: record.source.into(),
            operation: record.operation.into(),
            query: record.query,
            request_method: record.request.method.clone(),
            request_target: record.request.target.clone(),
            request_sha256: sha256_bytes(&record.request.body),
            request_body,
            response_status: record.response_status,
            response_sha256: sha256_bytes(record.response_body),
            response_body: response_body_text,
            returned_evidence_ids: record.returned_evidence_ids,
            status: "ok".into(),
            elapsed_ms: started.elapsed().as_millis(),
        });
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if value.len() % 2 != 0 {
        return Err("invalid public key".into());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
            u8::from_str_radix(text, 16).map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scout_ingest_protocol::cartography::{
        GraphObjectKind, GraphSnapshotQuery, MAX_GRAPH_SNAPSHOT_LIMIT,
    };
    use scout_platform_client::{
        enroll_machine, ClarkCartographyEnrollmentConfig, MachineEnrollmentRequest,
    };
    use std::collections::BTreeSet;
    use uuid::Uuid;

    #[test]
    fn organization_packets_preserve_provenance() {
        let evidence = [Evidence {
            id: "ORG-1",
            source: EvidenceSource::Org,
            role: EvidenceRole::Required,
            text: "Current decision",
        }];
        let packet = organization_response(&evidence, "decision");
        assert_eq!(
            packet.pointer("/organizations/0/hits/0/claim_id"),
            Some(&Value::String("ORG-1".into()))
        );
        assert!(packet
            .pointer("/organizations/0/hits/0/evidence_locator")
            .is_some());
    }

    #[tokio::test]
    async fn production_org_and_scout_clients_accept_gateway_contracts() {
        let scenario = &crate::fixtures::scenarios()[0];
        let evidence = scenario.evidence.iter().collect::<Vec<_>>();
        let gateway = Gateway::start("http://127.0.0.1:9/v1", "benchmark-key", &evidence)
            .await
            .unwrap();
        let org = provider_local::recall_organization_knowledge(
            &gateway.base_url,
            "benchmark-key",
            "residency",
            None,
            20,
        )
        .await
        .unwrap();
        assert!(org.organizations[0]
            .hits
            .iter()
            .any(|hit| hit.claim_id == "ORG-RESIDENCY-04"));
        let repository = provider_local::recall_repository_context(
            &gateway.base_url,
            "benchmark-key",
            "benchmark",
            "compatibility",
        )
        .await
        .unwrap();
        assert_eq!(repository.commits.len(), 2);
        assert!(repository.commits[0].subject.contains("compatibility"));

        let organization_id = Uuid::parse_str(ORG_ID).unwrap();
        let workspace_id = Uuid::parse_str(WORKSPACE_ID).unwrap();
        let enrolled = enroll_machine(
            ClarkCartographyEnrollmentConfig::new(&gateway.base_url, "benchmark-key").unwrap(),
            &MachineEnrollmentRequest {
                organization_id,
                workspace_id,
                public_key: "01".repeat(32),
                platform: "benchmark".into(),
                architecture: "portable".into(),
            },
        )
        .await
        .unwrap();
        let page = enrolled
            .client
            .query_snapshot(&GraphSnapshotQuery {
                organization_id,
                workspace_id,
                effective_at_ms: None,
                known_at_ms: None,
                object_kinds: BTreeSet::new(),
                limit: MAX_GRAPH_SNAPSHOT_LIMIT.min(100),
                cursor: None,
            })
            .await
            .unwrap();
        assert!(page
            .entries
            .iter()
            .any(|entry| { entry.event.fact.attributes["evidence_id"] == "SCOUT-AUDIT-GRAPH" }));
        assert!(page
            .entries
            .iter()
            .any(|entry| entry.object_kind == GraphObjectKind::Entity));
        assert!(page
            .entries
            .iter()
            .any(|entry| entry.object_kind == GraphObjectKind::Edge));
        let receipts = gateway.receipts();
        assert!(receipts
            .iter()
            .any(|receipt| { receipt.operation == "organization_knowledge.search" }));
        assert!(receipts
            .iter()
            .any(|receipt| receipt.operation == "snapshots.query"));
        assert!(receipts
            .iter()
            .any(|receipt| receipt.operation == "code.repositories.context"));
        for receipt in receipts {
            assert!(!receipt.request_method.is_empty());
            assert!(!receipt.request_target.is_empty());
            assert_eq!(
                receipt.request_sha256,
                sha256_bytes(receipt.request_body.as_bytes())
            );
            assert_eq!(receipt.response_status, 200);
            assert!(!receipt.response_body.is_empty());
            assert_eq!(
                receipt.response_sha256,
                sha256_bytes(receipt.response_body.as_bytes())
            );
        }
    }
}
