use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

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
use serde::Serialize;
use sha2::{Digest, Sha256};

struct Args {
    tasks: usize,
    claim_batch: usize,
    output: PathBuf,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut tasks = 10_000;
        let mut claim_batch = 1_024;
        let mut output = None;
        let mut args = std::env::args().skip(1);
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--tasks" => {
                    tasks = args
                        .next()
                        .ok_or("--tasks requires a value")?
                        .parse()
                        .map_err(|_| "--tasks must be an integer")?;
                }
                "--claim-batch" => {
                    claim_batch = args
                        .next()
                        .ok_or("--claim-batch requires a value")?
                        .parse()
                        .map_err(|_| "--claim-batch must be an integer")?;
                }
                "--out" => {
                    output = Some(PathBuf::from(args.next().ok_or("--out requires a path")?));
                }
                "--help" | "-h" => {
                    println!(
                        "scheduler_scale --out NEW_DIRECTORY \
                         [--tasks 10000] [--claim-batch 1024]"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown scheduler scale argument: {other}")),
            }
        }
        if tasks == 0 || tasks > 100_000 {
            return Err("--tasks must be in 1..=100000".into());
        }
        if claim_batch == 0 || claim_batch > tasks || claim_batch > 1_024 {
            return Err("--claim-batch must be in 1..=min(tasks,1024)".into());
        }
        Ok(Self {
            tasks,
            claim_batch,
            output: output.ok_or("--out is required")?,
        })
    }
}

#[derive(Serialize)]
struct Receipt {
    schema_version: u16,
    status: &'static str,
    tasks: usize,
    claim_batch: usize,
    task_build_ms: u128,
    scheduler_build_ms: u128,
    scheduler_json_bytes: usize,
    scheduler_encode_ms: u128,
    reference_claim_ms: u128,
    coordinator_initialize_ms: u128,
    coordinator_claim_ms: u128,
    idempotent_retry_ms: u128,
    restart_receipt_ms: u128,
    coordinator_state_bytes: u64,
    normalized_manifest_rows: u64,
    normalized_binding_rows: u64,
    normalized_task_rows: u64,
    normalized_attempt_rows: u64,
    normalized_operation_rows: u64,
    mutated_task_rows: u64,
    untouched_task_rows: u64,
    initial_state_sha256: String,
    claimed_state_sha256: String,
    claimed_unique_tasks: usize,
    exact_reference_match: bool,
    byte_identical_retry: bool,
    restart_receipt_matches: bool,
    claim_latency_budget_ms: u128,
    restart_latency_budget_ms: u128,
    scale_latency_gate_passed: bool,
    semantic_sha256: String,
}

struct NormalizedMetrics {
    manifests: u64,
    bindings: u64,
    tasks: u64,
    attempts: u64,
    operations: u64,
    mutated_tasks: u64,
    untouched_tasks: u64,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Scout scheduler scale gate failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse()?;
    if args.output.exists() {
        return Err(format!(
            "refusing to overwrite scheduler scale output {}",
            args.output.display()
        ));
    }
    std::fs::create_dir_all(&args.output).map_err(|error| error.to_string())?;
    let state_root = args.output.join("state");

    let enterprise_id = EnterpriseId::new("scheduler-scale").map_err(|error| error.to_string())?;
    let tenant_id =
        ScoutTenantId::new("organization:scheduler-scale").map_err(|error| error.to_string())?;
    let target_id =
        TargetId::new(format!("target:{}", "a".repeat(64))).map_err(|error| error.to_string())?;
    let adapter_id = AdapterId::new("clark/github@1").map_err(|error| error.to_string())?;
    let auth_context_id = AuthContextId::new(format!("authctx:{}", "b".repeat(64)))
        .map_err(|error| error.to_string())?;
    let auth_context_handle = AuthContextHandle::new("auth:00000000-0000-4000-8000-000000000001")
        .map_err(|error| error.to_string())?;
    let query = AdapterQuery {
        operation: "list_repositories".into(),
        authority_scope: "organization/scheduler-scale".into(),
        provider_resource_type: "repository".into(),
        filters: BTreeMap::<String, SafeFieldValue>::new(),
        projection: BTreeSet::from(["native_id".into()]),
        page_size: 1_000,
    };

    let started = Instant::now();
    let roots = (0..args.tasks)
        .map(|index| {
            let coverage = CoverageBinding {
                enterprise_id: enterprise_id.as_str().into(),
                charter_id: "charter-scale".into(),
                discovery_epoch: 1,
                sequence: index as u64 + 1,
                adapter_id: adapter_id.clone(),
                auth_context_id: auth_context_id.clone(),
                tenant: query.authority_scope.clone(),
                region_or_project: format!("partition-{index:06}"),
                resource_kind: query.provider_resource_type.clone(),
            };
            TaskSpec::new(
                enterprise_id.as_str(),
                "charter-scale",
                1,
                target_id.clone(),
                adapter_id.clone(),
                auth_context_id.clone(),
                auth_context_handle.clone(),
                coverage,
                query.clone(),
                0,
                None,
                TaskOrigin::Root,
                100,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let task_build_ms = started.elapsed().as_millis();
    let quota_key = roots
        .first()
        .ok_or_else(|| "scheduler scale fixture has no roots".to_string())?
        .quota_key();
    let manifest = ScheduleManifest::new(
        enterprise_id.as_str(),
        "charter-scale",
        1,
        roots
            .iter()
            .map(|task| task.task_id.clone())
            .collect::<BTreeSet<_>>(),
        BTreeMap::from([(
            quota_key,
            QuotaPolicy {
                max_in_flight: args.claim_batch as u16,
                min_start_interval_ms: 0,
                lease_duration_ms: 60_000,
                base_backoff_ms: 1_000,
                max_backoff_ms: 60_000,
                max_attempts: 3,
            },
        )]),
        BTreeMap::new(),
    )?;
    let started = Instant::now();
    let scheduler = Scheduler::new(manifest, roots, 10)?;
    let scheduler_build_ms = started.elapsed().as_millis();
    let initial_receipt = scheduler.receipt()?;
    let started = Instant::now();
    let scheduler_json_bytes = scheduler.encode()?.len();
    let scheduler_encode_ms = started.elapsed().as_millis();
    let eligible_targets = BTreeSet::from([target_id]);
    let mut reference = scheduler.clone();
    let started = Instant::now();
    let expected_claim =
        reference.claim("machine-scale", &eligible_targets, 100, args.claim_batch)?;
    let expected_receipt = reference.receipt()?;
    let reference_claim_ms = started.elapsed().as_millis();

    let store = CoordinatorStore::open(&state_root, CoordinatorSigningKey::from_seed([42; 32]))?;
    pin(&store, &tenant_id, &enterprise_id)?;
    let started = Instant::now();
    store.initialize_scheduler(&tenant_id, &enterprise_id, &scheduler)?;
    let coordinator_initialize_ms = started.elapsed().as_millis();
    let manifest_id = scheduler.manifest().manifest_id.clone();
    let operation_id = format!("scheduler-op:{}", "1".repeat(64));
    let started = Instant::now();
    let claim = store.scheduler_claim(
        &tenant_id,
        &enterprise_id,
        &manifest_id,
        &operation_id,
        "machine-scale",
        &eligible_targets,
        100,
        args.claim_batch,
    )?;
    let coordinator_claim_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let retried = store.scheduler_claim(
        &tenant_id,
        &enterprise_id,
        &manifest_id,
        &operation_id,
        "machine-scale",
        &eligible_targets,
        100,
        args.claim_batch,
    )?;
    let idempotent_retry_ms = started.elapsed().as_millis();
    let claimed_unique_tasks = claim
        .result
        .iter()
        .map(|lease| lease.task.task_id.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let exact_reference_match = claim.result == expected_claim && claim.receipt == expected_receipt;
    let byte_identical_retry = claim == retried;
    drop(store);

    let reopened = CoordinatorStore::open(&state_root, CoordinatorSigningKey::from_seed([42; 32]))?;
    let started = Instant::now();
    let restarted = reopened
        .scheduler_receipt(&tenant_id, &enterprise_id, &manifest_id)?
        .ok_or_else(|| "scheduler receipt disappeared after restart".to_string())?;
    let restart_receipt_ms = started.elapsed().as_millis();
    let restart_receipt_matches = restarted == claim.receipt;
    let normalized = normalized_metrics(
        &state_root.join("scout-coordinator.sqlite3"),
        &tenant_id,
        &enterprise_id,
        &manifest_id,
    )?;
    let coordinator_state_bytes = directory_bytes(&state_root)?;
    let semantic_sha256 = sha256(
        &serde_json::to_vec(&(
            "scout-scheduler-scale-v4",
            args.tasks,
            args.claim_batch,
            &initial_receipt.state_sha256,
            &claim.receipt.state_sha256,
            claimed_unique_tasks,
            exact_reference_match,
            byte_identical_retry,
            restart_receipt_matches,
            normalized.manifests,
            normalized.tasks,
            normalized.attempts,
            normalized.operations,
            normalized.mutated_tasks,
        ))
        .map_err(|error| error.to_string())?,
    );
    let claim_latency_budget_ms =
        500_u128.max((args.tasks as u128).saturating_mul(10_000) / 100_000);
    let restart_latency_budget_ms =
        250_u128.max((args.tasks as u128).saturating_mul(5_000) / 100_000);
    let scale_latency_gate_passed = coordinator_claim_ms <= claim_latency_budget_ms
        && restart_receipt_ms <= restart_latency_budget_ms;
    let passed = claimed_unique_tasks == args.claim_batch
        && exact_reference_match
        && byte_identical_retry
        && restart_receipt_matches
        && claim.receipt.tasks == args.tasks
        && claim.receipt.leased == args.claim_batch
        && normalized.manifests == 1
        && normalized.tasks == args.tasks as u64
        && normalized.attempts == args.claim_batch as u64
        && normalized.operations == 1
        && normalized.mutated_tasks == args.claim_batch as u64
        && normalized.untouched_tasks == (args.tasks - args.claim_batch) as u64
        && scale_latency_gate_passed;
    let receipt = Receipt {
        schema_version: 3,
        status: if passed { "passed" } else { "failed" },
        tasks: args.tasks,
        claim_batch: args.claim_batch,
        task_build_ms,
        scheduler_build_ms,
        scheduler_json_bytes,
        scheduler_encode_ms,
        reference_claim_ms,
        coordinator_initialize_ms,
        coordinator_claim_ms,
        idempotent_retry_ms,
        restart_receipt_ms,
        coordinator_state_bytes,
        normalized_manifest_rows: normalized.manifests,
        normalized_binding_rows: normalized.bindings,
        normalized_task_rows: normalized.tasks,
        normalized_attempt_rows: normalized.attempts,
        normalized_operation_rows: normalized.operations,
        mutated_task_rows: normalized.mutated_tasks,
        untouched_task_rows: normalized.untouched_tasks,
        initial_state_sha256: initial_receipt.state_sha256,
        claimed_state_sha256: claim.receipt.state_sha256,
        claimed_unique_tasks,
        exact_reference_match,
        byte_identical_retry,
        restart_receipt_matches,
        claim_latency_budget_ms,
        restart_latency_budget_ms,
        scale_latency_gate_passed,
        semantic_sha256,
    };
    std::fs::write(
        args.output.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    println!("receipt={}", args.output.join("receipt.json").display());
    println!("status={}", receipt.status);
    if passed {
        Ok(())
    } else {
        Err("scheduler scale invariants failed".into())
    }
}

fn pin(
    store: &CoordinatorStore,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
) -> Result<(), String> {
    let administrator = EnterpriseSigningKey::from_seed([7; 32]);
    let manifest = EnterpriseTrustManifest::initial(
        enterprise_id.clone(),
        format!("trust:{}", "c".repeat(64)),
        100,
        100_000,
        &administrator,
    )?;
    let chain = EnterpriseTrustChain {
        anchor_manifest_id: manifest.manifest_id.clone(),
        manifests: vec![manifest],
    };
    store.pin_enterprise(tenant_id, enterprise_id, &chain.anchor_manifest_id, &chain)?;
    Ok(())
}

fn directory_bytes(root: &Path) -> Result<u64, String> {
    let mut total = 0_u64;
    for entry in std::fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            total =
                total.saturating_add(entry.metadata().map_err(|error| error.to_string())?.len());
        }
    }
    Ok(total)
}

fn normalized_metrics(
    path: &Path,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
    manifest_id: &str,
) -> Result<NormalizedMetrics, String> {
    let connection = rusqlite::Connection::open(path).map_err(|error| error.to_string())?;
    let scoped_count = |table: &str, suffix: &str| -> Result<u64, String> {
        if !matches!(
            table,
            "scheduler_manifests"
                | "scheduler_bindings"
                | "scheduler_tasks"
                | "scheduler_attempts"
                | "scheduler_operation_rows"
        ) {
            return Err("unsupported scheduler scale metrics table".into());
        }
        connection
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM {table}
                     WHERE tenant_id = ?1 AND enterprise_id = ?2
                       AND manifest_id = ?3 {suffix}"
                ),
                rusqlite::params![tenant_id.as_str(), enterprise_id.as_str(), manifest_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    };
    Ok(NormalizedMetrics {
        manifests: scoped_count("scheduler_manifests", "")?,
        bindings: scoped_count("scheduler_bindings", "")?,
        tasks: scoped_count("scheduler_tasks", "")?,
        attempts: scoped_count("scheduler_attempts", "")?,
        operations: scoped_count("scheduler_operation_rows", "")?,
        mutated_tasks: scoped_count("scheduler_tasks", "AND revision > 1")?,
        untouched_tasks: scoped_count("scheduler_tasks", "AND revision = 1")?,
    })
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
