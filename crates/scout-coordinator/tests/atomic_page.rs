use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{
    AuthorityRef, EnterpriseBatch, EnterpriseBatchBundle, EnterpriseEntityKind, EnterpriseEvent,
    EnterpriseFact, EnterpriseGrantScope, EnterpriseId, EnterpriseProvenance,
    EnterpriseSignedBatch, EnterpriseSignerGrant, EnterpriseSignerRole, EnterpriseSigningKey,
    EnterpriseTrustChain, EnterpriseTrustManifest, GraphEntityObservation,
};
use scout_adapter_protocol::{
    AdapterId, AdapterPageLimits, AdapterPageOutcome, AdapterPageReceipt, AdapterPageRequest,
    AdapterQuery, AuthContextDescriptor, AuthContextHandle, AuthSourceKind, CoverageBinding,
    NormalizedRecord, RedactionSummary, RequestId, SafeFieldValue, TargetIdentity,
};
use scout_coordinator::CoordinatorStore;
use scout_ingest_protocol::{CoordinatorSigningKey, IngestRequest, ScoutTenantId};
use scout_scheduler::{
    CompletionDisposition, PageCompletion, QuotaPolicy, ScheduleManifest, Scheduler, TaskOrigin,
    TaskSpec,
};

const ENTERPRISE: &str = "enterprise-acme";
const CHARTER: &str = "charter-topology";

struct Fixture {
    tenant_id: ScoutTenantId,
    enterprise_id: EnterpriseId,
    chain: EnterpriseTrustChain,
    grant: EnterpriseSignerGrant,
    collector: EnterpriseSigningKey,
    adapter_receipt: AdapterPageReceipt,
    scheduler: Scheduler,
}

impl Fixture {
    fn new() -> Self {
        let tenant_id = ScoutTenantId::new("organization:acme").unwrap();
        let enterprise_id = EnterpriseId::new(ENTERPRISE).unwrap();
        let adapter_id = AdapterId::new("clark/github-organization@1").unwrap();
        let target = TargetIdentity::new(
            digest('1'),
            digest('2'),
            digest('3'),
            digest('4'),
            "linux".into(),
            "x86_64".into(),
        )
        .unwrap();
        let auth = AuthContextDescriptor::new(
            AuthContextHandle::new("auth:00000000-0000-4000-8000-000000000001").unwrap(),
            target.target_id.clone(),
            adapter_id.clone(),
            "github".into(),
            "acme".into(),
            "principal:42".into(),
            AuthSourceKind::CliProfile,
            digest('5'),
            900,
            Some(10_000),
        )
        .unwrap();
        let coverage = CoverageBinding {
            enterprise_id: ENTERPRISE.into(),
            charter_id: CHARTER.into(),
            discovery_epoch: 1,
            sequence: 1,
            adapter_id: adapter_id.clone(),
            auth_context_id: auth.context_id.clone(),
            tenant: "acme".into(),
            region_or_project: "global".into(),
            resource_kind: "repository".into(),
        };
        let query = AdapterQuery {
            operation: "list_repositories".into(),
            authority_scope: "acme".into(),
            provider_resource_type: "github.repository".into(),
            filters: BTreeMap::new(),
            projection: BTreeSet::from(["name".into()]),
            page_size: 100,
        };
        let request = AdapterPageRequest {
            protocol_version: scout_adapter_protocol::ADAPTER_PROTOCOL_VERSION,
            request_id: RequestId::new("request:00000000-0000-4000-8000-000000000001").unwrap(),
            target_id: target.target_id.clone(),
            target_identity_sha256: target.fingerprint_sha256().unwrap(),
            adapter_id: adapter_id.clone(),
            auth_context_handle: auth.handle.clone(),
            auth_context_id: auth.context_id.clone(),
            coverage: coverage.clone(),
            query: query.clone(),
            page_ordinal: 0,
            cursor_handle: None,
            limits: AdapterPageLimits {
                max_records: 100,
                max_response_bytes: 1_000_000,
                max_duration_ms: 30_000,
            },
            requested_at_ms: 1_000,
        };
        let record = NormalizedRecord::new(
            adapter_id.clone(),
            "github".into(),
            "github.repository".into(),
            "global".into(),
            "42".into(),
            Some("repository".into()),
            BTreeSet::from(["acme/checkout".into()]),
            BTreeMap::from([("name".into(), SafeFieldValue::Text("checkout".into()))]),
            BTreeSet::new(),
        )
        .unwrap();
        let adapter_receipt = AdapterPageReceipt::new(
            request,
            target.clone(),
            auth.clone(),
            digest('6'),
            1_100,
            AdapterPageOutcome::Succeeded { final_page: true },
            vec![record],
            None,
            RedactionSummary {
                source_records_seen: 1,
                records_emitted: 1,
                fields_omitted: 0,
                values_rejected: 0,
            },
        )
        .unwrap();
        let task = TaskSpec::new(
            ENTERPRISE,
            CHARTER,
            1,
            target.target_id.clone(),
            adapter_id,
            auth.context_id.clone(),
            auth.handle.clone(),
            coverage,
            query,
            0,
            None,
            TaskOrigin::Root,
            100,
        )
        .unwrap();
        let manifest = ScheduleManifest::new(
            ENTERPRISE,
            CHARTER,
            1,
            BTreeSet::from([task.task_id.clone()]),
            BTreeMap::from([(
                task.quota_key(),
                QuotaPolicy {
                    max_in_flight: 1,
                    min_start_interval_ms: 0,
                    lease_duration_ms: 10_000,
                    base_backoff_ms: 100,
                    max_backoff_ms: 10_000,
                    max_attempts: 3,
                },
            )]),
            BTreeMap::new(),
        )
        .unwrap();
        let scheduler = Scheduler::new(manifest, vec![task], 10).unwrap();

        let administrator = EnterpriseSigningKey::from_seed([7; 32]);
        let collector = EnterpriseSigningKey::from_seed([8; 32]);
        let trust_manifest = EnterpriseTrustManifest::initial(
            enterprise_id.clone(),
            format!("trust:{}", digest('a')),
            100,
            100_000,
            &administrator,
        )
        .unwrap();
        let grant = EnterpriseSignerGrant::issue(
            &trust_manifest,
            collector.signer_id(),
            collector.public_key_hex(),
            BTreeSet::from([EnterpriseSignerRole::Collector]),
            EnterpriseGrantScope {
                machine_id: target.target_id.to_string(),
                run_id: "run-a".into(),
                adapter_instance_id: "github-acme".into(),
                auth_context_id: auth.context_id.to_string(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                first_source_sequence: 1,
                last_source_sequence: 100,
            },
            100,
            90_000,
            &[&administrator],
        )
        .unwrap();
        Self {
            tenant_id,
            enterprise_id,
            chain: EnterpriseTrustChain {
                anchor_manifest_id: trust_manifest.manifest_id.clone(),
                manifests: vec![trust_manifest],
            },
            grant,
            collector,
            adapter_receipt,
            scheduler,
        }
    }

    fn ingest_request(&self) -> IngestRequest {
        let observation = GraphEntityObservation::new(
            &self.enterprise_id,
            EnterpriseEntityKind::Repository,
            AuthorityRef::new("github", "global", "42").unwrap(),
            BTreeSet::from(["acme/checkout".into()]),
            BTreeSet::from([self.adapter_receipt.safe_page_sha256.clone()]),
        )
        .unwrap();
        let event = EnterpriseEvent::new(
            self.enterprise_id.clone(),
            EnterpriseProvenance {
                machine_id: self.adapter_receipt.target.target_id.to_string(),
                run_id: "run-a".into(),
                adapter_instance_id: "github-acme".into(),
                auth_context_id: self.adapter_receipt.auth_context.context_id.to_string(),
                discovery_epoch: "epoch-1".into(),
                discovery_epoch_sequence: 1,
                source_sequence: 1,
                observed_at_ms: 1_100,
                source_fingerprint: self.adapter_receipt.safe_page_sha256.clone(),
            },
            EnterpriseFact::EntityObserved(observation),
        )
        .unwrap();
        let batch = EnterpriseBatch::new(self.enterprise_id.clone(), [event]).unwrap();
        let signed_batch = EnterpriseSignedBatch::sign(
            batch,
            &self.chain.manifests[0],
            self.grant.clone(),
            1_100,
            &self.collector,
        )
        .unwrap();
        IngestRequest::new(
            self.tenant_id.clone(),
            format!("outbox-attempt:{}", digest('b')),
            EnterpriseBatchBundle {
                trust_chain: self.chain.clone(),
                signed_batch,
            },
        )
        .unwrap()
    }
}

#[test]
fn adapter_receipt_batch_and_fenced_completion_commit_atomically() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new();
    let store =
        CoordinatorStore::open(root.path(), CoordinatorSigningKey::from_seed([42; 32])).unwrap();
    store
        .pin_enterprise(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.chain.anchor_manifest_id,
            &fixture.chain,
        )
        .unwrap();
    let manifest_id = fixture.scheduler.manifest().manifest_id.clone();
    store
        .initialize_scheduler(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.scheduler,
        )
        .unwrap();
    let claim = store
        .scheduler_claim(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id('1'),
            "machine-a",
            &BTreeSet::from([fixture.adapter_receipt.target.target_id.clone()]),
            1_000,
            1,
        )
        .unwrap()
        .result
        .remove(0);
    let completion = PageCompletion {
        task_id: claim.task.task_id,
        machine_id: "machine-a".into(),
        fence: claim.fence,
        completed_at_ms: 1_200,
        disposition: CompletionDisposition::Success { final_page: true },
        receipt_id: Some(fixture.adapter_receipt.receipt_id.to_string()),
        evidence_sha256: Some(fixture.adapter_receipt.safe_page_sha256.clone()),
        continuation: None,
        expansions: vec![],
    };
    let ingest = fixture.ingest_request();
    let batch_id = ingest.bundle.signed_batch.batch.batch_id.to_string();

    let mut stale = completion.clone();
    stale.fence += 1;
    assert!(store
        .commit_adapter_page(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id('2'),
            &fixture.adapter_receipt,
            &ingest,
            &stale,
            1_300,
        )
        .is_err());
    assert!(store
        .receipt(&fixture.tenant_id, &fixture.enterprise_id, &batch_id)
        .unwrap()
        .is_none());

    let committed = store
        .commit_adapter_page(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id('3'),
            &fixture.adapter_receipt,
            &ingest,
            &completion,
            1_300,
        )
        .unwrap();
    assert_eq!(committed.scheduler.receipt.terminal, 1);
    let replay = store
        .commit_adapter_page(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id('3'),
            &fixture.adapter_receipt,
            &ingest,
            &completion,
            1_300,
        )
        .unwrap();
    assert_eq!(replay, committed);

    let database =
        rusqlite::Connection::open(root.path().join("scout-coordinator.sqlite3")).unwrap();
    assert_eq!(table_count(&database, "ingest_receipts"), 1);
    assert_eq!(table_count(&database, "scheduler_page_commits"), 1);
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM scheduler_attempts
                 WHERE attempt_state = 'completed'",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
        1
    );
}

fn digest(character: char) -> String {
    std::iter::repeat_n(character, 64).collect()
}

fn operation_id(character: char) -> String {
    format!("scheduler-op:{}", digest(character))
}

fn table_count(database: &rusqlite::Connection, table: &str) -> u64 {
    database
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
