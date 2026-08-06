use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Barrier};
use std::thread;

use agent_orchestration::{
    EnterpriseId, EnterpriseSigningKey, EnterpriseTrustChain, EnterpriseTrustManifest,
};
use scout_adapter_protocol::{
    AdapterId, AdapterQuery, AuthContextHandle, AuthContextId, CoverageBinding, SafeFieldValue,
    TargetId,
};
use scout_coordinator::CoordinatorStore;
use scout_ingest_protocol::{CoordinatorSigningKey, ScoutTenantId};
use scout_scheduler::{
    CompletionDisposition, PageCompletion, QuotaPolicy, RetryClass, ScheduleManifest, Scheduler,
    TaskOrigin, TaskSpec, TaskStatus,
};

fn digest(character: char) -> String {
    std::iter::repeat_n(character, 64).collect()
}

fn operation_id(sequence: usize) -> String {
    format!("scheduler-op:{sequence:064x}")
}

fn pin(store: &CoordinatorStore, tenant_id: &ScoutTenantId, enterprise_id: &EnterpriseId) {
    let administrator = EnterpriseSigningKey::from_seed([7; 32]);
    let manifest = EnterpriseTrustManifest::initial(
        enterprise_id.clone(),
        format!("trust:{}", digest('a')),
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
        .pin_enterprise(tenant_id, enterprise_id, &chain.anchor_manifest_id, &chain)
        .unwrap();
}

struct ScheduleFixture {
    enterprise_id: EnterpriseId,
    tenant_id: ScoutTenantId,
    target_id: TargetId,
    scheduler: Scheduler,
}

impl ScheduleFixture {
    fn new(task_count: usize, max_in_flight: u16, max_attempts: u16) -> Self {
        let enterprise_id = EnterpriseId::new("acme").unwrap();
        let tenant_id = ScoutTenantId::new("organization:acme").unwrap();
        let target_id = TargetId::new(format!("target:{}", digest('b'))).unwrap();
        let adapter_id = AdapterId::new("clark/github@1").unwrap();
        let auth_context_id = AuthContextId::new(format!("authctx:{}", digest('c'))).unwrap();
        let auth_context_handle =
            AuthContextHandle::new("auth:00000000-0000-4000-8000-000000000001").unwrap();
        let query = AdapterQuery {
            operation: "list_repositories".into(),
            authority_scope: "organization/acme".into(),
            provider_resource_type: "repository".into(),
            filters: BTreeMap::<String, SafeFieldValue>::new(),
            projection: BTreeSet::from(["native_id".into()]),
            page_size: 100,
        };
        let roots = (0..task_count)
            .map(|index| {
                TaskSpec::new(
                    enterprise_id.as_str(),
                    "charter-acme",
                    1,
                    target_id.clone(),
                    adapter_id.clone(),
                    auth_context_id.clone(),
                    auth_context_handle.clone(),
                    CoverageBinding {
                        enterprise_id: enterprise_id.as_str().into(),
                        charter_id: "charter-acme".into(),
                        discovery_epoch: 1,
                        sequence: index as u64 + 1,
                        adapter_id: adapter_id.clone(),
                        auth_context_id: auth_context_id.clone(),
                        tenant: "organization/acme".into(),
                        region_or_project: format!("partition-{index:06}"),
                        resource_kind: "repository".into(),
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
        let root_ids = roots
            .iter()
            .map(|root| root.task_id.clone())
            .collect::<BTreeSet<_>>();
        let quota_policies = BTreeMap::from([(
            roots[0].quota_key(),
            QuotaPolicy {
                max_in_flight,
                min_start_interval_ms: 0,
                lease_duration_ms: 1_000,
                base_backoff_ms: 100,
                max_backoff_ms: 10_000,
                max_attempts,
            },
        )]);
        let manifest = ScheduleManifest::new(
            enterprise_id.as_str(),
            "charter-acme",
            1,
            root_ids,
            quota_policies,
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

fn coordinator_store(root: &std::path::Path) -> CoordinatorStore {
    CoordinatorStore::open(root, CoordinatorSigningKey::from_seed([42; 32])).unwrap()
}

#[test]
fn concurrent_workers_claim_each_task_once_and_retries_are_byte_identical() {
    const WORKERS: usize = 24;
    let root = tempfile::tempdir().unwrap();
    let fixture = ScheduleFixture::new(WORKERS, WORKERS as u16, 3);
    let store = coordinator_store(root.path());
    pin(&store, &fixture.tenant_id, &fixture.enterprise_id);
    let manifest_id = fixture.scheduler.manifest().manifest_id.clone();
    store
        .initialize_scheduler(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.scheduler,
        )
        .unwrap();
    let barrier = Arc::new(Barrier::new(WORKERS));
    let mut workers = Vec::new();
    for index in 0..WORKERS {
        let worker = store.clone();
        let barrier = Arc::clone(&barrier);
        let tenant_id = fixture.tenant_id.clone();
        let enterprise_id = fixture.enterprise_id.clone();
        let manifest_id = manifest_id.clone();
        let eligible = BTreeSet::from([fixture.target_id.clone()]);
        workers.push(thread::spawn(move || {
            barrier.wait();
            worker.scheduler_claim(
                &tenant_id,
                &enterprise_id,
                &manifest_id,
                &operation_id(index + 1),
                &format!("machine-{index:02}"),
                &eligible,
                100,
                1,
            )
        }));
    }
    let claims = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .collect::<Vec<_>>();
    let task_ids = claims
        .iter()
        .flat_map(|claim| claim.result.iter())
        .map(|claim| claim.task.task_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(task_ids.len(), WORKERS);
    assert!(claims
        .iter()
        .all(|claim| claim.result.len() == 1 && claim.result[0].fence == 1));
    let receipt = store
        .scheduler_receipt(&fixture.tenant_id, &fixture.enterprise_id, &manifest_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.tasks, WORKERS);
    assert_eq!(receipt.leased, WORKERS);

    let retried = store
        .scheduler_claim(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id(1),
            "machine-00",
            &BTreeSet::from([fixture.target_id.clone()]),
            100,
            1,
        )
        .unwrap();
    assert_eq!(retried, claims[0]);
    assert!(store
        .scheduler_claim(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id(1),
            "machine-00",
            &BTreeSet::from([fixture.target_id.clone()]),
            101,
            1,
        )
        .is_err());
    let database =
        rusqlite::Connection::open(root.path().join("scout-coordinator.sqlite3")).unwrap();
    assert_eq!(table_count(&database, "scheduler_tasks"), WORKERS as u64);
    assert_eq!(table_count(&database, "scheduler_attempts"), WORKERS as u64);
    assert_eq!(
        table_count(&database, "scheduler_operation_rows"),
        WORKERS as u64
    );
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM scheduler_tasks WHERE revision = 2",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
        WORKERS as u64
    );
}

#[test]
fn restart_preserves_retry_state_and_rejects_a_stale_worker() {
    let root = tempfile::tempdir().unwrap();
    let fixture = ScheduleFixture::new(1, 1, 2);
    let store = coordinator_store(root.path());
    pin(&store, &fixture.tenant_id, &fixture.enterprise_id);
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
            &operation_id(1),
            "machine-a",
            &BTreeSet::from([fixture.target_id.clone()]),
            20,
            1,
        )
        .unwrap()
        .result
        .remove(0);
    store
        .scheduler_reap(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id(2),
            1_021,
        )
        .unwrap();
    drop(store);

    let reopened = coordinator_store(root.path());
    let stale = PageCompletion {
        task_id: claim.task.task_id.clone(),
        machine_id: claim.machine_id,
        fence: claim.fence,
        completed_at_ms: 900,
        disposition: CompletionDisposition::Retry {
            class: RetryClass::TransientTransport,
            retry_after_ms: None,
            error_sha256: digest('e'),
        },
        receipt_id: None,
        evidence_sha256: None,
        continuation: None,
        expansions: vec![],
    };
    assert!(reopened
        .scheduler_complete(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id(3),
            &stale,
        )
        .is_err());
    let receipt = reopened
        .scheduler_receipt(&fixture.tenant_id, &fixture.enterprise_id, &manifest_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.retry_wait, 1);
    let retry = reopened
        .scheduler_claim(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id(4),
            "machine-b",
            &BTreeSet::from([fixture.target_id.clone()]),
            1_121,
            1,
        )
        .unwrap()
        .result
        .remove(0);
    assert!(retry.fence > stale.fence);
    assert!(matches!(
        retry.task.task_id,
        task_id if matches!(
            fixture.scheduler.task_status(&task_id),
            Some(TaskStatus::Pending { .. })
        )
    ));
    let database =
        rusqlite::Connection::open(root.path().join("scout-coordinator.sqlite3")).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM scheduler_attempts
                 WHERE attempt_state = 'reaped'",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM scheduler_attempts
                 WHERE attempt_state = 'leased'",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn normalized_transitions_touch_only_affected_rows_and_preserve_attempt_history() {
    let root = tempfile::tempdir().unwrap();
    let fixture = ScheduleFixture::new(2, 2, 3);
    let store = coordinator_store(root.path());
    pin(&store, &fixture.tenant_id, &fixture.enterprise_id);
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
            &operation_id(1),
            "machine-a",
            &BTreeSet::from([fixture.target_id.clone()]),
            100,
            1,
        )
        .unwrap()
        .result
        .remove(0);
    let renewed = store
        .scheduler_heartbeat(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id(2),
            &claim.task.task_id,
            "machine-a",
            claim.fence,
            200,
        )
        .unwrap();
    let completion = PageCompletion {
        task_id: claim.task.task_id.clone(),
        machine_id: "machine-a".into(),
        fence: claim.fence,
        completed_at_ms: 300,
        disposition: CompletionDisposition::Retry {
            class: RetryClass::TransientTransport,
            retry_after_ms: None,
            error_sha256: digest('e'),
        },
        receipt_id: None,
        evidence_sha256: None,
        continuation: None,
        expansions: vec![],
    };
    let completed = store
        .scheduler_complete(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &manifest_id,
            &operation_id(3),
            &completion,
        )
        .unwrap();
    assert_eq!(completed.receipt.retry_wait, 1);
    assert_eq!(renewed.result, 1_200);

    let database =
        rusqlite::Connection::open(root.path().join("scout-coordinator.sqlite3")).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM scheduler_tasks WHERE revision = 4",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM scheduler_tasks WHERE revision = 1",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
        1
    );
    let attempt = database
        .query_row(
            "SELECT fence, attempt, lease_expires_at_ms, attempt_state,
                    result_sha256 IS NOT NULL
             FROM scheduler_attempts",
            [],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, u16>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        attempt,
        (
            claim.fence,
            claim.attempt,
            renewed.result,
            "completed".into(),
            true
        )
    );
}

#[test]
fn scheduler_state_is_tenant_isolated_and_requires_a_pin() {
    let root = tempfile::tempdir().unwrap();
    let fixture = ScheduleFixture::new(1, 1, 2);
    let store = coordinator_store(root.path());
    assert!(store
        .initialize_scheduler(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.scheduler,
        )
        .is_err());
    pin(&store, &fixture.tenant_id, &fixture.enterprise_id);
    let receipt = store
        .initialize_scheduler(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.scheduler,
        )
        .unwrap();
    let same = store
        .initialize_scheduler(
            &fixture.tenant_id,
            &fixture.enterprise_id,
            &fixture.scheduler,
        )
        .unwrap();
    assert_eq!(same, receipt);
    let other_tenant = ScoutTenantId::new("organization:other").unwrap();
    assert!(store
        .scheduler_receipt(
            &other_tenant,
            &fixture.enterprise_id,
            &fixture.scheduler.manifest().manifest_id,
        )
        .unwrap()
        .is_none());
}

fn table_count(connection: &rusqlite::Connection, table: &str) -> u64 {
    assert!(matches!(
        table,
        "scheduler_manifests"
            | "scheduler_tasks"
            | "scheduler_attempts"
            | "scheduler_operation_rows"
    ));
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
