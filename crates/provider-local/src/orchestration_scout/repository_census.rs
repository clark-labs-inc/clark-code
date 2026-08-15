use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use scout_adapter_protocol::{
    AdapterId, AdapterPageOutcome, AdapterPageReceipt, AdapterPageRequest, AuthContextDescriptor,
    AuthContextHandle, AuthSourceKind, CoverageBinding, NormalizedLink, NormalizedRecord,
    RedactionSummary, RequestId, SafeFieldValue, ADAPTER_PROTOCOL_VERSION,
};
use scout_cartography_adapter::{AdapterPageTaskScope, ADAPTER_PAGE_TASK_KIND};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::exec::Executor;
use crate::repository::inspect_repository;
use crate::tools::{ToolCtx, ToolExecutor, ToolOutcome};

use super::enterprise_backend::CartographyBackendState;
use super::ScoutToolState;

const LOCAL_REPOSITORY_ADAPTER_ID: &str = "clark/local-repository@1";

/// One host-approved checkout observation. Absolute local paths are deliberately
/// replaced with a stable opaque id and basename before the result reaches the
/// model or enterprise graph.
#[derive(Clone, Debug, Serialize)]
struct CheckoutObservation {
    checkout_id: String,
    display_name: String,
    repository_fingerprint: String,
    canonical_remote: Option<String>,
    head_oid: Option<String>,
    current_branch: Option<String>,
    default_branch: Option<String>,
    commit_count: u64,
    shallow: bool,
    dirty: bool,
}

#[derive(Debug, Serialize)]
struct RepositoryCensus {
    schema_version: &'static str,
    coverage_scope: &'static str,
    inspected_root_count: usize,
    checkout_count: usize,
    unresolved_root_count: usize,
    repositories: Vec<CheckoutObservation>,
    gaps: Vec<&'static str>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RepositoryAction {
    Census,
    Inspect,
    Collect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryCensusArgs {
    action: RepositoryAction,
    #[serde(default)]
    checkout_id: Option<String>,
}

pub(super) struct ScoutRepositoryCensusTool {
    pub state: Arc<ScoutToolState>,
    pub cartography: Arc<CartographyBackendState>,
}

#[async_trait]
impl ToolExecutor for ScoutRepositoryCensusTool {
    fn name(&self) -> &str {
        "scout_repository_census"
    }

    fn description(&self) -> &str {
        "Census only host-approved read-only roots for local Git checkouts, inspect one census-issued opaque checkout id, or collect a backend-issued local-repository task into a retained adapter receipt. Census and inspect are non-authoritative hints. Collect reruns bounded inspection under the backend task and produces the only form that may be submitted to the enterprise graph. No action returns arbitrary source, secret files, local paths, or remote credentials."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["census", "inspect", "collect"],
                    "description": "Use census for hints, inspect for one bounded hint, or collect only after claiming the backend local-repository task and running scout_adapter census on this target."
                },
                "checkout_id": {
                    "type": "string",
                    "description": "Opaque checkout id returned by census; required only for inspect."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args = match serde_json::from_value::<RepositoryCensusArgs>(args) {
            Ok(args) => args,
            Err(_) => return ToolOutcome::error("invalid Scout repository census request"),
        };
        match (args.action, args.checkout_id) {
            (RepositoryAction::Census, None) => {
                let roots = ctx.sandbox.read_roots().to_vec();
                let (census, bindings) = match census_roots(ctx.executor.as_ref(), &roots).await {
                    Ok(result) => result,
                    Err(error) => return ToolOutcome::error(error),
                };
                *self
                    .state
                    .repositories
                    .lock()
                    .expect("Scout repository bindings lock") = bindings;
                census_outcome(census)
            }
            (RepositoryAction::Inspect, Some(checkout_id)) => {
                let root = self
                    .state
                    .repositories
                    .lock()
                    .expect("Scout repository bindings lock")
                    .get(&checkout_id)
                    .cloned();
                let Some(root) = root else {
                    return ToolOutcome::error(
                        "checkout_id was not issued by this Scout session's latest census",
                    );
                };
                match inspect_checkout(ctx.executor.as_ref(), &checkout_id, &root).await {
                    Ok(summary) => match serde_json::to_value(&summary) {
                        Ok(details) => ToolOutcome::ok(
                            "Inspected bounded repository manifests and component markers without returning source or local paths.",
                        )
                        .with_model_visible_details(details),
                        Err(_) => ToolOutcome::error("Scout repository summary encoding failed"),
                    },
                    Err(error) => ToolOutcome::error(error),
                }
            }
            (RepositoryAction::Census, Some(_)) => {
                ToolOutcome::error("repository census does not accept checkout_id")
            }
            (RepositoryAction::Collect, None) => self.collect(ctx).await,
            (RepositoryAction::Collect, Some(_)) => {
                ToolOutcome::error("repository collection does not accept checkout_id")
            }
            (RepositoryAction::Inspect, None) => {
                ToolOutcome::error("repository inspection requires checkout_id from census")
            }
        }
    }
}

fn census_outcome(census: RepositoryCensus) -> ToolOutcome {
    let details = match serde_json::to_value(&census) {
        Ok(details) => details,
        Err(_) => return ToolOutcome::error("Scout repository census encoding failed"),
    };
    ToolOutcome::ok(format!(
        "Inspected {} host-approved roots and identified {} local Git checkouts; {} roots were unresolved. Unapproved filesystem locations were not scanned.",
        census.inspected_root_count,
        census.checkout_count,
        census.unresolved_root_count,
    ))
    .with_model_visible_details(details)
}

impl ScoutRepositoryCensusTool {
    async fn collect(&self, ctx: &ToolCtx) -> ToolOutcome {
        let task = match self
            .cartography
            .claimed_task_for_adapter(LOCAL_REPOSITORY_ADAPTER_ID)
        {
            Ok(task) => task,
            Err(error) => return ToolOutcome::error(error),
        };
        if task.task_kind != ADAPTER_PAGE_TASK_KIND {
            return ToolOutcome::error("backend local-repository task has an invalid kind");
        }
        let scope: AdapterPageTaskScope = match serde_json::from_value(task.scope.clone()) {
            Ok(scope) => scope,
            Err(_) => {
                return ToolOutcome::error("backend local-repository task has an invalid scope")
            }
        };
        if scope.adapter_id.as_str() != LOCAL_REPOSITORY_ADAPTER_ID
            || scope.query.operation != "list_host_approved_checkouts"
            || scope.query.provider_resource_type != "local.repository_checkout"
            || scope.page_ordinal != 0
            || scope.cursor_handle.is_some()
        {
            return ToolOutcome::error(
                "backend local-repository task does not match the host collector contract",
            );
        }
        let target =
            match self.state.target.lock().expect("Scout target lock").clone() {
                Some(target) => target,
                None => return ToolOutcome::error(
                    "run scout_adapter census on this target before collecting local repositories",
                ),
            };
        let roots = ctx.sandbox.read_roots().to_vec();
        let (census, bindings) = match census_roots(ctx.executor.as_ref(), &roots).await {
            Ok(result) => result,
            Err(error) => return ToolOutcome::error(error),
        };
        let observed_at_ms = match now_ms() {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error),
        };
        let receipt = match local_repository_receipt(
            ctx.executor.as_ref(),
            &scope,
            target,
            &census,
            &bindings,
            observed_at_ms,
        )
        .await
        {
            Ok(receipt) => receipt,
            Err(error) => return ToolOutcome::error(error),
        };
        let receipt_id = receipt.receipt_id.to_string();
        if let Err(error) = self.cartography.record_receipt(receipt) {
            return ToolOutcome::error(error);
        }
        ToolOutcome::ok(format!(
            "Collected {} host-approved local checkouts into a task-bound receipt. Submit it through scout_enterprise before treating any checkout as graph evidence.",
            census.checkout_count,
        ))
        .with_model_visible_details(json!({
            "task_id": task.task_id,
            "receipt_id": receipt_id,
            "checkout_count": census.checkout_count,
            "unresolved_root_count": census.unresolved_root_count,
            "authority": "pending_backend_acceptance",
        }))
    }
}

async fn census_roots(
    exec: &dyn Executor,
    roots: &[PathBuf],
) -> Result<(RepositoryCensus, HashMap<String, PathBuf>), String> {
    let mut repositories = Vec::new();
    let mut bindings = HashMap::new();
    let mut unresolved_root_count = 0;
    for root in roots {
        let canonical_root = match exec.canonicalize(root).await {
            Ok(root) => root,
            Err(_) => {
                unresolved_root_count += 1;
                continue;
            }
        };
        match inspect_repository(exec, &canonical_root).await {
            Ok(Some(repository)) => {
                let id = checkout_id(&canonical_root);
                bindings.insert(id.clone(), canonical_root.clone());
                repositories.push(CheckoutObservation {
                    checkout_id: id,
                    display_name: canonical_root
                        .file_name()
                        .and_then(|name| name.to_str())
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or("repository")
                        .to_string(),
                    repository_fingerprint: repository.fingerprint,
                    canonical_remote: repository.canonical_remote,
                    head_oid: repository.head_oid,
                    current_branch: repository.current_branch,
                    default_branch: repository.default_branch,
                    commit_count: repository.commit_count,
                    shallow: repository.shallow,
                    dirty: repository.dirty,
                })
            }
            Ok(None) | Err(_) => unresolved_root_count += 1,
        }
    }
    repositories.sort_by(|left, right| left.checkout_id.cmp(&right.checkout_id));
    Ok((
        RepositoryCensus {
            schema_version: "scout-repository-census-v1",
            coverage_scope: "host_approved_read_roots",
            inspected_root_count: roots.len(),
            checkout_count: repositories.len(),
            unresolved_root_count,
            repositories,
            gaps: vec!["unapproved_filesystem_locations_not_scanned"],
        },
        bindings,
    ))
}

async fn local_repository_receipt(
    exec: &dyn Executor,
    scope: &AdapterPageTaskScope,
    target: scout_adapter_protocol::TargetIdentity,
    census: &RepositoryCensus,
    bindings: &HashMap<String, PathBuf>,
    observed_at_ms: u64,
) -> Result<AdapterPageReceipt, String> {
    let adapter_id =
        AdapterId::new(LOCAL_REPOSITORY_ADAPTER_ID).map_err(|error| error.to_string())?;
    if scope.adapter_id != adapter_id {
        return Err("backend local-repository adapter id is invalid".into());
    }
    let target_fingerprint = target
        .fingerprint_sha256()
        .map_err(|error| error.to_string())?;
    let mut grant = Sha256::new();
    grant.update(b"clark.local-repository-grant/v1\0");
    grant.update(target_fingerprint.as_bytes());
    for repository in &census.repositories {
        grant.update(repository.checkout_id.as_bytes());
        grant.update(repository.repository_fingerprint.as_bytes());
    }
    let auth = AuthContextDescriptor::new(
        AuthContextHandle::random(),
        target.target_id.clone(),
        adapter_id.clone(),
        "local".into(),
        scope.query.authority_scope.clone(),
        target.target_id.to_string(),
        AuthSourceKind::BrokeredSession,
        format!("{:x}", grant.finalize()),
        observed_at_ms,
        None,
    )
    .map_err(|error| error.to_string())?;
    let request = AdapterPageRequest {
        protocol_version: ADAPTER_PROTOCOL_VERSION,
        request_id: RequestId::random(),
        target_id: target.target_id.clone(),
        target_identity_sha256: target_fingerprint,
        adapter_id: adapter_id.clone(),
        auth_context_handle: auth.handle.clone(),
        auth_context_id: auth.context_id.clone(),
        coverage: CoverageBinding {
            enterprise_id: scope.enterprise_id.clone(),
            charter_id: scope.charter_id.clone(),
            discovery_epoch: scope.discovery_epoch,
            sequence: scope.coverage_sequence,
            adapter_id: adapter_id.clone(),
            auth_context_id: auth.context_id.clone(),
            tenant: scope.query.authority_scope.clone(),
            region_or_project: scope.region_or_project.clone(),
            resource_kind: scope.resource_kind.clone(),
        },
        query: scope.query.clone(),
        page_ordinal: scope.page_ordinal,
        cursor_handle: scope.cursor_handle.clone(),
        limits: scope.limits,
        requested_at_ms: observed_at_ms,
    };

    let mut records = Vec::new();
    let record_limit = request.limits.max_records as usize;
    for observation in census.repositories.iter().take(record_limit) {
        let inspection = match bindings.get(&observation.checkout_id) {
            Some(root) => inspect_checkout(exec, &observation.checkout_id, root)
                .await
                .ok(),
            None => None,
        };
        records.push(local_repository_record(
            &request,
            &target,
            observation,
            inspection.as_ref(),
        )?);
    }
    let truncated = census.repositories.len() > record_limit;
    let outcome = if truncated {
        AdapterPageOutcome::Truncated {
            reason: scout_adapter_protocol::TruncationReason::RecordLimit,
            continuation_available: false,
        }
    } else {
        AdapterPageOutcome::Succeeded { final_page: true }
    };
    let adapter_build_sha256 = format!(
        "{:x}",
        Sha256::digest(concat!(
            "clark/local-repository@1/",
            env!("CARGO_PKG_VERSION")
        ))
    );
    AdapterPageReceipt::new(
        request,
        target,
        auth,
        adapter_build_sha256,
        observed_at_ms,
        outcome,
        records,
        None,
        RedactionSummary {
            source_records_seen: census.repositories.len() as u64,
            records_emitted: census.repositories.len().min(record_limit) as u64,
            fields_omitted: census.unresolved_root_count as u64,
            values_rejected: 0,
        },
    )
    .map_err(|error| error.to_string())
}

fn local_repository_record(
    request: &AdapterPageRequest,
    target: &scout_adapter_protocol::TargetIdentity,
    observation: &CheckoutObservation,
    inspection: Option<&RepositoryInspection>,
) -> Result<NormalizedRecord, String> {
    let mut fields = BTreeMap::new();
    let mut insert = |name: &str, value: Option<SafeFieldValue>| {
        if request.query.projection.contains(name) {
            if let Some(value) = value {
                fields.insert(name.to_owned(), value);
            }
        }
    };
    insert(
        "display_name",
        Some(SafeFieldValue::Text(observation.display_name.clone())),
    );
    insert(
        "repository_fingerprint",
        Some(SafeFieldValue::Text(
            observation.repository_fingerprint.clone(),
        )),
    );
    insert(
        "canonical_remote",
        observation
            .canonical_remote
            .clone()
            .map(SafeFieldValue::Text),
    );
    insert(
        "head_oid",
        observation.head_oid.clone().map(SafeFieldValue::Text),
    );
    insert(
        "current_branch",
        observation.current_branch.clone().map(SafeFieldValue::Text),
    );
    insert(
        "default_branch",
        observation.default_branch.clone().map(SafeFieldValue::Text),
    );
    insert(
        "commit_count",
        Some(SafeFieldValue::Unsigned(observation.commit_count)),
    );
    insert(
        "shallow",
        Some(SafeFieldValue::Boolean(observation.shallow)),
    );
    insert("dirty", Some(SafeFieldValue::Boolean(observation.dirty)));
    if let Some(inspection) = inspection {
        insert("manifests", text_set(inspection.manifests.iter().cloned()));
        insert(
            "package_names",
            text_set(inspection.package_names.iter().cloned()),
        );
        insert(
            "descriptions",
            text_set(inspection.descriptions.iter().cloned()),
        );
        insert(
            "dependency_names",
            text_set(inspection.dependency_names.iter().cloned()),
        );
        insert(
            "command_names",
            text_set(inspection.command_names.iter().cloned()),
        );
        insert(
            "component_names",
            text_set(inspection.component_names.iter().cloned()),
        );
        insert(
            "workflow_names",
            text_set(inspection.workflow_names.iter().cloned()),
        );
        insert(
            "infrastructure_markers",
            text_set(inspection.infrastructure_markers.iter().cloned()),
        );
    }
    let links = observation
        .canonical_remote
        .as_ref()
        .map(|remote| {
            BTreeSet::from([NormalizedLink {
                relationship_type: "checkout_of".into(),
                target_provider_namespace: "git".into(),
                target_provider_type: "git.repository".into(),
                target_authority_scope: "global".into(),
                target_native_id: remote.clone(),
                qualifier: None,
            }])
        })
        .unwrap_or_default();
    NormalizedRecord::new(
        request.adapter_id.clone(),
        "local".into(),
        request.query.provider_resource_type.clone(),
        target.target_id.to_string(),
        observation.checkout_id.clone(),
        Some("repository_checkout".into()),
        BTreeSet::new(),
        fields,
        links,
    )
    .map_err(|error| error.to_string())
}

fn text_set(values: impl IntoIterator<Item = String>) -> Option<SafeFieldValue> {
    let values = values.into_iter().take(256).collect::<BTreeSet<_>>();
    (!values.is_empty()).then_some(SafeFieldValue::TextSet(values))
}

fn now_ms() -> Result<u64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?;
    u64::try_from(elapsed.as_millis()).map_err(|_| "system clock exceeds u64".to_string())
}

#[derive(Debug, Serialize)]
struct RepositoryInspection {
    schema_version: &'static str,
    checkout_id: String,
    manifests: Vec<String>,
    package_names: BTreeSet<String>,
    descriptions: BTreeSet<String>,
    dependency_names: BTreeSet<String>,
    command_names: BTreeSet<String>,
    component_names: BTreeSet<String>,
    workflow_names: BTreeSet<String>,
    infrastructure_markers: BTreeSet<String>,
    gaps: Vec<&'static str>,
}

async fn inspect_checkout(
    exec: &dyn Executor,
    checkout_id: &str,
    root: &Path,
) -> Result<RepositoryInspection, String> {
    if exec.canonicalize(root).await? != root {
        return Err("censused checkout identity changed; run the repository census again".into());
    }
    let mut summary = RepositoryInspection {
        schema_version: "scout-repository-inspection-v1",
        checkout_id: checkout_id.to_owned(),
        manifests: Vec::new(),
        package_names: BTreeSet::new(),
        descriptions: BTreeSet::new(),
        dependency_names: BTreeSet::new(),
        command_names: BTreeSet::new(),
        component_names: BTreeSet::new(),
        workflow_names: BTreeSet::new(),
        infrastructure_markers: BTreeSet::new(),
        gaps: vec![
            "arbitrary_source_not_read",
            "runtime_relationships_require_independent_evidence",
        ],
    };
    for manifest in [
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "go.mod",
        "docker-compose.yml",
        "docker-compose.yaml",
        "Dockerfile",
    ] {
        let path = root.join(manifest);
        let Ok(metadata) = exec.metadata(&path).await else {
            continue;
        };
        if metadata.is_dir || metadata.is_symlink || metadata.len > 256 * 1024 {
            continue;
        }
        summary.manifests.push(manifest.to_owned());
        let bytes = exec.read(&path).await?;
        if bytes.len() > 256 * 1024 {
            continue;
        }
        match manifest {
            "package.json" => inspect_package_json(&bytes, &mut summary),
            "Cargo.toml" | "pyproject.toml" => inspect_toml(&bytes, &mut summary),
            "go.mod" => inspect_go_mod(&bytes, &mut summary),
            _ => {
                summary.infrastructure_markers.insert(manifest.to_owned());
            }
        }
    }
    for component_root in ["apps", "services", "packages", "crates"] {
        let component_path = root.join(component_root);
        if exec
            .canonicalize(&component_path)
            .await
            .is_ok_and(|path| path.starts_with(root))
        {
            let entries = exec.read_dir(&component_path).await.unwrap_or_default();
            for entry in entries
                .into_iter()
                .filter(|entry| entry.is_dir && !entry.is_symlink && !entry.name.starts_with('.'))
                .take(128)
            {
                summary
                    .component_names
                    .insert(format!("{component_root}/{}", entry.name));
            }
        }
    }
    let workflows_path = root.join(".github/workflows");
    if exec
        .canonicalize(&workflows_path)
        .await
        .is_ok_and(|path| path.starts_with(root))
    {
        let entries = exec.read_dir(&workflows_path).await.unwrap_or_default();
        for entry in entries
            .into_iter()
            .filter(|entry| !entry.is_dir && !entry.is_symlink)
            .take(128)
        {
            if entry.name.ends_with(".yml") || entry.name.ends_with(".yaml") {
                summary.workflow_names.insert(entry.name);
            }
        }
    }
    for marker in ["terraform", "infra", "k8s", "helm", "deploy"] {
        if exec.metadata(&root.join(marker)).await.is_ok() {
            summary.infrastructure_markers.insert(marker.to_owned());
        }
    }
    Ok(summary)
}

fn inspect_package_json(bytes: &[u8], summary: &mut RepositoryInspection) {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return;
    };
    if let Some(name) = value.get("name").and_then(Value::as_str) {
        insert_bounded(&mut summary.package_names, name, 256);
    }
    if let Some(description) = value.get("description").and_then(Value::as_str) {
        insert_bounded(&mut summary.descriptions, description, 1_000);
    }
    for section in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(entries) = value.get(section).and_then(Value::as_object) {
            for name in entries.keys().take(512) {
                insert_bounded(&mut summary.dependency_names, name, 256);
            }
        }
    }
    if let Some(scripts) = value.get("scripts").and_then(Value::as_object) {
        for name in scripts.keys().take(128) {
            insert_bounded(&mut summary.command_names, name, 128);
        }
    }
}

fn inspect_toml(bytes: &[u8], summary: &mut RepositoryInspection) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let Ok(value) = toml::from_str::<toml::Value>(text) else {
        return;
    };
    for section in ["package", "project"] {
        if let Some(table) = value.get(section).and_then(toml::Value::as_table) {
            if let Some(name) = table.get("name").and_then(toml::Value::as_str) {
                insert_bounded(&mut summary.package_names, name, 256);
            }
            if let Some(description) = table.get("description").and_then(toml::Value::as_str) {
                insert_bounded(&mut summary.descriptions, description, 1_000);
            }
        }
    }
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = value.get(section).and_then(toml::Value::as_table) {
            for name in table.keys().take(512) {
                insert_bounded(&mut summary.dependency_names, name, 256);
            }
        }
    }
}

fn inspect_go_mod(bytes: &[u8], summary: &mut RepositoryInspection) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(module) = line.strip_prefix("module ") {
            insert_bounded(&mut summary.package_names, module.trim(), 256);
        }
    }
}

fn insert_bounded(values: &mut BTreeSet<String>, value: &str, maximum: usize) {
    let value = value.trim();
    if !value.is_empty()
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
        && !looks_secret_bearing(value)
        && values.len() < 512
    {
        values.insert(value.to_owned());
    }
}

fn looks_secret_bearing(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.starts_with("ghp_")
        || value.starts_with("github_pat_")
        || value.starts_with("AKIA")
        || value.starts_with("sk-")
        || lower.contains("password=")
        || lower.contains("token=")
        || lower.contains("secret=")
}

fn checkout_id(root: &Path) -> String {
    format!(
        "checkout:{:x}",
        Sha256::digest(root.to_string_lossy().as_bytes())
    )
}

#[cfg(test)]
#[path = "repository_census/tests.rs"]
mod tests;
