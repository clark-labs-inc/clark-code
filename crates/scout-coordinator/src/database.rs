use std::path::Path;
use std::time::Duration;

use exec_private_fs::PrivateFileOptions;
use rusqlite::Connection;

pub(super) const DATABASE_FILE: &str = "scout-coordinator.sqlite3";

pub(super) fn open(root: &Path) -> Result<Connection, String> {
    exec_private_fs::ensure_private_dir(root).map_err(|error| error.to_string())?;
    let path = root.join(DATABASE_FILE);
    let mut options = PrivateFileOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    let connection = Connection::open(&path).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(30))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS coordinator_meta (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 schema_version INTEGER NOT NULL,
                 coordinator_id TEXT NOT NULL,
                 coordinator_public_key TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS enterprise_pins (
                 tenant_id TEXT NOT NULL,
                 enterprise_id TEXT NOT NULL,
                 anchor_manifest_id TEXT NOT NULL,
                 trust_chain_json BLOB NOT NULL,
                 batch_accumulator_head_json BLOB NOT NULL,
                 next_sequence INTEGER NOT NULL CHECK (next_sequence > 0),
                 last_issued_at_ms INTEGER NOT NULL,
                 last_receipt_id TEXT,
                 PRIMARY KEY (tenant_id, enterprise_id)
             );
             CREATE TABLE IF NOT EXISTS accumulator_nodes (
                 tenant_id TEXT NOT NULL,
                 enterprise_id TEXT NOT NULL,
                 namespace TEXT NOT NULL,
                 node_digest TEXT NOT NULL,
                 node_json BLOB NOT NULL,
                 PRIMARY KEY (tenant_id, enterprise_id, namespace, node_digest),
                 FOREIGN KEY (tenant_id, enterprise_id)
                     REFERENCES enterprise_pins(tenant_id, enterprise_id)
             );
             CREATE TABLE IF NOT EXISTS ingest_receipts (
                 tenant_id TEXT NOT NULL,
                 enterprise_id TEXT NOT NULL,
                 batch_id TEXT NOT NULL,
                 envelope_sha256 TEXT NOT NULL,
                 bundle_json BLOB NOT NULL,
                 receipt_json BLOB NOT NULL,
                 sequence INTEGER NOT NULL CHECK (sequence > 0),
                 receipt_id TEXT NOT NULL UNIQUE,
                 PRIMARY KEY (tenant_id, enterprise_id, batch_id),
                 UNIQUE (tenant_id, enterprise_id, sequence),
                 FOREIGN KEY (tenant_id, enterprise_id)
                     REFERENCES enterprise_pins(tenant_id, enterprise_id)
             );
             CREATE TABLE IF NOT EXISTS scheduler_manifests (
                 tenant_id TEXT NOT NULL,
                 enterprise_id TEXT NOT NULL,
                 manifest_id TEXT NOT NULL,
                 manifest_json BLOB NOT NULL,
                 generation INTEGER NOT NULL CHECK (generation > 0),
                 state_sha256 TEXT NOT NULL,
                 PRIMARY KEY (tenant_id, enterprise_id, manifest_id),
                 FOREIGN KEY (tenant_id, enterprise_id)
                     REFERENCES enterprise_pins(tenant_id, enterprise_id)
             );
             CREATE TABLE IF NOT EXISTS scheduler_bindings (
                 tenant_id TEXT NOT NULL,
                 enterprise_id TEXT NOT NULL,
                 manifest_id TEXT NOT NULL,
                 binding_id TEXT NOT NULL,
                 binding_json BLOB NOT NULL,
                 PRIMARY KEY (
                     tenant_id, enterprise_id, manifest_id, binding_id
                 ),
                 FOREIGN KEY (tenant_id, enterprise_id, manifest_id)
                     REFERENCES scheduler_manifests(
                         tenant_id, enterprise_id, manifest_id
                     ) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS scheduler_tasks (
                 tenant_id TEXT NOT NULL,
                 enterprise_id TEXT NOT NULL,
                 manifest_id TEXT NOT NULL,
                 task_id TEXT NOT NULL,
                 binding_id TEXT NOT NULL,
                 spec_json BLOB NOT NULL,
                 status_json BLOB NOT NULL,
                 attempts INTEGER NOT NULL CHECK (attempts >= 0),
                 fence INTEGER NOT NULL CHECK (fence >= 0),
                 priority INTEGER NOT NULL CHECK (priority >= 0),
                 ready_at_ms INTEGER NOT NULL CHECK (ready_at_ms >= 0),
                 state_kind TEXT NOT NULL CHECK (
                     state_kind IN (
                         'pending', 'leased', 'retry_wait', 'terminal'
                     )
                 ),
                 target_id TEXT NOT NULL,
                 quota_key TEXT NOT NULL,
                 lease_machine_id TEXT,
                 lease_expires_at_ms INTEGER,
                 revision INTEGER NOT NULL CHECK (revision > 0),
                 PRIMARY KEY (
                     tenant_id, enterprise_id, manifest_id, task_id
                 ),
                 FOREIGN KEY (
                     tenant_id, enterprise_id, manifest_id, binding_id
                 ) REFERENCES scheduler_bindings(
                     tenant_id, enterprise_id, manifest_id, binding_id
                 ),
                 FOREIGN KEY (tenant_id, enterprise_id, manifest_id)
                     REFERENCES scheduler_manifests(
                         tenant_id, enterprise_id, manifest_id
                     ) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS scheduler_tasks_claim
                 ON scheduler_tasks (
                     tenant_id, enterprise_id, manifest_id, state_kind,
                     ready_at_ms, priority DESC, task_id
                 );
             CREATE INDEX IF NOT EXISTS scheduler_tasks_target_claim
                 ON scheduler_tasks (
                     tenant_id, enterprise_id, manifest_id, target_id,
                     state_kind, ready_at_ms, priority DESC, task_id
                 );
             CREATE TABLE IF NOT EXISTS scheduler_attempts (
                 tenant_id TEXT NOT NULL,
                 enterprise_id TEXT NOT NULL,
                 manifest_id TEXT NOT NULL,
                 task_id TEXT NOT NULL,
                 fence INTEGER NOT NULL CHECK (fence > 0),
                 machine_id TEXT NOT NULL,
                 attempt INTEGER NOT NULL CHECK (attempt > 0),
                 lease_expires_at_ms INTEGER NOT NULL,
                 attempt_state TEXT NOT NULL CHECK (
                     attempt_state IN (
                         'leased', 'completed', 'reaped'
                     )
                 ),
                 result_sha256 TEXT,
                 PRIMARY KEY (
                     tenant_id, enterprise_id, manifest_id, task_id, fence
                 ),
                 FOREIGN KEY (
                     tenant_id, enterprise_id, manifest_id, task_id
                 ) REFERENCES scheduler_tasks(
                     tenant_id, enterprise_id, manifest_id, task_id
                 ) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS scheduler_attempts_active
                 ON scheduler_attempts (
                     tenant_id, enterprise_id, manifest_id, attempt_state,
                     lease_expires_at_ms, task_id
                 );
             CREATE UNIQUE INDEX IF NOT EXISTS scheduler_attempts_one_active
                 ON scheduler_attempts (
                     tenant_id, enterprise_id, manifest_id, task_id
                 )
                 WHERE attempt_state = 'leased';
             CREATE TABLE IF NOT EXISTS scheduler_quotas (
                 tenant_id TEXT NOT NULL,
                 enterprise_id TEXT NOT NULL,
                 manifest_id TEXT NOT NULL,
                 quota_key TEXT NOT NULL,
                 policy_json BLOB NOT NULL,
                 next_start_at_ms INTEGER NOT NULL CHECK (
                     next_start_at_ms >= 0
                 ),
                 in_flight INTEGER NOT NULL CHECK (in_flight >= 0),
                 revision INTEGER NOT NULL CHECK (revision > 0),
                 PRIMARY KEY (
                     tenant_id, enterprise_id, manifest_id, quota_key
                 ),
                 FOREIGN KEY (tenant_id, enterprise_id, manifest_id)
                     REFERENCES scheduler_manifests(
                         tenant_id, enterprise_id, manifest_id
                     ) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS scheduler_operation_rows (
                 tenant_id TEXT NOT NULL,
                 enterprise_id TEXT NOT NULL,
                 manifest_id TEXT NOT NULL,
                 operation_id TEXT NOT NULL,
                 request_sha256 TEXT NOT NULL,
                 response_json BLOB NOT NULL,
                 generation INTEGER NOT NULL CHECK (generation > 0),
                 PRIMARY KEY (
                     tenant_id, enterprise_id, manifest_id, operation_id
                 ),
                 FOREIGN KEY (tenant_id, enterprise_id, manifest_id)
                     REFERENCES scheduler_manifests(
                         tenant_id, enterprise_id, manifest_id
                     ) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS scheduler_page_commits (
                 tenant_id TEXT NOT NULL,
                 enterprise_id TEXT NOT NULL,
                 manifest_id TEXT NOT NULL,
                 task_id TEXT NOT NULL,
                 fence INTEGER NOT NULL CHECK (fence > 0),
                 operation_id TEXT NOT NULL,
                 adapter_receipt_id TEXT NOT NULL,
                 safe_page_sha256 TEXT NOT NULL,
                 adapter_receipt_json BLOB NOT NULL,
                 batch_id TEXT NOT NULL,
                 ingest_receipt_id TEXT NOT NULL,
                 PRIMARY KEY (
                     tenant_id, enterprise_id, manifest_id, task_id, fence
                 ),
                 UNIQUE (
                     tenant_id, enterprise_id, manifest_id, operation_id
                 ),
                 UNIQUE (
                     tenant_id, enterprise_id, manifest_id, adapter_receipt_id
                 ),
                 FOREIGN KEY (
                     tenant_id, enterprise_id, manifest_id, task_id, fence
                 ) REFERENCES scheduler_attempts(
                     tenant_id, enterprise_id, manifest_id, task_id, fence
                 ),
                 FOREIGN KEY (
                     tenant_id, enterprise_id, manifest_id, operation_id
                 ) REFERENCES scheduler_operation_rows(
                     tenant_id, enterprise_id, manifest_id, operation_id
                 ),
                 FOREIGN KEY (tenant_id, enterprise_id, batch_id)
                     REFERENCES ingest_receipts(
                         tenant_id, enterprise_id, batch_id
                     ),
                 FOREIGN KEY (ingest_receipt_id)
                     REFERENCES ingest_receipts(receipt_id)
             );",
        )
        .map_err(|error| error.to_string())?;
    Ok(connection)
}
