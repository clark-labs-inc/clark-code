use super::cloud_authority::current_cloud_access;
use super::*;
use conversation_cloud::{
    ConversationClient, ConversationWrite, CredentialSurface, SpecialistContext,
};
use reqwest::StatusCode;
use tauri::Emitter;

pub(super) fn desktop_conversation_client(
    rest_base: &str,
    token: &str,
) -> Result<ConversationClient, String> {
    ConversationClient::new(
        rest_base,
        token,
        CredentialSurface::DesktopSession,
        concat!("clark-desktop/", env!("CARGO_PKG_VERSION")),
    )
    .map_err(|error| error.to_string())
}

fn transient_cloud_read_status(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        )
}

/// List the signed-in user's desktop conversations (metadata only). The cloud
/// response is authoritative; the account-scoped SQLite cache only fills rows
/// that have not reached the cloud yet or keeps history available offline.
#[tauri::command]
pub async fn desktop_conv_list(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let cloud = desktop_conversation_client(&access.rest_base, &token)?
        .list()
        .await;
    let (rows, cloud_available) = match cloud {
        Ok(summaries) => (
            summaries
                .into_iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("desktop list serialization failed: {error}"))?,
            true,
        ),
        Err(error) => {
            tracing::warn!(%error, "desktop cloud list unavailable; using local acknowledged cache");
            (Vec::new(), false)
        }
    };
    let merged = crate::trajectory::merge_local_summaries(
        crate::trajectory::outbox_path(&app)?,
        access.owner_scope,
        rows,
        cloud_available,
    )
    .await?;
    Ok(Value::Array(merged))
}

/// Fetch one desktop conversation including its full snapshot blob.
#[tauri::command]
pub async fn desktop_conv_get(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let cloud = desktop_conversation_client(&access.rest_base, &token)?
        .get(&id)
        .await;
    let cloud_detail = match cloud {
        Ok(detail) => Some(
            serde_json::to_value(detail)
                .map_err(|error| format!("desktop get serialization failed: {error}"))?,
        ),
        Err(error) => {
            let status = error
                .status()
                .and_then(|value| StatusCode::from_u16(value).ok());
            if status == Some(StatusCode::UNAUTHORIZED) {
                let _ = app.emit("cloud-auth-expired", ());
                return Err(error.to_string());
            }
            if status.is_some_and(|status| !transient_cloud_read_status(status)) {
                // A reachable cloud is authoritative. In particular, never turn
                // another device's 404 deletion into a local recovery PUT.
                return Err(error.to_string());
            }
            tracing::warn!(%error, conversation_id = %id, "desktop cloud get temporarily unavailable; using local acknowledged cache");
            None
        }
    };
    let cloud_snapshot = cloud_detail.as_ref().and_then(|detail| {
        let raw = crate::trajectory::normalize_snapshot_value(detail.get("snapshot")?.clone());
        let snapshot = serde_json::from_value(raw).ok()?;
        let rev = detail
            .get("rev")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        Some((snapshot, rev))
    });
    let recovered = crate::trajectory::recover_snapshot(
        crate::trajectory::outbox_path(&app)?,
        access.owner_scope,
        id.clone(),
        cloud_snapshot,
    )
    .await?;
    match (cloud_detail, recovered) {
        (Some(mut detail), Some(recovered)) => {
            let snapshot_recovery_required = recovered.needs_snapshot_publication;
            let mut snapshot =
                serde_json::to_value(recovered.snapshot).map_err(|e| e.to_string())?;
            if recovered.pending {
                snapshot["sync_pending"] = true.into();
            }
            detail["snapshot"] = snapshot;
            detail["syncPending"] = recovered.pending.into();
            // A trajectory batch can be acknowledged before this recovered full
            // snapshot is checkpointed. Tell the WebView to publish the exact
            // recovered projection rather than treating `syncPending` as proof
            // that mobile already has it.
            detail["snapshotRecoveryRequired"] = snapshot_recovery_required.into();
            Ok(detail)
        }
        (Some(detail), None) => Ok(detail),
        (None, Some(recovered)) => {
            // Only a local cache can yield this branch. Its persisted metadata is
            // required for the WebView's recovery PUT; without it, returning a
            // bare snapshot would look usable but could never converge to cloud.
            let mut detail = recovered.metadata.ok_or_else(|| {
                format!("desktop conversation {id} recovery is missing cached metadata")
            })?;
            let mut snapshot =
                serde_json::to_value(recovered.snapshot).map_err(|e| e.to_string())?;
            if recovered.pending {
                snapshot["sync_pending"] = true.into();
            }
            detail["snapshot"] = snapshot;
            detail["syncPending"] = recovered.pending.into();
            detail["snapshotRecoveryRequired"] = recovered.needs_snapshot_publication.into();
            Ok(detail)
        }
        (None, None) => Err(format!(
            "desktop conversation {id} is unavailable locally and in Clark cloud"
        )),
    }
}

/// Fetch one small composer draft without loading or rewriting its transcript.
#[tauri::command]
pub async fn desktop_draft_get(
    app: AppHandle,
    draft_key: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let url = format!(
        "{}/api/desktop/drafts/{}",
        access.rest_base,
        urlencoding::encode(&draft_key)
    );
    let response = clark_http_client()?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|error| format!("desktop draft get request failed: {error}"))?;
    if response.status() == StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    if response.status() == StatusCode::UNAUTHORIZED {
        let _ = app.emit("cloud-auth-expired", ());
    }
    read_json_or_err(response, "desktop draft get").await
}

/// Compare-and-swap one composer draft independently from transcript history.
/// Conflicts are returned as data so the WebView can preserve both versions.
#[tauri::command]
pub async fn desktop_draft_put(
    app: AppHandle,
    draft_key: String,
    text: String,
    base_rev: i64,
    mutation_id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let url = format!(
        "{}/api/desktop/drafts/{}",
        access.rest_base,
        urlencoding::encode(&draft_key)
    );
    let response = clark_http_client()?
        .put(url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "text": text,
            "baseRev": base_rev,
            "mutationId": mutation_id,
        }))
        .send()
        .await
        .map_err(|error| format!("desktop draft put request failed: {error}"))?;
    if response.status() == StatusCode::UNAUTHORIZED {
        let _ = app.emit("cloud-auth-expired", ());
    }
    if response.status() == StatusCode::CONFLICT {
        let current = read_json_or_err(response, "desktop draft conflict").await?;
        return Ok(serde_json::json!({ "conflict": true, "current": current["current"] }));
    }
    read_json_or_err(response, "desktop draft put").await
}

/// Insert or replace a desktop conversation snapshot.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn desktop_conv_put(
    app: AppHandle,
    id: String,
    title: String,
    provider: String,
    project: Option<String>,
    repository_fingerprint: Option<String>,
    remote_host: Option<String>,
    mode: Option<String>,
    title_locked: bool,
    specialist_context: Option<Value>,
    rev: i64,
    mut snapshot: Value,
    status: Option<String>,
    base_rev: Option<i64>,
    mutation_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_cloud_access(state.inner()).await?;
    let token = access.token.clone();
    let owner_scope = access.owner_scope;
    let local_live = status.as_deref() == Some("running");
    let checkpoint_seq = snapshot
        .get("history_checkpoint")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    crate::trajectory::wait_for_acknowledged_prefix(
        crate::trajectory::outbox_path(&app)?,
        owner_scope.clone(),
        id.clone(),
        checkpoint_seq,
        std::time::Duration::from_secs(10),
    )
    .await?;
    if let Some(object) = snapshot.as_object_mut() {
        object.remove("history_checkpoint");
        object.remove("sync_pending");
    }
    snapshot = crate::trajectory::normalize_snapshot_value(snapshot);
    let typed_snapshot: Snapshot = serde_json::from_value(snapshot)
        .map_err(|error| format!("checkpoint desktop snapshot: {error}"))?;
    let typed_specialist_context = specialist_context
        .clone()
        .map(serde_json::from_value::<SpecialistContext>)
        .transpose()
        .map_err(|error| format!("desktop specialist context is invalid: {error}"))?;
    let checkpoint_metadata = serde_json::json!({
        "id": id,
        "title": title,
        "provider": provider,
        "project": project,
        "repositoryFingerprint": repository_fingerprint,
        "remoteHost": remote_host,
        "mode": mode,
        "titleLocked": title_locked,
        "specialistContext": specialist_context.clone(),
        "rev": rev,
        "archived": false,
    });
    let parsed_mutation_id = mutation_id
        .as_deref()
        .map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|error| format!("desktop mutation id is invalid: {error}"))?;
    let write = ConversationWrite {
        id: id.clone(),
        title: title.clone(),
        provider: provider.clone(),
        project: project.clone(),
        repository_fingerprint: repository_fingerprint.clone(),
        remote_host: remote_host.clone(),
        mode: mode.clone(),
        title_locked,
        specialist_context: typed_specialist_context,
        rev,
        snapshot: typed_snapshot.clone(),
        status: status.clone(),
        base_rev,
        mutation_id: parsed_mutation_id,
    };
    let summary = match desktop_conversation_client(&access.rest_base, &token)?
        .put(&write)
        .await
    {
        Err(error)
            if matches!(
                error.status(),
                Some(value) if value == StatusCode::NOT_FOUND.as_u16()
                    || value == StatusCode::GONE.as_u16()
            ) =>
        {
            let _ = app.emit("cloud-conversation-deleted", &id);
            return Err(format!(
                "cloud_deleted: this conversation was deleted on another device: {error}"
            ));
        }
        Err(error) if error.status() == Some(StatusCode::CONFLICT.as_u16()) => {
            crate::trajectory::quarantine_snapshot_branch(
                crate::trajectory::outbox_path(&app)?,
                owner_scope,
                id,
            )
            .await?;
            return Err(error.to_string());
        }
        Err(error) => {
            tracing::warn!(
                event = "conversation_cloud_checkpoint_failed",
                conversation_id = %id,
                provider,
                status = error.status(),
                "Clark cloud rejected the conversation checkpoint"
            );
            return Err(error.to_string());
        }
        Ok(summary) => serde_json::to_value(summary)
            .map_err(|error| format!("desktop put serialization failed: {error}"))?,
    };
    let stored_rev = summary
        .get("rev")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if stored_rev > rev {
        return Err(format!(
            "cloud_conflict: Clark cloud revision {stored_rev} is newer than local revision {rev}"
        ));
    }
    crate::trajectory::checkpoint_snapshot(
        crate::trajectory::outbox_path(&app)?,
        owner_scope,
        id.clone(),
        checkpoint_metadata,
        typed_snapshot,
        stored_rev,
        checkpoint_seq,
        local_live,
    )
    .await?;
    Ok(summary)
}
