use std::collections::BTreeSet;
use std::path::Path;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseEdgeKind, EnterpriseEntityId, EnterpriseEntityKind,
    EnterpriseEvent, EnterpriseFact, EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance,
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey,
    EnterpriseTrustChain, EnterpriseTrustManifest, GraphEdgeObservation, GraphEntityObservation,
    MAX_ENTERPRISE_EVENTS_PER_BATCH,
};
use rusqlite::Connection;
use scout_store::{
    dispatch as scout_store_dispatch, EdgePage, EdgeQuery, EntityPage, EntityQuery, IndexReceipt,
    IndexedStatus, ScoutStoreRequest, ScoutStoreResponse, SERVICE_NAME as SCOUT_STORE_SERVICE,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

const HOT_LABEL: &str = "affected-row-benchmark-update";

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct AffectedState {
    entity: EntityPage,
    outgoing_edges: EdgePage,
    incoming_edges: EdgePage,
}

pub(super) struct ScaleFixture {
    root: tempfile::TempDir,
    pub(super) enterprise: EnterpriseId,
    manifest: EnterpriseTrustManifest,
    coordinator: EnterpriseSigningKey,
    services: usize,
}

impl ScaleFixture {
    pub(super) fn new(services: usize) -> Result<Self, String> {
        let root = tempfile::tempdir().map_err(|error| error.to_string())?;
        let enterprise = EnterpriseId::new(format!("benchmark-affected-{services}"))?;
        let coordinator = EnterpriseSigningKey::from_seed([0x6a; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise.clone(),
            "trust:00000000-0000-4000-8000-00000000006a".into(),
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
            services,
        };
        fixture.write_seed_batches()?;
        Ok(fixture)
    }

    pub(super) fn rebuild(&self) -> Result<(IndexReceipt, usize), String> {
        let response = self.call(ScoutStoreRequest::Rebuild {
            enterprise_id: self.enterprise.clone(),
        })?;
        let ScoutStoreResponse::Rebuilt(receipt) = response else {
            return Err("affected-row seed rebuild returned the wrong response".into());
        };
        Ok((receipt, self.services * 2 - 1))
    }

    fn write_seed_batches(&self) -> Result<(), String> {
        let mut events = Vec::with_capacity(self.services * 2 - 1);
        let entities = (0..self.services)
            .map(|index| self.entity(index, false))
            .collect::<Result<Vec<_>, _>>()?;
        for (index, entity) in entities.iter().enumerate() {
            events.push(EnterpriseEvent::new(
                self.enterprise.clone(),
                provenance("affected-row-seed", 1, index as u64 + 1),
                EnterpriseFact::EntityObserved(entity.clone()),
            )?);
        }
        for index in 1..self.services {
            let sequence = self.services + index;
            let edge = GraphEdgeObservation::new(
                &self.enterprise,
                entities[index - 1].entity_id.clone(),
                entities[index].entity_id.clone(),
                EnterpriseEdgeKind::Calls,
                None,
                evidence(&format!("seed-edge-{index}")),
            )?;
            events.push(EnterpriseEvent::new(
                self.enterprise.clone(),
                provenance("affected-row-seed", 1, sequence as u64),
                EnterpriseFact::EdgeObserved(edge),
            )?);
        }

        for (chunk_index, chunk) in events.chunks(MAX_ENTERPRISE_EVENTS_PER_BATCH).enumerate() {
            let envelope = self.sign_events(chunk.to_vec(), 10_000 + chunk_index as u64)?;
            ingest_envelope(self.root.path(), &envelope)?;
        }
        force_cold_rebuild(self.root.path())
    }

    pub(super) fn updated_middle_entity(&self) -> Result<GraphEntityObservation, String> {
        self.entity(self.services / 2, true)
    }

    fn entity(&self, index: usize, updated: bool) -> Result<GraphEntityObservation, String> {
        let mut labels = BTreeSet::from([format!("service-{index}")]);
        if updated {
            labels.insert(HOT_LABEL.into());
        }
        GraphEntityObservation::new(
            &self.enterprise,
            EnterpriseEntityKind::Service,
            AuthorityRef::new(
                "affected-benchmark",
                format!("tenant-{}", self.services),
                format!("service:{index}"),
            )?,
            labels,
            evidence(&format!("seed-entity-{index}-updated-{updated}")),
        )
    }

    pub(super) fn sign_facts(
        &self,
        machine: &str,
        epoch: u64,
        facts: Vec<EnterpriseFact>,
    ) -> Result<EnterpriseSignedBatch, String> {
        let events = facts
            .into_iter()
            .enumerate()
            .map(|(offset, fact)| {
                EnterpriseEvent::new(
                    self.enterprise.clone(),
                    provenance(machine, epoch, offset as u64 + 1),
                    fact,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.sign_events(events, 20_000 + epoch)
    }

    fn sign_events(
        &self,
        events: Vec<EnterpriseEvent>,
        signed_at_ms: u64,
    ) -> Result<EnterpriseSignedBatch, String> {
        let first = events
            .first()
            .ok_or_else(|| "affected-row benchmark cannot sign an empty batch".to_string())?;
        let last = events
            .last()
            .ok_or_else(|| "affected-row benchmark cannot sign an empty batch".to_string())?;
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
        let batch = EnterpriseBatch::new(self.enterprise.clone(), events)?;
        EnterpriseSignedBatch::sign(
            batch,
            &self.manifest,
            grant,
            signed_at_ms,
            &self.coordinator,
        )
    }

    pub(super) fn call(&self, request: ScoutStoreRequest) -> Result<ScoutStoreResponse, String> {
        index_call(self.root.path(), request)
    }

    pub(super) fn status(&self) -> Result<IndexedStatus, String> {
        let response = self.call(ScoutStoreRequest::Status {
            enterprise_id: self.enterprise.clone(),
        })?;
        let ScoutStoreResponse::Status { status, .. } = response else {
            return Err("affected-row status returned the wrong response".into());
        };
        Ok(*status)
    }

    pub(super) fn affected_state(
        &self,
        entity_id: &EnterpriseEntityId,
    ) -> Result<AffectedState, String> {
        let entity = self.call(ScoutStoreRequest::Entities {
            enterprise_id: self.enterprise.clone(),
            query: EntityQuery {
                label_contains: Some(HOT_LABEL.into()),
                limit: 2,
                ..EntityQuery::default()
            },
        })?;
        let ScoutStoreResponse::Entities { page: entity, .. } = entity else {
            return Err("affected entity query returned the wrong response".into());
        };
        if entity.entities.len() != 1 || entity.next_cursor.is_some() {
            return Err("affected entity query was not exact".into());
        }
        let outgoing_edges = self.edge_page(Some(entity_id.clone()), None)?;
        let incoming_edges = self.edge_page(None, Some(entity_id.clone()))?;
        Ok(AffectedState {
            entity,
            outgoing_edges,
            incoming_edges,
        })
    }

    fn edge_page(
        &self,
        from: Option<EnterpriseEntityId>,
        to: Option<EnterpriseEntityId>,
    ) -> Result<EdgePage, String> {
        let response = self.call(ScoutStoreRequest::Edges {
            enterprise_id: self.enterprise.clone(),
            query: EdgeQuery {
                from,
                to,
                limit: 4,
                ..EdgeQuery::default()
            },
        })?;
        let ScoutStoreResponse::Edges { page, .. } = response else {
            return Err("affected edge query returned the wrong response".into());
        };
        Ok(page)
    }

    pub(super) fn force_cold(&self) -> Result<IndexReceipt, String> {
        force_cold_rebuild(self.root.path())?;
        let response = self.call(ScoutStoreRequest::Rebuild {
            enterprise_id: self.enterprise.clone(),
        })?;
        let ScoutStoreResponse::Rebuilt(receipt) = response else {
            return Err("affected-row forced rebuild returned the wrong response".into());
        };
        if !receipt.rebuilt {
            return Err("affected-row cold comparison did not rebuild".into());
        }
        Ok(receipt)
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
    let mut bootstrap = vec![0x6a; 32];
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
        .ok_or_else(|| "affected-row benchmark seed ingest returned the wrong response".into())
}

fn force_cold_rebuild(root: &Path) -> Result<(), String> {
    let connection = Connection::open(root.join("index-v4.sqlite3")).map_err(to_string)?;
    connection
        .execute(
            "UPDATE meta SET value = 'affected-row-force-cold' \
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
        run_id: format!("affected-row-{machine}-{epoch}"),
        adapter_instance_id: "affected-row-benchmark".into(),
        auth_context_id: "benchmark-read-only".into(),
        discovery_epoch: format!("epoch-{epoch}"),
        discovery_epoch_sequence: epoch,
        source_sequence: sequence,
        observed_at_ms: epoch * 1_000_000 + sequence,
        source_fingerprint: "a".repeat(64),
    }
}

fn evidence(tag: &str) -> BTreeSet<String> {
    BTreeSet::from([format!("{:x}", Sha256::digest(tag.as_bytes()))])
}

fn to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
