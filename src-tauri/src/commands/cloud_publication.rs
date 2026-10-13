use super::cloud_authority::current_account_access;
use super::*;
use agent_core::TranscriptPage;
use serde::Deserialize;

pub(super) enum PublicationFailure {
    Retry(String),
    Conflict(String),
    Deleted(String),
    Rejected(String),
}

mod head;
use head::{append_transcript_pages, publish_snapshot_head};

mod pages;
use pages::prepare_paged_snapshot;

const TRANSCRIPT_BATCH_TARGET_BYTES: usize = 24 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationPublication {
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
    base_rev: i64,
    status: Option<String>,
    mutation_id: String,
    #[serde(default)]
    retry_attempt: u32,
}

/// Publish pages and a bounded head after `desktop_conv_commit_pending` has
/// durably staged the full projection. A failed publication retries the same
/// mutation id without trusting a renderer-owned page decision.
#[tauri::command]
pub async fn desktop_conv_publish(
    app: AppHandle,
    publication: ConversationPublication,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let retry_attempt = publication.retry_attempt;
    let access = match current_account_access(state.inner()).await {
        Ok(access) => access,
        Err(error) => {
            return Ok(publication_failure_value(
                PublicationFailure::Retry(error),
                retry_attempt,
            ))
        }
    };
    let cursor_key = (access.owner_scope, publication.id.clone());
    let already_sealed_through = state
        .history_cursors
        .lock()
        .await
        .get(&cursor_key)
        .copied()
        .unwrap_or(0);
    let history_cursors = state.history_cursors.clone();
    let staged = crate::trajectory::publication_snapshot(
        crate::trajectory::outbox_path(&app)?,
        cursor_key.0.clone(),
        publication.id.clone(),
        publication.mutation_id.clone(),
    )
    .await?;
    let full = match staged {
        crate::trajectory::PublicationSnapshot::Pending(snapshot) => snapshot,
        crate::trajectory::PublicationSnapshot::Published(snapshot, stored_rev) => {
            state
                .history_cursors
                .lock()
                .await
                .entry(cursor_key)
                .and_modify(|cursor| *cursor = (*cursor).max(snapshot.timeline_offset))
                .or_insert(snapshot.timeline_offset);
            return Ok(serde_json::json!({
                "outcome": "published",
                "rev": stored_rev,
                "sealedThrough": snapshot.timeline_offset,
                "large": serde_json::to_vec(&snapshot).map_err(|error| error.to_string())?.len()
                    > 8 * 1024 * 1024,
            }));
        }
        crate::trajectory::PublicationSnapshot::Superseded => {
            return Ok(serde_json::json!({ "outcome": "superseded" }));
        }
    };
    let (head, pages) = match prepare_paged_snapshot(full, already_sealed_through) {
        Ok(prepared) => prepared,
        Err(error) => {
            let quarantine = async {
                let access = super::cloud_authority::current_account_access(state.inner()).await?;
                crate::trajectory::quarantine_snapshot_branch(
                    crate::trajectory::outbox_path(&app)?,
                    access.owner_scope,
                    publication.id.clone(),
                )
                .await
            }
            .await;
            return Ok(publication_failure_value(
                match quarantine {
                    Ok(()) => PublicationFailure::Conflict(error),
                    Err(error) => PublicationFailure::Retry(error),
                },
                retry_attempt,
            ));
        }
    };
    let sealed_through = if pages.is_empty() {
        already_sealed_through
    } else {
        head.timeline_offset
    };
    let large = serde_json::to_vec(&head)
        .map_err(|error| error.to_string())?
        .len()
        > 8 * 1024 * 1024;

    let mut batch = Vec::new();
    let mut batch_bytes = 0usize;
    for page in pages {
        let page_bytes = serde_json::to_vec(&page)
            .map_err(|error| error.to_string())?
            .len();
        if !batch.is_empty()
            && (batch.len() == 4
                || batch_bytes.saturating_add(page_bytes) > TRANSCRIPT_BATCH_TARGET_BYTES)
        {
            if let Err(failure) = append_pages(
                &app,
                &publication,
                std::mem::take(&mut batch),
                state.clone(),
            )
            .await
            {
                return Ok(publication_failure_value(failure, retry_attempt));
            }
            batch_bytes = 0;
        }
        batch_bytes += page_bytes;
        batch.push(page);
    }
    if !batch.is_empty() {
        if let Err(failure) = append_pages(&app, &publication, batch, state.clone()).await {
            return Ok(publication_failure_value(failure, retry_attempt));
        }
    }

    let summary = publish_snapshot_head(
        app,
        publication.id,
        publication.title,
        publication.provider,
        publication.project,
        publication.repository_fingerprint,
        publication.remote_host,
        publication.mode,
        publication.title_locked,
        publication.specialist_context,
        publication.rev,
        serde_json::to_value(head).map_err(|error| error.to_string())?,
        publication.status,
        Some(publication.base_rev),
        publication.mutation_id,
        state,
    )
    .await;
    let mut summary = match summary {
        Ok(summary) => summary,
        Err(failure) => return Ok(publication_failure_value(failure, retry_attempt)),
    };
    let result = summary
        .as_object_mut()
        .ok_or("product cloud publication summary is not an object")?;
    result.insert("sealedThrough".into(), sealed_through.into());
    result.insert("large".into(), large.into());
    result.insert("outcome".into(), "published".into());
    history_cursors
        .lock()
        .await
        .entry(cursor_key)
        .and_modify(|cursor| {
            *cursor = (*cursor).max(sealed_through);
        })
        .or_insert(sealed_through);
    Ok(summary)
}

fn publication_failure_value(failure: PublicationFailure, retry_attempt: u32) -> Value {
    match failure {
        PublicationFailure::Retry(message) => serde_json::json!({
            "outcome": "retry",
            "message": message,
            "retryAfterMs": (1_000u64 << retry_attempt.min(5)).min(30_000),
        }),
        PublicationFailure::Conflict(message) => {
            serde_json::json!({ "outcome": "conflict", "message": message })
        }
        PublicationFailure::Deleted(message) => {
            serde_json::json!({ "outcome": "deleted", "message": message })
        }
        PublicationFailure::Rejected(message) => {
            serde_json::json!({ "outcome": "rejected", "message": message })
        }
    }
}

async fn append_pages(
    app: &AppHandle,
    publication: &ConversationPublication,
    pages: Vec<TranscriptPage>,
    state: State<'_, AppState>,
) -> Result<(), PublicationFailure> {
    append_transcript_pages(
        app.clone(),
        publication.id.clone(),
        publication.title.clone(),
        publication.provider.clone(),
        publication.project.clone(),
        publication.repository_fingerprint.clone(),
        publication.remote_host.clone(),
        publication.mode.clone(),
        pages,
        state,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::TimelineItem;

    #[test]
    fn prepares_only_pages_after_the_acknowledged_cloud_prefix() {
        let mut snapshot = Snapshot::new();
        snapshot.history_checkpoint = Some(37);
        snapshot.timeline = (0..480)
            .map(|index| TimelineItem::Message {
                run: agent_core::RunId::new("run"),
                role: agent_core::Role::User,
                blocks: vec![agent_core::ContentBlock::Text {
                    text: format!("message-{index}"),
                }],
                phase: None,
                stream_boundary: false,
            })
            .collect();
        snapshot.model_context_checkpoint = Some(agent_core::projection::ModelContextCheckpoint {
            transcript: agent_core::ResumeTranscript::default(),
            timeline_index: 480,
        });
        let (head, pages) = prepare_paged_snapshot(snapshot, 240).expect("paged publication");
        assert_eq!(pages.first().map(|page| page.start_index), Some(240));
        assert_eq!(pages.iter().map(|page| page.items.len()).sum::<usize>(), 80);
        assert_eq!(head.timeline_offset, 320);
        assert_eq!(head.timeline.len(), 160);
        assert_eq!(head.history_checkpoint, Some(37));
    }

    #[test]
    fn a_cursor_outside_the_full_snapshot_is_a_conflict() {
        let mut snapshot = Snapshot::new();
        snapshot.timeline_offset = 240;
        assert!(prepare_paged_snapshot(snapshot.clone(), 0).is_err());
        assert!(prepare_paged_snapshot(snapshot, 241).is_err());
    }

    #[test]
    fn native_retry_policy_is_bounded_and_typed() {
        let first = publication_failure_value(PublicationFailure::Retry("offline".into()), 0);
        let late = publication_failure_value(PublicationFailure::Retry("offline".into()), 10);
        assert_eq!(first["outcome"], "retry");
        assert_eq!(first["retryAfterMs"], 1_000);
        assert_eq!(late["retryAfterMs"], 30_000);
    }
}
