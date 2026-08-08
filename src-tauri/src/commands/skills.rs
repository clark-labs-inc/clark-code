use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::{project_executor, RemoteArg};
use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogChange {
    pub changed: bool,
    pub revision: String,
    pub snapshot: Option<provider_local::SkillCatalogSnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackOperationResult {
    pub receipt: provider_local::SkillPackReceipt,
    pub catalog: provider_local::SkillCatalogSnapshot,
}

async fn catalog_context(
    cwd: &str,
    remote: Option<RemoteArg>,
    state: &AppState,
) -> Result<(Box<dyn provider_local::Executor>, PathBuf, String), String> {
    let remote_identity = remote.as_ref().map(|remote| remote.id.clone());
    let executor = project_executor(remote, state).await?;
    let requested = PathBuf::from(cwd.trim());
    let root = executor.canonicalize(&requested).await.unwrap_or(requested);
    let environment_id = provider_local::skill_environment_id(&root, remote_identity.as_deref());
    Ok((executor, root, environment_id))
}

#[tauri::command]
pub async fn skills_list(
    cwd: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<provider_local::SkillCatalogSnapshot, String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let catalogs = state.runtime_registry.current_skill_catalogs().await;
    let (executor, cwd, environment_id) = catalog_context(&cwd, remote, state.inner()).await?;
    if let Some(snapshot) = catalogs.current_snapshot(&cwd, &environment_id).await {
        return Ok(snapshot);
    }
    Ok(catalogs
        .refresh_snapshot(executor.as_ref(), &cwd, &environment_id)
        .await)
}

#[tauri::command]
pub async fn skills_reload(
    app: AppHandle,
    cwd: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<provider_local::SkillCatalogSnapshot, String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let catalogs = state.runtime_registry.current_skill_catalogs().await;
    let (executor, cwd, environment_id) = catalog_context(&cwd, remote, state.inner()).await?;
    let prior = catalogs
        .current_snapshot(&cwd, &environment_id)
        .await
        .map(|snapshot| snapshot.revision);
    let snapshot = catalogs
        .refresh_snapshot(executor.as_ref(), &cwd, &environment_id)
        .await;
    if prior.as_deref() != Some(snapshot.revision.as_str()) {
        let _ = app.emit("skill-catalog-changed", &snapshot);
    }
    Ok(snapshot)
}

#[tauri::command]
pub async fn skills_changes(
    app: AppHandle,
    cwd: String,
    since_revision: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<SkillCatalogChange, String> {
    let snapshot = skills_reload(app, cwd, remote, state).await?;
    let changed = snapshot.revision != since_revision;
    Ok(SkillCatalogChange {
        changed,
        revision: snapshot.revision.clone(),
        snapshot: changed.then_some(snapshot),
    })
}

#[tauri::command]
pub async fn instructions_list(
    cwd: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<Option<provider_local::ProjectInstructions>, String> {
    let (executor, cwd, _) = catalog_context(&cwd, remote, state.inner()).await?;
    provider_local::discover_instructions(executor.as_ref(), &cwd).await
}

#[tauri::command]
pub async fn skill_packs_list(
    cwd: String,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<Vec<provider_local::InstalledSkillPack>, String> {
    let (executor, cwd, _) = catalog_context(&cwd, remote, state.inner()).await?;
    provider_local::list_skill_packs(executor.as_ref(), &cwd).await
}

#[tauri::command]
pub async fn skill_pack_install(
    app: AppHandle,
    cwd: String,
    request: provider_local::InstallSkillPackRequest,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<SkillPackOperationResult, String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let catalogs = state.runtime_registry.current_skill_catalogs().await;
    let (executor, cwd, environment_id) = catalog_context(&cwd, remote, state.inner()).await?;
    let receipt = provider_local::install_skill_pack(executor.as_ref(), &cwd, request).await?;
    let catalog = catalogs
        .refresh_snapshot(executor.as_ref(), &cwd, &environment_id)
        .await;
    let _ = app.emit("skill-catalog-changed", &catalog);
    Ok(SkillPackOperationResult { receipt, catalog })
}

#[tauri::command]
pub async fn skill_pack_uninstall(
    app: AppHandle,
    cwd: String,
    pack_id: String,
    scope: provider_local::SkillPackScope,
    remote: Option<RemoteArg>,
    state: State<'_, AppState>,
) -> Result<SkillPackOperationResult, String> {
    let _account_lifecycle = state.account_lifecycle.read().await;
    let catalogs = state.runtime_registry.current_skill_catalogs().await;
    let (executor, cwd, environment_id) = catalog_context(&cwd, remote, state.inner()).await?;
    let receipt =
        provider_local::uninstall_skill_pack(executor.as_ref(), &cwd, &pack_id, scope).await?;
    let catalog = catalogs
        .refresh_snapshot(executor.as_ref(), &cwd, &environment_id)
        .await;
    let _ = app.emit("skill-catalog-changed", &catalog);
    Ok(SkillPackOperationResult { receipt, catalog })
}
