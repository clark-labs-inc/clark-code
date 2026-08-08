use std::collections::{BTreeMap, BTreeSet};

use agent_orchestration::{
    EnterpriseId, EnterpriseSigningKey, EnterpriseTrustChain, EnterpriseTrustManifest,
};
use scout_adapter_protocol::{
    AdapterId, AdapterQuery, AuthContextHandle, AuthContextId, CoverageBinding, SafeFieldValue,
    TargetId,
};
use scout_coordinator::CoordinatorStore;
use scout_ingest_protocol::{CoordinatorSigningKey, ScoutTenantId};
use scout_scheduler::{QuotaPolicy, ScheduleManifest, Scheduler, TaskOrigin, TaskSpec};

struct Fixture {
    enterprise_id: EnterpriseId,
    tenant_id: ScoutTenantId,
    target_id: TargetId,
    scheduler: Scheduler,
}

impl Fixture {
    fn new(task_count: usize, max_in_flight: u16) -> Self {
        let enterprise_id = EnterpriseId::new("fast-path").unwrap();
        let tenant_id = ScoutTenantId::new("organization:fast-path").unwrap();
        let target_id = TargetId::new(format!("target:{}", "b".repeat(64))).unwrap();
        let adapter_id = AdapterId::new("clark/github@1").unwrap();
        let auth_context_id = AuthContextId::new(format!("authctx:{}", "c".repeat(64))).unwrap();
        let auth_context_handle =
            AuthContextHandle::new("auth:00000000-0000-4000-8000-000000000001").unwrap();
        let query = AdapterQuery {
            operation: "list_repositories".into(),
            authority_scope: "organization/fast-path".into(),
            provider_resource_type: "repository".into(),
            filters: BTreeMap::<String, SafeFieldValue>::new(),
            projection: BTreeSet::from(["native_id".into()]),
            page_size: 100,
        };
        let roots = (0..task_count)
            .map(|index| {
                TaskSpec::new(
                    enterprise_id.as_str(),
                    "charter-fast-path",
                    1,
                    target_id.clone(),
                    adapter_id.clone(),
                    auth_context_id.clone(),
                    auth_context_handle.clone(),
                    CoverageBinding {
                        enterprise_id: enterprise_id.as_str().into(),
                        charter_id: "charter-fast-path".into(),
                        discovery_epoch: 1,
                        sequence: index as u64 + 1,
                        adapter_id: adapter_id.clone(),
                        auth_context_id: auth_context_id.clone(),
                        tenant: query.authority_scope.clone(),
                        region_or_project: format!("partition-{index:06}"),
                        resource_kind: query.provider_resource_type.clone(),
                    },
                    query.clone(),
                    0,
                    None,
                    TaskOrigin::Root,
                    100,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let quota_key = roots[0].quota_key();
        let manifest = ScheduleManifest::new(
            enterprise_id.as_str(),
            "charter-fast-path",
            1,
            roots.iter().map(|task| task.task_id.clone()).collect(),
            BTreeMap::from([(
                quota_key,
                QuotaPolicy {
                    max_in_flight,
                    min_start_interval_ms: 0,
                    lease_duration_ms: 1_000,
                    base_backoff_ms: 100,
                    max_backoff_ms: 10_000,
                    max_attempts: 3,
                },
            )]),
            BTreeMap::new(),
        )
        .unwrap();
        let scheduler = Scheduler::new(manifest, roots, 10).unwrap();
        Self {
            enterprise_id,
            tenant_id,
            target_id,
            scheduler,
        }
    }
}

fn operation_id(sequence: usize) -> String {
    format!("scheduler-op:{sequence:064x}")
}

fn open_store(root: &std::path::Path) -> CoordinatorStore {
    CoordinatorStore::open(root, CoordinatorSigningKey::from_seed([42; 32])).unwrap()
}

fn pin(store: &CoordinatorStore, fixture: &Fixture) {
    let administrator = EnterpriseSigningKey::from_seed([7; 32]);
    let manifest = EnterpriseTrustManifest::initial(
        fixture.enterprise_id.clone(),
        format!("trust:{}", "a".repeat(64)),
        100,
        100_000,
        &administrator,
    )
    .unwrap();
    let chain = EnterpriseTrustChain {
        anchor_manifest_id: manifest.manifest_id.clone(),
        manifests: vec![manifest],
    };
    store
        .pin_enterprise(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &chain.anchor_manifest_id,
            &chain,
        )
        .unwrap();
}

#[test]
fn normalized_claim_matches_reference_roots_across_reap_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(16, 8);
    let mut reference = fixture.scheduler.clone();
    let store = open_store(root.path());
    pin(&store, &fixture);
    let manifest_id = fixture.scheduler.manifest().manifest_id.clone();
    store
        .initialize_scheduler(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.scheduler,
        )
        .unwrap();
    let eligible = BTreeSet::from([fixture.target_id.clone()]);
    for (sequence, machine, now_ms) in [(1, "machine-a", 100), (2, "machine-b", 1_101)] {
        let expected = reference.claim(machine, &eligible, now_ms, 8).unwrap();
        let observed = store
            .scheduler_claim(
                &fixture.tenant_id,
                &fixture.enterprise_id,
                &manifest_id,
                &operation_id(sequence),
                machine,
                &eligible,
                now_ms,
                8,
            )
            .unwrap();
        assert_eq!(observed.result, expected);
        assert_eq!(observed.receipt, reference.receipt().unwrap());
    }
    drop(store);
    assert_eq!(
        open_store(root.path())
            .scheduler_receipt(&fixture.tenant_id, &fixture.enterprise_id, &manifest_id)
            .unwrap()
            .unwrap(),
        reference.receipt().unwrap()
    );
}

#[test]
fn normalized_claim_rolls_back_every_row_when_attempt_insert_fails() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(4, 4);
    let store = open_store(root.path());
    pin(&store, &fixture);
    let manifest_id = fixture.scheduler.manifest().manifest_id.clone();
    let initial = store
        .initialize_scheduler(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.scheduler,
        )
        .unwrap();
    let database_path = root.path().join("scout-coordinator.sqlite3");
    let database = rusqlite::Connection::open(&database_path).unwrap();
    database
        .execute_batch(
            "CREATE TRIGGER abort_scheduler_attempt
             BEFORE INSERT ON scheduler_attempts
             BEGIN
                 SELECT RAISE(ABORT, 'injected attempt persistence failure');
             END;",
        )
        .unwrap();
    drop(database);
    assert!(store
        .scheduler_claim(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id(1),
            "machine-a",
            &BTreeSet::from([fixture.target_id.clone()]),
            100,
            4,
        )
        .is_err());
    let database = rusqlite::Connection::open(&database_path).unwrap();
    database
        .execute_batch("DROP TRIGGER abort_scheduler_attempt;")
        .unwrap();
    let changed = database
        .query_row(
            "SELECT COUNT(*) FROM scheduler_tasks WHERE revision != 1",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap();
    let attempts = database
        .query_row("SELECT COUNT(*) FROM scheduler_attempts", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap();
    let operations = database
        .query_row("SELECT COUNT(*) FROM scheduler_operation_rows", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap();
    assert_eq!((changed, attempts, operations), (0, 0, 0));
    drop(database);
    assert_eq!(
        store
            .scheduler_receipt(&fixture.tenant_id, &fixture.enterprise_id, &manifest_id)
            .unwrap()
            .unwrap(),
        initial
    );
}
