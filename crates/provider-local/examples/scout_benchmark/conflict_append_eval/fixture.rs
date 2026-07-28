use std::collections::BTreeSet;
use std::path::Path;

use agent_orchestration::{
    AuthorityRef, CoverageKey, CoverageObservation, CoverageStatus, EnterpriseBatch,
    EnterpriseEntityKind, EnterpriseEvent, EnterpriseFact, EnterpriseGrantScope, EnterpriseId,
    EnterpriseProvenance, EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole,
    EnterpriseSigningKey, EnterpriseTrustChain, EnterpriseTrustManifest, GraphEntityObservation,
    MAX_ENTERPRISE_EVENTS_PER_BATCH,
};
use rusqlite::Connection;
use scout_store::{
    dispatch as scout_store_dispatch, IndexReceipt, IndexedStatus, ScoutStoreRequest,
    ScoutStoreResponse, SERVICE_NAME as SCOUT_STORE_SERVICE,
};
use serde_json::json;
use sha2::{Digest, Sha256};

pub(super) struct ConflictFixture {
    root: tempfile::TempDir,
    pub(super) enterprise: EnterpriseId,
    manifest: EnterpriseTrustManifest,
    coordinator: EnterpriseSigningKey,
    conflicts: usize,
}

impl ConflictFixture {
    pub(super) fn new(conflicts: usize) -> Result<Self, String> {
        let root = tempfile::tempdir().map_err(to_string)?;
        let enterprise = EnterpriseId::new(format!("benchmark-conflicts-{conflicts}"))?;
        let coordinator = EnterpriseSigningKey::from_seed([0x7c; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise.clone(),
            "trust:00000000-0000-4000-8000-00000000007c".into(),
            1,
            1_000_000_000,
            &coordinator,
        )?;
        let chain = EnterpriseTrustChain {
            anchor_manifest_id: manifest.manifest_id.clone(),
            manifests: vec![manifest.clone()],
        };
        initialize_store(root.path(), &chain, &coordinator)?;
        let fixture = Self {
            root,
            enterprise,
            manifest,
            coordinator,
            conflicts,
        };
        fixture.write_seed_batches()?;
        Ok(fixture)
    }

    pub(super) fn rebuild(&self) -> Result<IndexReceipt, String> {
        let response = self.call(ScoutStoreRequest::Rebuild {
            enterprise_id: self.enterprise.clone(),
        })?;
        let ScoutStoreResponse::Rebuilt(receipt) = response else {
            return Err("conflict-corpus rebuild returned the wrong response".into());
        };
        Ok(receipt)
    }

    pub(super) fn unrelated_entity_envelope(&self) -> Result<EnterpriseSignedBatch, String> {
        let entity = GraphEntityObservation::new(
            &self.enterprise,
            EnterpriseEntityKind::Service,
            AuthorityRef::new(
                "conflict-benchmark",
                format!("tenant-{}", self.conflicts),
                "service:unrelated-hot-append",
            )?,
            BTreeSet::from(["unrelated-hot-append".into()]),
            evidence("unrelated-hot-append"),
        )?;
        let event = EnterpriseEvent::new(
            self.enterprise.clone(),
            provenance("conflict-hot", 2, 1),
            EnterpriseFact::EntityObserved(entity),
        )?;
        self.sign_events(vec![event], 2_000_001)
    }

    pub(super) fn call(&self, request: ScoutStoreRequest) -> Result<ScoutStoreResponse, String> {
        index_call(self.root.path(), request)
    }

    pub(super) fn status(&self) -> Result<IndexedStatus, String> {
        let response = self.call(ScoutStoreRequest::Status {
            enterprise_id: self.enterprise.clone(),
        })?;
        let ScoutStoreResponse::Status { status, .. } = response else {
            return Err("conflict-corpus status returned the wrong response".into());
        };
        Ok(*status)
    }

    pub(super) fn normalized_conflict_count(&self) -> Result<usize, String> {
        let connection =
            Connection::open(self.root.path().join("index-v4.sqlite3")).map_err(to_string)?;
        let count = connection
            .query_row("SELECT COUNT(*) FROM projection_conflicts", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(to_string)?;
        usize::try_from(count).map_err(|_| "normalized conflict count is negative".into())
    }

    pub(super) fn force_cold(&self) -> Result<IndexReceipt, String> {
        force_cold_rebuild(self.root.path())?;
        let receipt = self.rebuild()?;
        if !receipt.rebuilt {
            return Err("conflict-corpus cold comparison did not rebuild".into());
        }
        Ok(receipt)
    }

    fn write_seed_batches(&self) -> Result<(), String> {
        let mut events = Vec::with_capacity(MAX_ENTERPRISE_EVENTS_PER_BATCH);
        let mut batch_sequence = 0_u64;
        for index in 0..self.conflicts {
            let key = CoverageKey::new(
                "conflict-benchmark",
                "benchmark-read-only",
                format!("tenant-{}", self.conflicts),
                format!("scope-{index:06}"),
                "service",
            )?;
            let supported = CoverageObservation::new(
                &self.enterprise,
                key.clone(),
                CoverageStatus::Supported,
                None,
                1,
                evidence(&format!("coverage-supported-{index}")),
            )?;
            let denied = CoverageObservation::new(
                &self.enterprise,
                key,
                CoverageStatus::Denied,
                None,
                0,
                evidence(&format!("coverage-denied-{index}")),
            )?;
            for observation in [supported, denied] {
                let source_sequence = (index * 2 + events.len() % 2 + 1) as u64;
                events.push(EnterpriseEvent::new(
                    self.enterprise.clone(),
                    provenance("conflict-seed", 1, source_sequence),
                    EnterpriseFact::CoverageObserved(observation),
                )?);
                if events.len() == MAX_ENTERPRISE_EVENTS_PER_BATCH {
                    batch_sequence += 1;
                    self.write_batch(std::mem::take(&mut events), batch_sequence)?;
                    events = Vec::with_capacity(MAX_ENTERPRISE_EVENTS_PER_BATCH);
                }
            }
        }
        if !events.is_empty() {
            batch_sequence += 1;
            self.write_batch(events, batch_sequence)?;
        }
        force_cold_rebuild(self.root.path())
    }

    fn write_batch(&self, events: Vec<EnterpriseEvent>, batch_sequence: u64) -> Result<(), String> {
        let envelope = self.sign_events(events, 1_000_000 + batch_sequence)?;
        ingest_envelope(self.root.path(), &envelope)
    }

    fn sign_events(
        &self,
        events: Vec<EnterpriseEvent>,
        signed_at_ms: u64,
    ) -> Result<EnterpriseSignedBatch, String> {
        let first = events
            .first()
            .ok_or_else(|| "conflict benchmark cannot sign an empty batch".to_string())?;
        let last = events
            .last()
            .ok_or_else(|| "conflict benchmark cannot sign an empty batch".to_string())?;
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
    let mut bootstrap = vec![0x7c; 32];
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
    let response = index_call(
        root,
        ScoutStoreRequest::Ingest {
            enterprise_id: envelope.batch.enterprise_id.clone(),
            envelope: Box::new(envelope.clone()),
        },
    )?;
    matches!(response, ScoutStoreResponse::Ingested { .. })
        .then_some(())
        .ok_or_else(|| "conflict benchmark seed ingest returned the wrong response".into())
}

fn force_cold_rebuild(root: &Path) -> Result<(), String> {
    let connection = Connection::open(root.join("index-v4.sqlite3")).map_err(to_string)?;
    connection
        .execute(
            "UPDATE meta SET value = 'conflict-corpus-force-cold' \
             WHERE key = 'projection_version'",
            [],
        )
        .map_err(to_string)?;
    Ok(())
}

fn index_call(root: &Path, request: ScoutStoreRequest) -> Result<ScoutStoreResponse, String> {
    let request = serde_json::to_vec(&request).map_err(to_string)?;
    let response = scout_store_dispatch(SCOUT_STORE_SERVICE, root, &request)?;
    serde_json::from_slice(&response).map_err(to_string)
}

fn provenance(machine: &str, epoch: u64, sequence: u64) -> EnterpriseProvenance {
    EnterpriseProvenance {
        machine_id: machine.into(),
        run_id: format!("conflict-{machine}-{epoch}"),
        adapter_instance_id: "conflict-benchmark".into(),
        auth_context_id: "benchmark-read-only".into(),
        discovery_epoch: format!("epoch-{epoch}"),
        discovery_epoch_sequence: epoch,
        source_sequence: sequence,
        observed_at_ms: epoch * 1_000_000 + sequence,
        source_fingerprint: "c".repeat(64),
    }
}

fn evidence(tag: &str) -> BTreeSet<String> {
    BTreeSet::from([format!("{:x}", Sha256::digest(tag.as_bytes()))])
}

fn to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
