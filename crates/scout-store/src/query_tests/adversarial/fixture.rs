use std::collections::BTreeSet;

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseEntityKind, EnterpriseEvent, EnterpriseFact,
    EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance, EnterpriseSignedBatch,
    EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey, EnterpriseTrustChain,
    EnterpriseTrustManifest, GraphEntityObservation,
};

use crate::{request, EntityQuery, ScoutStoreRequest, ScoutStoreResponse};

pub(super) struct ForeignFixture {
    pub(super) enterprise: EnterpriseId,
    pub(super) root: tempfile::TempDir,
    coordinator: EnterpriseSigningKey,
    manifest: EnterpriseTrustManifest,
}

impl ForeignFixture {
    pub(super) fn new(enterprise_id: &str, seed: u8) -> Self {
        let enterprise = EnterpriseId::new(enterprise_id).unwrap();
        let coordinator = EnterpriseSigningKey::from_seed([seed; 32]);
        let manifest = EnterpriseTrustManifest::initial(
            enterprise.clone(),
            format!("trust:00000000-0000-4000-8000-{seed:012x}"),
            100,
            100_000,
            &coordinator,
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        for directory in ["trust", "batches", "private"] {
            std::fs::create_dir_all(root.path().join(directory)).unwrap();
        }
        let chain = EnterpriseTrustChain {
            anchor_manifest_id: manifest.manifest_id.clone(),
            manifests: vec![manifest.clone()],
        };
        std::fs::write(
            root.path().join("trust/chain.json"),
            serde_json::to_vec(&chain).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.path().join("private/anchor-manifest-id"),
            chain.anchor_manifest_id.as_bytes(),
        )
        .unwrap();
        let mut bootstrap = [0_u8; 40];
        bootstrap[..32].copy_from_slice(&[seed; 32]);
        bootstrap[32..].copy_from_slice(&100_u64.to_le_bytes());
        std::fs::write(
            root.path().join("private/local-signing-bootstrap"),
            bootstrap,
        )
        .unwrap();
        std::fs::write(
            root.path().join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 3,
                "enterprise_id": enterprise,
                "anchor_manifest_id": chain.anchor_manifest_id,
                "local_signer_id": coordinator.signer_id(),
                "mode": "coordinator"
            }))
            .unwrap(),
        )
        .unwrap();
        Self {
            enterprise,
            root,
            coordinator,
            manifest,
        }
    }

    pub(super) fn ingest(&self, machine: &str) {
        let observation = GraphEntityObservation::new(
            &self.enterprise,
            EnterpriseEntityKind::CloudResource,
            AuthorityRef::new("fixture", "tenant:fixture", format!("resource:{machine}")).unwrap(),
            BTreeSet::from([format!("resource-{machine}")]),
            BTreeSet::from(["a".repeat(64)]),
        )
        .unwrap();
        let event = EnterpriseEvent::new(
            self.enterprise.clone(),
            EnterpriseProvenance {
                machine_id: machine.into(),
                run_id: format!("run-{machine}"),
                adapter_instance_id: "fixture-adapter".into(),
                auth_context_id: "fixture-auth".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                source_sequence: 1,
                observed_at_ms: 501,
                source_fingerprint: "f".repeat(64),
            },
            EnterpriseFact::EntityObserved(observation),
        )
        .unwrap();
        let batch = EnterpriseBatch::new(self.enterprise.clone(), [event]).unwrap();
        let grant = EnterpriseSignerGrant::issue(
            &self.manifest,
            self.coordinator.signer_id(),
            self.coordinator.public_key_hex(),
            BTreeSet::from([
                EnterpriseSignerRole::Collector,
                EnterpriseSignerRole::Coordinator,
            ]),
            EnterpriseGrantScope {
                machine_id: machine.into(),
                run_id: format!("run-{machine}"),
                adapter_instance_id: "fixture-adapter".into(),
                auth_context_id: "fixture-auth".into(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                first_source_sequence: 1,
                last_source_sequence: 1,
            },
            100,
            100_000,
            &[&self.coordinator],
        )
        .unwrap();
        let envelope =
            EnterpriseSignedBatch::sign(batch, &self.manifest, grant, 1_001, &self.coordinator)
                .unwrap();
        request(
            self.root.path(),
            ScoutStoreRequest::Ingest {
                enterprise_id: self.enterprise.clone(),
                envelope: Box::new(envelope),
            },
        )
        .unwrap();
    }

    pub(super) fn entity_page(&self, query: EntityQuery) -> crate::EntityPage {
        let response = request(
            self.root.path(),
            ScoutStoreRequest::Entities {
                enterprise_id: self.enterprise.clone(),
                query,
            },
        )
        .unwrap();
        let ScoutStoreResponse::Entities { page, .. } = response else {
            panic!("wrong entity response");
        };
        page
    }

    pub(super) fn index_path(&self) -> std::path::PathBuf {
        self.root.path().join("index-v4.sqlite3")
    }
}
