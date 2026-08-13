use super::cloud_authority::current_account_access;
use super::*;
use tauri::Emitter;

pub(crate) enum ProductCloudOutcome {
    Ok(Value),
    Unauthorized(String),
    NotFound(String),
    Conflict(String),
    Unavailable(String),
    Rejected(String),
}

struct ConversationListSource {
    rows: Vec<Value>,
    cloud_available: bool,
    auth_expired: bool,
}

fn conversation_list_source(cloud: ProductCloudOutcome) -> Result<ConversationListSource, String> {
    match cloud {
        ProductCloudOutcome::Ok(Value::Array(rows)) => Ok(ConversationListSource {
            rows,
            cloud_available: true,
            auth_expired: false,
        }),
        ProductCloudOutcome::Ok(_) => Err("product cloud conversation list is not an array".into()),
        ProductCloudOutcome::Unavailable(_) => Ok(ConversationListSource {
            rows: Vec::new(),
            cloud_available: false,
            auth_expired: false,
        }),
        ProductCloudOutcome::Unauthorized(_) => Ok(ConversationListSource {
            rows: Vec::new(),
            cloud_available: false,
            auth_expired: true,
        }),
        ProductCloudOutcome::NotFound(error)
        | ProductCloudOutcome::Conflict(error)
        | ProductCloudOutcome::Rejected(error) => Err(error),
    }
}

pub(crate) async fn product_cloud_request(
    operation: &str,
    payload: Value,
    app: &AppHandle,
    state: &AppState,
) -> Result<ProductCloudOutcome, String> {
    let response = super::product::dispatch_product_request(operation, payload, app, state).await?;
    let object = response
        .as_object()
        .ok_or("product cloud response must be an object")?;
    let outcome = object
        .get("outcome")
        .and_then(Value::as_str)
        .ok_or("product cloud response has no outcome")?;
    let error = object
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("product cloud request failed")
        .to_string();
    match outcome {
        "ok" => Ok(ProductCloudOutcome::Ok(
            object.get("value").cloned().unwrap_or(Value::Null),
        )),
        "unauthorized" => Ok(ProductCloudOutcome::Unauthorized(error)),
        "not_found" => Ok(ProductCloudOutcome::NotFound(error)),
        "conflict" => Ok(ProductCloudOutcome::Conflict(error)),
        "unavailable" => Ok(ProductCloudOutcome::Unavailable(error)),
        "rejected" => Ok(ProductCloudOutcome::Rejected(error)),
        _ => Err("product cloud response has an invalid outcome".into()),
    }
}

/// List the signed-in user's desktop conversations (metadata only). The cloud
/// response is authoritative; the account-scoped SQLite cache only fills rows
/// that have not reached the cloud yet or keeps history available offline.
#[tauri::command]
pub async fn desktop_conv_list(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_account_access(state.inner()).await?;
    let cloud = product_cloud_request(
        "conversation.list",
        serde_json::json!({}),
        &app,
        state.inner(),
    )
    .await?;
    let cloud_unavailable = matches!(&cloud, ProductCloudOutcome::Unavailable(_));
    let source = conversation_list_source(cloud)?;
    if source.auth_expired {
        let _ = app.emit("cloud-auth-expired", ());
        tracing::warn!(
            "desktop cloud list authorization expired; refreshing and using local acknowledged cache"
        );
    } else if cloud_unavailable {
        tracing::warn!("desktop cloud list unavailable; using local acknowledged cache");
    }
    let merged = crate::trajectory::merge_local_summaries(
        crate::trajectory::outbox_path(&app)?,
        access.owner_scope,
        source.rows,
        source.cloud_available,
    )
    .await?;
    Ok(Value::Array(merged))
}

#[cfg(test)]
mod tests {
    use super::{conversation_list_source, ProductCloudOutcome};
    use serde_json::json;

    #[test]
    fn expired_list_authorization_uses_cache_and_requests_refresh() {
        let source = conversation_list_source(ProductCloudOutcome::Unauthorized("expired".into()))
            .expect("expired authorization should preserve readable local history");

        assert!(source.rows.is_empty());
        assert!(!source.cloud_available);
        assert!(source.auth_expired);
    }

    #[test]
    fn successful_list_remains_cloud_authoritative() {
        let source = conversation_list_source(ProductCloudOutcome::Ok(json!([
            { "id": "conversation-1" }
        ])))
        .expect("cloud list should decode");

        assert_eq!(source.rows, vec![json!({ "id": "conversation-1" })]);
        assert!(source.cloud_available);
        assert!(!source.auth_expired);
    }

    #[test]
    fn rejected_list_does_not_masquerade_as_offline_recovery() {
        let error = conversation_list_source(ProductCloudOutcome::Rejected("forbidden".into()))
            .err()
            .expect("permanent rejection should remain terminal");

        assert_eq!(error, "forbidden");
    }
}

/// Fetch one desktop conversation including its full snapshot blob.
#[tauri::command]
pub async fn desktop_conv_get(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let access = current_account_access(state.inner()).await?;
    let cloud = product_cloud_request(
        "conversation.get",
        serde_json::json!({ "id": id }),
        &app,
        state.inner(),
    )
    .await?;
    let cloud_detail = match cloud {
        ProductCloudOutcome::Ok(detail) => Some(detail),
        ProductCloudOutcome::Unauthorized(error) => {
            let _ = app.emit("cloud-auth-expired", ());
            return Err(error);
        }
        ProductCloudOutcome::Unavailable(error) => {
            tracing::warn!(%error, conversation_id = %id, "desktop cloud get temporarily unavailable; using local acknowledged cache");
            None
        }
        ProductCloudOutcome::NotFound(error)
        | ProductCloudOutcome::Conflict(error)
        | ProductCloudOutcome::Rejected(error) => return Err(error),
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
            "desktop conversation {id} is unavailable locally and in product cloud"
        )),
    }
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
    let access = current_account_access(state.inner()).await?;
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
    let summary = match product_cloud_request(
        "conversation.put",
        serde_json::json!({
            "id": id,
            "title": title,
            "provider": provider,
            "project": project,
            "repositoryFingerprint": repository_fingerprint,
            "remoteHost": remote_host,
            "mode": mode,
            "titleLocked": title_locked,
            "specialistContext": specialist_context,
            "rev": rev,
            "snapshot": typed_snapshot,
            "status": status,
            "baseRev": base_rev,
            "mutationId": mutation_id,
        }),
        &app,
        state.inner(),
    )
    .await?
    {
        ProductCloudOutcome::NotFound(error) => {
            let _ = app.emit("cloud-conversation-deleted", &id);
            return Err(format!(
                "cloud_deleted: this conversation was deleted on another device: {error}"
            ));
        }
        ProductCloudOutcome::Conflict(error) => {
            crate::trajectory::quarantine_snapshot_branch(
                crate::trajectory::outbox_path(&app)?,
                owner_scope,
                id,
            )
            .await?;
            return Err(error);
        }
        ProductCloudOutcome::Ok(summary) => summary,
        ProductCloudOutcome::Unauthorized(error)
        | ProductCloudOutcome::Unavailable(error)
        | ProductCloudOutcome::Rejected(error) => {
            tracing::warn!(
                event = "conversation_cloud_checkpoint_failed",
                conversation_id = %id,
                provider,
                "product cloud rejected the conversation checkpoint"
            );
            return Err(error);
        }
    };
    let stored_rev = summary
        .get("rev")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if stored_rev > rev {
        return Err(format!(
            "cloud_conflict: product cloud revision {stored_rev} is newer than local revision {rev}"
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
