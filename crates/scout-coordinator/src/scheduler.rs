use std::collections::BTreeSet;

use agent_orchestration::EnterpriseId;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use scout_adapter_protocol::{AdapterPageReceipt, TargetId};
use scout_ingest_protocol::{IngestReceipt, IngestRequest, ScoutTenantId};
use scout_scheduler::{
    CompletionDisposition, LeaseClaim, PageCompletion, Scheduler, SchedulerReceipt, SchedulerTaskId,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{database, CoordinatorStore};

mod claim;
mod model;
mod page_commit;
mod receipt;
mod storage;

use model::{canonical_sha256, SchedulerImage};
use page_commit::{persist_page_commit, validate_atomic_page_inputs, validate_task_binding};
use storage::{load_scheduler, persist_mutation, SchedulerScope};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "T: Serialize",
    deserialize = "T: serde::de::DeserializeOwned"
))]
pub struct SchedulerMutation<T> {
    pub operation_id: String,
    pub manifest_id: String,
    pub result: T,
    pub receipt: SchedulerReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPageCommit {
    pub ingest_receipt: IngestReceipt,
    pub scheduler: SchedulerMutation<()>,
}

impl CoordinatorStore {
    pub fn initialize_scheduler(
        &self,
        authorized_tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        scheduler: &Scheduler,
    ) -> Result<SchedulerReceipt, String> {
        scheduler.validate()?;
        if scheduler.manifest().enterprise_id != enterprise_id.as_str() {
            return Err("scheduler enterprise does not match the coordinator request".into());
        }
        let mut connection = database::open(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        require_enterprise_pin(&transaction, authorized_tenant_id, enterprise_id)?;
        let manifest_id = &scheduler.manifest().manifest_id;
        let scope = SchedulerScope {
            tenant_id: authorized_tenant_id.as_str(),
            enterprise_id: enterprise_id.as_str(),
            manifest_id,
        };
        let image = SchedulerImage::from_scheduler(scheduler)?;
        if let Some(existing) = load_scheduler(&transaction, scope)? {
            if existing != image {
                return Err(
                    "scheduler manifest is already initialized with different state".into(),
                );
            }
            return existing.receipt();
        }
        storage::insert_scheduler(&transaction, scope, &image)?;
        let receipt = image.receipt()?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(receipt)
    }

    pub fn scheduler_receipt(
        &self,
        authorized_tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        manifest_id: &str,
    ) -> Result<Option<SchedulerReceipt>, String> {
        let connection = database::open(&self.root)?;
        receipt::verified_receipt(
            &connection,
            SchedulerScope {
                tenant_id: authorized_tenant_id.as_str(),
                enterprise_id: enterprise_id.as_str(),
                manifest_id,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn scheduler_claim(
        &self,
        authorized_tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        manifest_id: &str,
        operation_id: &str,
        machine_id: &str,
        eligible_targets: &BTreeSet<TargetId>,
        now_ms: u64,
        max_tasks: usize,
    ) -> Result<SchedulerMutation<Vec<LeaseClaim>>, String> {
        validate_operation_id(operation_id)?;
        let request_sha256 =
            canonical_sha256(&("claim", machine_id, eligible_targets, now_ms, max_tasks))?;
        let mut connection = database::open(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        require_enterprise_pin(&transaction, authorized_tenant_id, enterprise_id)?;
        if let Some(stored) = read_operation::<Vec<LeaseClaim>>(
            &transaction,
            authorized_tenant_id,
            enterprise_id,
            manifest_id,
            operation_id,
            &request_sha256,
        )? {
            return Ok(stored);
        }
        let scope = SchedulerScope {
            tenant_id: authorized_tenant_id.as_str(),
            enterprise_id: enterprise_id.as_str(),
            manifest_id,
        };
        let (result, receipt) = claim::claim(
            &transaction,
            scope,
            machine_id,
            eligible_targets,
            now_ms,
            max_tasks,
        )?;
        let response = SchedulerMutation {
            operation_id: operation_id.to_owned(),
            manifest_id: manifest_id.to_owned(),
            result,
            receipt,
        };
        transaction
            .execute(
                "INSERT INTO scheduler_operation_rows (
                     tenant_id, enterprise_id, manifest_id, operation_id,
                     request_sha256, response_json, generation
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    authorized_tenant_id.as_str(),
                    enterprise_id.as_str(),
                    manifest_id,
                    operation_id,
                    request_sha256,
                    serde_json::to_vec(&response).map_err(|error| error.to_string())?,
                    response.receipt.generation,
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(response)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn scheduler_heartbeat(
        &self,
        authorized_tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        manifest_id: &str,
        operation_id: &str,
        task_id: &SchedulerTaskId,
        machine_id: &str,
        fence: u64,
        now_ms: u64,
    ) -> Result<SchedulerMutation<u64>, String> {
        self.mutate_scheduler(
            authorized_tenant_id,
            enterprise_id,
            manifest_id,
            operation_id,
            "heartbeat",
            &("heartbeat", task_id, machine_id, fence, now_ms),
            |scheduler| scheduler.heartbeat(task_id, machine_id, fence, now_ms),
        )
    }

    pub fn scheduler_complete(
        &self,
        authorized_tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        manifest_id: &str,
        operation_id: &str,
        completion: &PageCompletion,
    ) -> Result<SchedulerMutation<()>, String> {
        if !matches!(completion.disposition, CompletionDisposition::Retry { .. }) {
            return Err(
                "terminal scheduler pages require atomic adapter evidence and signed-batch commit"
                    .into(),
            );
        }
        self.mutate_scheduler(
            authorized_tenant_id,
            enterprise_id,
            manifest_id,
            operation_id,
            "complete",
            &("complete", completion),
            |scheduler| scheduler.complete(completion.clone()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit_adapter_page(
        &self,
        authorized_tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        manifest_id: &str,
        operation_id: &str,
        adapter_receipt: &AdapterPageReceipt,
        ingest_request: &IngestRequest,
        completion: &PageCompletion,
        observed_at_ms: u64,
    ) -> Result<AtomicPageCommit, String> {
        validate_atomic_page_inputs(
            authorized_tenant_id,
            enterprise_id,
            adapter_receipt,
            ingest_request,
            completion,
            observed_at_ms,
        )?;
        let mut connection = database::open(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let scope = SchedulerScope {
            tenant_id: authorized_tenant_id.as_str(),
            enterprise_id: enterprise_id.as_str(),
            manifest_id,
        };
        let before = load_scheduler(&transaction, scope)?
            .ok_or_else(|| "scheduler manifest is not initialized".to_string())?;
        let task = before
            .tasks
            .get(&completion.task_id)
            .ok_or_else(|| "scheduler task is unknown".to_string())?;
        validate_task_binding(task, adapter_receipt, completion)?;

        let ingest_receipt = self.ingest_transaction(
            &transaction,
            authorized_tenant_id,
            ingest_request,
            observed_at_ms,
        )?;
        let request = (
            "scout-atomic-page-commit-v1",
            completion,
            adapter_receipt.receipt_id.as_str(),
            &ingest_receipt.receipt_id,
            &ingest_receipt.envelope_sha256,
        );
        let scheduler = self.mutate_scheduler_transaction(
            &transaction,
            authorized_tenant_id,
            enterprise_id,
            manifest_id,
            operation_id,
            "complete",
            &request,
            |scheduler| scheduler.complete(completion.clone()),
        )?;
        persist_page_commit(
            &transaction,
            scope,
            operation_id,
            adapter_receipt,
            ingest_request,
            &ingest_receipt,
            completion,
        )?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(AtomicPageCommit {
            ingest_receipt,
            scheduler,
        })
    }

    pub fn scheduler_reap(
        &self,
        authorized_tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        manifest_id: &str,
        operation_id: &str,
        now_ms: u64,
    ) -> Result<SchedulerMutation<usize>, String> {
        self.mutate_scheduler(
            authorized_tenant_id,
            enterprise_id,
            manifest_id,
            operation_id,
            "reap",
            &("reap", now_ms),
            |scheduler| scheduler.reap_expired(now_ms),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn mutate_scheduler<T, R, F>(
        &self,
        tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        manifest_id: &str,
        operation_id: &str,
        operation_kind: &str,
        request: &R,
        mutation: F,
    ) -> Result<SchedulerMutation<T>, String>
    where
        T: Clone + Serialize + DeserializeOwned,
        R: Serialize,
        F: FnOnce(&mut Scheduler) -> Result<T, String>,
    {
        let mut connection = database::open(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let response = self.mutate_scheduler_transaction(
            &transaction,
            tenant_id,
            enterprise_id,
            manifest_id,
            operation_id,
            operation_kind,
            request,
            mutation,
        )?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(response)
    }

    #[allow(clippy::too_many_arguments)]
    fn mutate_scheduler_transaction<T, R, F>(
        &self,
        transaction: &Transaction<'_>,
        tenant_id: &ScoutTenantId,
        enterprise_id: &EnterpriseId,
        manifest_id: &str,
        operation_id: &str,
        operation_kind: &str,
        request: &R,
        mutation: F,
    ) -> Result<SchedulerMutation<T>, String>
    where
        T: Clone + Serialize + DeserializeOwned,
        R: Serialize,
        F: FnOnce(&mut Scheduler) -> Result<T, String>,
    {
        validate_operation_id(operation_id)?;
        let request_sha256 = canonical_sha256(request)?;
        require_enterprise_pin(transaction, tenant_id, enterprise_id)?;
        if let Some(stored) = read_operation::<T>(
            transaction,
            tenant_id,
            enterprise_id,
            manifest_id,
            operation_id,
            &request_sha256,
        )? {
            return Ok(stored);
        }
        let scope = SchedulerScope {
            tenant_id: tenant_id.as_str(),
            enterprise_id: enterprise_id.as_str(),
            manifest_id,
        };
        let before = load_scheduler(transaction, scope)?
            .ok_or_else(|| "scheduler manifest is not initialized".to_string())?;
        let mut scheduler = before.to_scheduler()?;
        let result = mutation(&mut scheduler)?;
        let after = SchedulerImage::from_scheduler(&scheduler)?;
        persist_mutation(transaction, scope, &before, &after, operation_kind)?;
        let response = SchedulerMutation {
            operation_id: operation_id.to_owned(),
            manifest_id: manifest_id.to_owned(),
            result,
            receipt: after.receipt()?,
        };
        transaction
            .execute(
                "INSERT INTO scheduler_operation_rows (
                     tenant_id, enterprise_id, manifest_id, operation_id,
                     request_sha256, response_json, generation
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    tenant_id.as_str(),
                    enterprise_id.as_str(),
                    manifest_id,
                    operation_id,
                    request_sha256,
                    serde_json::to_vec(&response).map_err(|error| error.to_string())?,
                    response.receipt.generation,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(response)
    }
}

fn require_enterprise_pin(
    transaction: &Transaction<'_>,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
) -> Result<(), String> {
    let exists = transaction
        .query_row(
            "SELECT 1 FROM enterprise_pins
             WHERE tenant_id = ?1 AND enterprise_id = ?2",
            params![tenant_id.as_str(), enterprise_id.as_str()],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err("scheduler enterprise trust anchor is not pinned".into())
    }
}

fn read_operation<T: DeserializeOwned>(
    transaction: &Transaction<'_>,
    tenant_id: &ScoutTenantId,
    enterprise_id: &EnterpriseId,
    manifest_id: &str,
    operation_id: &str,
    request_sha256: &str,
) -> Result<Option<SchedulerMutation<T>>, String> {
    let stored = transaction
        .query_row(
            "SELECT request_sha256, response_json
             FROM scheduler_operation_rows
             WHERE tenant_id = ?1 AND enterprise_id = ?2
               AND manifest_id = ?3 AND operation_id = ?4",
            params![
                tenant_id.as_str(),
                enterprise_id.as_str(),
                manifest_id,
                operation_id
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    stored
        .map(|(stored_request, response)| {
            if stored_request != request_sha256 {
                return Err("scheduler operation id was reused for another request".into());
            }
            serde_json::from_slice(&response).map_err(|error| error.to_string())
        })
        .transpose()
}

fn validate_operation_id(operation_id: &str) -> Result<(), String> {
    let digest = operation_id
        .strip_prefix("scheduler-op:")
        .ok_or_else(|| "scheduler operation id must start with scheduler-op:".to_string())?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("scheduler operation id must contain a lowercase SHA-256 digest".into());
    }
    Ok(())
}
