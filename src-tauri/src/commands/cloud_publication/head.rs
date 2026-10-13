use super::super::cloud_conversations::{product_cloud_request, ProductCloudOutcome};
use super::*;
use tauri::Emitter;

#[allow(clippy::too_many_arguments)]
pub(super) async fn append_transcript_pages(
    app: AppHandle,
    id: String,
    title: String,
    provider: String,
    project: Option<String>,
    repository_fingerprint: Option<String>,
    remote_host: Option<String>,
    mode: Option<String>,
    pages: Vec<agent_core::TranscriptPage>,
    state: State<'_, AppState>,
) -> Result<(), PublicationFailure> {
    let access = current_account_access(state.inner())
        .await
        .map_err(PublicationFailure::Retry)?;
    if pages.is_empty() || pages.len() > 4 {
        return Err(PublicationFailure::Rejected(
            "transcript page batch must contain 1 to 4 pages".into(),
        ));
    }
    match product_cloud_request(
        "conversation.append_transcript_pages",
        serde_json::json!({
            "id": id,
            "title": title,
            "provider": provider,
            "project": project,
            "repositoryFingerprint": repository_fingerprint,
            "remoteHost": remote_host,
            "mode": mode,
            "pages": pages,
        }),
        &app,
        state.inner(),
    )
    .await
    .map_err(PublicationFailure::Retry)?
    {
        ProductCloudOutcome::Ok(_) => Ok(()),
        ProductCloudOutcome::Unauthorized(error) => {
            let _ = app.emit("cloud-auth-expired", ());
            Err(PublicationFailure::Retry(error))
        }
        ProductCloudOutcome::NotFound(error) => Err(PublicationFailure::Deleted(error)),
        ProductCloudOutcome::Conflict(error) => {
            crate::trajectory::quarantine_snapshot_branch(
                crate::trajectory::outbox_path(&app).map_err(PublicationFailure::Retry)?,
                access.owner_scope,
                id,
            )
            .await
            .map_err(PublicationFailure::Retry)?;
            Err(PublicationFailure::Conflict(error))
        }
        ProductCloudOutcome::Unavailable(error) => Err(PublicationFailure::Retry(error)),
        ProductCloudOutcome::Rejected(error) => Err(PublicationFailure::Rejected(error)),
    }
}

/// Insert or replace a desktop conversation snapshot.
#[allow(clippy::too_many_arguments)]
pub(super) async fn publish_snapshot_head(
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
    mutation_id: String,
    state: State<'_, AppState>,
) -> Result<Value, PublicationFailure> {
    let access = current_account_access(state.inner())
        .await
        .map_err(PublicationFailure::Retry)?;
    let owner_scope = access.owner_scope;
    let local_live = status.as_deref() == Some("running");
    let checkpoint_seq = snapshot
        .get("history_checkpoint")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    crate::trajectory::wait_for_acknowledged_prefix(
        crate::trajectory::outbox_path(&app).map_err(PublicationFailure::Retry)?,
        owner_scope.clone(),
        id.clone(),
        checkpoint_seq,
        std::time::Duration::from_secs(10),
    )
    .await
    .map_err(PublicationFailure::Retry)?;
    if let Some(object) = snapshot.as_object_mut() {
        object.remove("history_checkpoint");
        object.remove("sync_pending");
    }
    snapshot = crate::trajectory::normalize_snapshot_value(snapshot);
    let typed_snapshot: Snapshot = serde_json::from_value(snapshot).map_err(|error| {
        PublicationFailure::Rejected(format!("checkpoint desktop snapshot: {error}"))
    })?;
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
    let pending_mutation_id = mutation_id.clone();
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
    .await
    .map_err(PublicationFailure::Retry)?
    {
        ProductCloudOutcome::NotFound(error) => {
            let _ = app.emit("cloud-conversation-deleted", &id);
            return Err(PublicationFailure::Deleted(error));
        }
        ProductCloudOutcome::Conflict(error) => {
            crate::trajectory::quarantine_snapshot_branch(
                crate::trajectory::outbox_path(&app).map_err(PublicationFailure::Retry)?,
                owner_scope.clone(),
                id.clone(),
            )
            .await
            .map_err(PublicationFailure::Retry)?;
            return Err(PublicationFailure::Conflict(error));
        }
        ProductCloudOutcome::Ok(summary) => summary,
        ProductCloudOutcome::Unauthorized(error) => {
            let _ = app.emit("cloud-auth-expired", ());
            return Err(PublicationFailure::Retry(error));
        }
        ProductCloudOutcome::Unavailable(error) => {
            tracing::warn!(
                event = "conversation_cloud_checkpoint_failed",
                conversation_id = %id,
                provider,
                "product cloud rejected the conversation checkpoint"
            );
            return Err(PublicationFailure::Retry(error));
        }
        ProductCloudOutcome::Rejected(error) => return Err(PublicationFailure::Rejected(error)),
    };
    let stored_rev = summary.get("rev").and_then(Value::as_i64).ok_or_else(|| {
        PublicationFailure::Retry("product cloud returned no conversation revision".into())
    })?;
    if stored_rev > rev {
        crate::trajectory::quarantine_snapshot_branch(
            crate::trajectory::outbox_path(&app).map_err(PublicationFailure::Retry)?,
            owner_scope.clone(),
            id.clone(),
        )
        .await
        .map_err(PublicationFailure::Retry)?;
        return Err(PublicationFailure::Conflict(format!(
            "cloud_conflict: product cloud revision {stored_rev} is newer than local revision {rev}"
        )));
    }
    let acknowledged = crate::trajectory::acknowledge_snapshot_publication(
        crate::trajectory::outbox_path(&app).map_err(PublicationFailure::Retry)?,
        owner_scope,
        id.clone(),
        checkpoint_metadata,
        typed_snapshot,
        stored_rev,
        checkpoint_seq,
        local_live,
        pending_mutation_id.clone(),
    )
    .await
    .map_err(PublicationFailure::Retry)?;
    if !acknowledged {
        tracing::debug!(
            event = "conversation_snapshot_ack_superseded",
            conversation_id = %id,
            mutation_id = %pending_mutation_id,
            "ignored a cloud acknowledgement for a superseded local snapshot"
        );
    }
    Ok(summary)
}
