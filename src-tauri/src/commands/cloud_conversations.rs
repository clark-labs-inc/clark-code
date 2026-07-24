use super::cloud_authority::require_cloud_access;
use super::*;
use reqwest::StatusCode;
use tauri::Emitter;

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
    endpoint: String,
    token: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = require_cloud_access(state.inner(), &endpoint, &token).await?;
    let url = format!("{}/api/desktop/conversations", access.rest_base);
    let cloud = clark_http_client()?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await;
    let (rows, cloud_available) = match cloud {
        Ok(response) => match read_json_or_err(response, "desktop list").await {
            Ok(value) => (value.as_array().cloned().unwrap_or_default(), true),
            Err(error) => {
                tracing::warn!(%error, "desktop cloud list unavailable; using local acknowledged cache");
                (Vec::new(), false)
            }
        },
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
    endpoint: String,
    token: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = require_cloud_access(state.inner(), &endpoint, &token).await?;
    let url = format!(
        "{}/api/desktop/conversations/{}",
        access.rest_base,
        urlencoding::encode(&id)
    );
    let cloud = clark_http_client()?
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await;
    let cloud_detail = match cloud {
        Ok(response) if response.status().is_success() => {
            Some(read_json_or_err(response, "desktop get").await?)
        }
        Ok(response) => {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            let error = format!("desktop get failed ({status}): {detail}");
            if status == StatusCode::UNAUTHORIZED {
                let _ = app.emit("cloud-auth-expired", ());
                return Err(error);
            }
            if !transient_cloud_read_status(status) {
                // A reachable cloud is authoritative. In particular, never turn
                // another device's 404 deletion into a local recovery PUT.
                return Err(error);
            }
            tracing::warn!(%error, conversation_id = %id, "desktop cloud get temporarily unavailable; using local acknowledged cache");
            None
        }
        Err(error) => {
            tracing::warn!(%error, conversation_id = %id, "desktop cloud get unavailable; using local acknowledged cache");
            None
        }
    };
    let cloud_snapshot = cloud_detail.as_ref().and_then(|detail| {
        let snapshot = serde_json::from_value(detail.get("snapshot")?.clone()).ok()?;
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

/// Insert or replace a desktop conversation snapshot.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn desktop_conv_put(
    app: AppHandle,
    endpoint: String,
    token: String,
    id: String,
    title: String,
    provider: String,
    project: Option<String>,
    repository_fingerprint: Option<String>,
    remote_host: Option<String>,
    mode: Option<String>,
    title_locked: bool,
    rev: i64,
    mut snapshot: Value,
    status: Option<String>,
    base_rev: Option<i64>,
    mutation_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = require_cloud_access(state.inner(), &endpoint, &token).await?;
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
    let checkpoint_snapshot = snapshot.clone();
    let checkpoint_metadata = serde_json::json!({
        "id": id,
        "title": title,
        "provider": provider,
        "project": project,
        "repositoryFingerprint": repository_fingerprint,
        "remoteHost": remote_host,
        "mode": mode,
        "titleLocked": title_locked,
        "rev": rev,
        "archived": false,
    });
    let url = format!(
        "{}/api/desktop/conversations/{}",
        access.rest_base,
        urlencoding::encode(&id)
    );
    let resp = clark_http_client()?
        .put(url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": title,
            "provider": provider,
            "project": project,
            "repositoryFingerprint": repository_fingerprint,
            "remoteHost": remote_host,
            "mode": mode,
            "titleLocked": title_locked,
            "rev": rev,
            "snapshot": snapshot,
            "status": status,
            "baseRev": base_rev,
            "mutationId": mutation_id,
        }))
        .send()
        .await
        .map_err(|e| format!("desktop put request failed: {e}"))?;
    if matches!(resp.status(), StatusCode::NOT_FOUND | StatusCode::GONE) {
        let detail = resp.text().await.unwrap_or_default();
        let _ = app.emit("cloud-conversation-deleted", &id);
        return Err(format!(
            "cloud_deleted: this conversation was deleted on another device: {detail}"
        ));
    }
    if resp.status() == reqwest::StatusCode::CONFLICT {
        let detail = resp.text().await.unwrap_or_default();
        crate::trajectory::quarantine_snapshot_branch(
            crate::trajectory::outbox_path(&app)?,
            owner_scope,
            id,
        )
        .await?;
        return Err(format!("desktop put failed (409 Conflict): {detail}"));
    }
    let summary = read_json_or_err(resp, "desktop put").await?;
    let stored_rev = summary
        .get("rev")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if stored_rev > rev {
        return Err(format!(
            "cloud_conflict: Clark cloud revision {stored_rev} is newer than local revision {rev}"
        ));
    }
    let typed_snapshot: Snapshot = serde_json::from_value(checkpoint_snapshot)
        .map_err(|error| format!("checkpoint desktop snapshot: {error}"))?;
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
