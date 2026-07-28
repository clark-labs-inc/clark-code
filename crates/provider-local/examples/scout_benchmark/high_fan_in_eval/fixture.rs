use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use agent_orchestration::{
    AuthorityRef, CoverageKey, CoverageObservation, CoverageStatus, EnterpriseBatch,
    EnterpriseEntityId, EnterpriseEntityKind, EnterpriseEvent, EnterpriseFact,
    EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance, EnterpriseSignedBatch,
    EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey, EnterpriseTrustChain,
    EnterpriseTrustManifest, FrontierKey, FrontierObservation, FrontierState,
    GraphEntityObservation, SimulationContractObservation, MAX_ENTERPRISE_EVENTS_PER_BATCH,
};
use rusqlite::Connection;
use scout_store::{
    dispatch as scout_store_dispatch, IndexReceipt, IndexedStatus, ScoutStoreRequest,
    ScoutStoreResponse, SERVICE_NAME as SCOUT_STORE_SERVICE,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

pub(super) const LOCATOR_KINDS: [&str; 4] = ["entity", "coverage", "frontier", "simulation"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct MaterializedRows {
    pub(super) json_by_kind: BTreeMap<String, String>,
}

impl MaterializedRows {
    pub(super) fn byte_len(&self, kind: &str) -> Result<usize, String> {
        self.json_by_kind
            .get(kind)
            .map(String::len)
            .ok_or_else(|| format!("high-fan-in fixture has no {kind} row"))
    }
}

pub(super) struct FanInFixture {
    root: tempfile::TempDir,
    pub(super) enterprise: EnterpriseId,
    manifest: EnterpriseTrustManifest,
    coordinator: EnterpriseSigningKey,
    entity_id: EnterpriseEntityId,
    coverage_key: CoverageKey,
    n: usize,
}

impl FanInFixture {
    pub(super) fn new(n: usize) -> Result<Self, String> {
        let root = tempfile::tempdir().map_err(to_string)?;
        let enterprise = EnterpriseId::new(format!("benchmark-high-fan-in-{n}"))?;
        let coordinator = EnterpriseSigningKey::from_seed([0x6b; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise.clone(),
            "trust:00000000-0000-4000-8000-00000000006b".into(),
            1,
            1_000_000_000,
            &coordinator,
        )?;
        let chain = EnterpriseTrustChain {
            anchor_manifest_id: manifest.manifest_id.clone(),
            manifests: vec![manifest.clone()],
        };
        initialize_store(root.path(), &chain, &coordinator)?;
        let entity_id = GraphEntityObservation::new(
            &enterprise,
            EnterpriseEntityKind::Service,
            authority(n)?,
            BTreeSet::from(["high-fan-in-runtime".into()]),
            evidence("entity-id"),
        )?
        .entity_id;
        let coverage_key = CoverageKey::new(
            "high-fan-in-adapter",
            "benchmark-read-only",
            format!("tenant-{n}"),
            "all-regions",
            "runtime",
        )?;
        Ok(Self {
            root,
            enterprise,
            manifest,
            coordinator,
            entity_id,
            coverage_key,
            n,
        })
    }

    pub(super) fn write_seed_batches(&self) -> Result<usize, String> {
        let mut events = Vec::with_capacity(MAX_ENTERPRISE_EVENTS_PER_BATCH);
        let mut batch_index = 0_u64;
        for index in 0..self.n {
            let base = u64::try_from(index)
                .map_err(|_| "high-fan-in index exceeds u64".to_string())?
                .checked_mul(4)
                .ok_or_else(|| "high-fan-in source sequence overflow".to_string())?;
            for (offset, fact) in self.facts(index)?.into_iter().enumerate() {
                let sequence = base + offset as u64 + 1;
                events.push(EnterpriseEvent::new(
                    self.enterprise.clone(),
                    provenance("high-fan-in-seed", sequence),
                    fact,
                )?);
                if events.len() == MAX_ENTERPRISE_EVENTS_PER_BATCH {
                    self.flush_seed_batch(std::mem::take(&mut events), batch_index)?;
                    events = Vec::with_capacity(MAX_ENTERPRISE_EVENTS_PER_BATCH);
                    batch_index += 1;
                }
            }
        }
        if !events.is_empty() {
            self.flush_seed_batch(events, batch_index)?;
        }
        force_cold_rebuild(self.root.path())?;
        self.n
            .checked_mul(4)
            .ok_or_else(|| "high-fan-in seed event count overflow".to_string())
    }

    pub(super) fn rebuild(&self) -> Result<IndexReceipt, String> {
        let response = self.call(ScoutStoreRequest::Rebuild {
            enterprise_id: self.enterprise.clone(),
        })?;
        let ScoutStoreResponse::Rebuilt(receipt) = response else {
            return Err("high-fan-in rebuild returned the wrong response".into());
        };
        Ok(receipt)
    }

    pub(super) fn append(&self, kind: &str) -> Result<IndexReceipt, String> {
        let index = self.n;
        let fact = match kind {
            "entity" => self.entity_fact(index)?,
            "coverage" => self.coverage_fact(index)?,
            "frontier" => self.frontier_fact(index)?,
            "simulation" => self.simulation_fact(index),
            _ => return Err(format!("unknown high-fan-in locator kind {kind}")),
        };
        let envelope = self.sign_events(
            vec![EnterpriseEvent::new(
                self.enterprise.clone(),
                provenance(&format!("high-fan-in-hot-{kind}"), 1),
                fact,
            )?],
            50_000 + index as u64,
        )?;
        let response = self.call(ScoutStoreRequest::Ingest {
            enterprise_id: self.enterprise.clone(),
            envelope: Box::new(envelope),
        })?;
        let ScoutStoreResponse::Ingested { receipt, .. } = response else {
            return Err(format!(
                "{kind} high-fan-in append returned the wrong response"
            ));
        };
        Ok(receipt)
    }

    pub(super) fn prime_hot_path(&self) -> Result<IndexReceipt, String> {
        let fact = GraphEntityObservation::new(
            &self.enterprise,
            EnterpriseEntityKind::Service,
            AuthorityRef::new(
                "high-fan-in-benchmark",
                format!("tenant-{}", self.n),
                "runtime:hot-path-prime",
            )?,
            BTreeSet::from(["high-fan-in-prime".into()]),
            evidence("hot-path-prime"),
        )
        .map(EnterpriseFact::EntityObserved)?;
        let envelope = self.sign_events(
            vec![EnterpriseEvent::new(
                self.enterprise.clone(),
                provenance("high-fan-in-hot-path-prime", 1),
                fact,
            )?],
            49_999,
        )?;
        let response = self.call(ScoutStoreRequest::Ingest {
            enterprise_id: self.enterprise.clone(),
            envelope: Box::new(envelope),
        })?;
        let ScoutStoreResponse::Ingested { receipt, .. } = response else {
            return Err("high-fan-in hot-path prime returned the wrong response".into());
        };
        Ok(receipt)
    }

    pub(super) fn rows(&self) -> Result<MaterializedRows, String> {
        let connection = Connection::open(self.database_path()).map_err(to_string)?;
        let frontier_id = FrontierKey::new(self.coverage_key.clone(), None)?.id(&self.enterprise)?;
        let coverage_id = self.coverage_key.id(&self.enterprise)?;
        let mut rows = BTreeMap::new();
        rows.insert(
            "entity".into(),
            query_one(
                &connection,
                "SELECT materialized_json FROM entities WHERE entity_id = ?1",
                &[self.entity_id.as_str()],
            )?,
        );
        for (kind, id) in [
            ("coverage", coverage_id.as_str()),
            ("frontier", frontier_id.as_str()),
            ("simulation", self.entity_id.as_str()),
        ] {
            rows.insert(
                kind.into(),
                query_one(
                    &connection,
                    "SELECT materialized_json FROM auxiliary_projection \
                     WHERE lane = ?1 AND object_id = ?2",
                    &[kind, id],
                )?,
            );
        }
        Ok(MaterializedRows { json_by_kind: rows })
    }

    pub(super) fn status(&self) -> Result<IndexedStatus, String> {
        let response = self.call(ScoutStoreRequest::Status {
            enterprise_id: self.enterprise.clone(),
        })?;
        let ScoutStoreResponse::Status { status, .. } = response else {
            return Err("high-fan-in status returned the wrong response".into());
        };
        Ok(*status)
    }

    pub(super) fn force_cold(&self) -> Result<IndexReceipt, String> {
        force_cold_rebuild(self.root.path())?;
        let receipt = self.rebuild()?;
        if !receipt.rebuilt {
            return Err("high-fan-in cold comparison did not rebuild".into());
        }
        Ok(receipt)
    }

    pub(super) fn database_bytes(&self) -> Result<u64, String> {
        std::fs::metadata(self.database_path())
            .map(|value| value.len())
            .map_err(to_string)
    }

    fn database_path(&self) -> std::path::PathBuf {
        self.root.path().join("index-v4.sqlite3")
    }

    fn facts(&self, index: usize) -> Result<[EnterpriseFact; 4], String> {
        Ok([
            self.entity_fact(index)?,
            self.coverage_fact(index)?,
            self.frontier_fact(index)?,
            self.simulation_fact(index),
        ])
    }

    fn entity_fact(&self, index: usize) -> Result<EnterpriseFact, String> {
        GraphEntityObservation::new(
            &self.enterprise,
            EnterpriseEntityKind::Service,
            authority(self.n)?,
            BTreeSet::from(["high-fan-in-runtime".into()]),
            evidence(&format!("entity-{index}")),
        )
        .map(EnterpriseFact::EntityObserved)
    }

    fn coverage_fact(&self, index: usize) -> Result<EnterpriseFact, String> {
        CoverageObservation::new(
            &self.enterprise,
            self.coverage_key.clone(),
            CoverageStatus::Supported,
            None,
            1,
            evidence(&format!("coverage-{index}")),
        )
        .map(EnterpriseFact::CoverageObserved)
    }

    fn frontier_fact(&self, index: usize) -> Result<EnterpriseFact, String> {
        FrontierObservation::new(
            &self.enterprise,
            FrontierKey::new(self.coverage_key.clone(), None)?,
            FrontierState::Terminal {
                status: CoverageStatus::Supported,
                reason: "high-fan-in fixture completed".into(),
            },
            evidence(&format!("frontier-{index}")),
        )
        .map(EnterpriseFact::FrontierObserved)
    }

    fn simulation_fact(&self, index: usize) -> EnterpriseFact {
        EnterpriseFact::SimulationContractObserved(SimulationContractObservation {
            runtime_id: self.entity_id.clone(),
            inputs: true,
            outputs: true,
            state_effects: true,
            timeouts: true,
            retries: true,
            idempotency: true,
            failure_behavior: true,
            observability: true,
            recovery: true,
            evidence_digests: evidence(&format!("simulation-{index}")),
        })
    }

    fn flush_seed_batch(
        &self,
        events: Vec<EnterpriseEvent>,
        batch_index: u64,
    ) -> Result<(), String> {
        let envelope = self.sign_events(events, 10_000 + batch_index)?;
        ingest_envelope(self.root.path(), &envelope)
    }

    fn sign_events(
        &self,
        events: Vec<EnterpriseEvent>,
        signed_at_ms: u64,
    ) -> Result<EnterpriseSignedBatch, String> {
        let first = events
            .first()
            .ok_or_else(|| "high-fan-in benchmark cannot sign an empty batch".to_string())?;
        let last = events
            .last()
            .ok_or_else(|| "high-fan-in benchmark cannot sign an empty batch".to_string())?;
        let grant = EnterpriseSignerGrant::issue(
            &self.manifest,
            self.coordinator.signer_id(),
            self.coordinator.public_key_hex(),
            BTreeSet::from([
                EnterpriseSignerRole::Collector,
                EnterpriseSignerRole::Coordinator,
            ]),
            EnterpriseGrantScope {
                machine_id: first.provenance.machine_id.clone(),
                run_id: first.provenance.run_id.clone(),
                adapter_instance_id: first.provenance.adapter_instance_id.clone(),
                auth_context_id: first.provenance.auth_context_id.clone(),
                discovery_epoch: first.provenance.discovery_epoch.clone(),
                discovery_epoch_sequence: first.provenance.discovery_epoch_sequence,
                first_source_sequence: first.provenance.source_sequence,
                last_source_sequence: last.provenance.source_sequence,
            },
            1,
            1_000_000_000,
            &[&self.coordinator],
        )?;
        EnterpriseSignedBatch::sign(
            EnterpriseBatch::new(self.enterprise.clone(), events)?,
            &self.manifest,
            grant,
            signed_at_ms,
            &self.coordinator,
        )
    }

    fn call(&self, request: ScoutStoreRequest) -> Result<ScoutStoreResponse, String> {
        let request = serde_json::to_vec(&request).map_err(to_string)?;
        let response = scout_store_dispatch(SCOUT_STORE_SERVICE, self.root.path(), &request)?;
        serde_json::from_slice(&response).map_err(to_string)
    }
}

fn initialize_store(
    root: &Path,
    chain: &EnterpriseTrustChain,
    coordinator: &EnterpriseSigningKey,
) -> Result<(), String> {
    std::fs::create_dir_all(root.join("trust")).map_err(to_string)?;
    std::fs::create_dir_all(root.join("private")).map_err(to_string)?;
    std::fs::write(
        root.join("trust/chain.json"),
        serde_json::to_vec(chain).map_err(to_string)?,
    )
    .map_err(to_string)?;
    std::fs::write(
        root.join("private/anchor-manifest-id"),
        chain.anchor_manifest_id.as_bytes(),
    )
    .map_err(to_string)?;
    let mut bootstrap = vec![0x6b; 32];
    bootstrap.extend_from_slice(&1_u64.to_le_bytes());
    std::fs::write(root.join("private/local-signing-bootstrap"), bootstrap).map_err(to_string)?;
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&json!({
            "schema_version": 3,
            "enterprise_id": chain.manifests[0].enterprise_id,
            "anchor_manifest_id": chain.anchor_manifest_id,
            "local_signer_id": coordinator.signer_id(),
            "mode": "coordinator",
        }))
        .map_err(to_string)?,
    )
    .map_err(to_string)
}

fn ingest_envelope(root: &Path, envelope: &EnterpriseSignedBatch) -> Result<(), String> {
    let request = ScoutStoreRequest::Ingest {
        enterprise_id: envelope.batch.enterprise_id.clone(),
        envelope: Box::new(envelope.clone()),
    };
    let request = serde_json::to_vec(&request).map_err(to_string)?;
    let response = scout_store_dispatch(SCOUT_STORE_SERVICE, root, &request)?;
    let response: ScoutStoreResponse = serde_json::from_slice(&response).map_err(to_string)?;
    matches!(response, ScoutStoreResponse::Ingested { .. })
        .then_some(())
        .ok_or_else(|| "high-fan-in seed ingest returned the wrong response".into())
}

fn force_cold_rebuild(root: &Path) -> Result<(), String> {
    let connection = Connection::open(root.join("index-v4.sqlite3")).map_err(to_string)?;
    connection
        .execute(
            "UPDATE meta SET value = 'high-fan-in-force-cold' \
             WHERE key = 'projection_version'",
            [],
        )
        .map_err(to_string)?;
    Ok(())
}

fn query_one(connection: &Connection, sql: &str, parameters: &[&str]) -> Result<String, String> {
    connection
        .query_row(sql, rusqlite::params_from_iter(parameters), |row| {
            row.get(0)
        })
        .map_err(to_string)
}

fn authority(n: usize) -> Result<AuthorityRef, String> {
    AuthorityRef::new(
        "high-fan-in-benchmark",
        format!("tenant-{n}"),
        "runtime:shared",
    )
}

fn provenance(machine: &str, sequence: u64) -> EnterpriseProvenance {
    EnterpriseProvenance {
        machine_id: machine.into(),
        run_id: format!("{machine}-run"),
        adapter_instance_id: "high-fan-in-benchmark".into(),
        auth_context_id: "benchmark-read-only".into(),
        discovery_epoch: "epoch-1".into(),
        discovery_epoch_sequence: 1,
        source_sequence: sequence,
        observed_at_ms: 1_000_000 + sequence,
        source_fingerprint: "b".repeat(64),
    }
}

fn evidence(tag: &str) -> BTreeSet<String> {
    BTreeSet::from([format!("{:x}", Sha256::digest(tag.as_bytes()))])
}

fn to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
