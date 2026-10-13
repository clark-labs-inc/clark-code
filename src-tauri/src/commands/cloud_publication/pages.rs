use agent_core::{Snapshot, TimelineItem, TranscriptPage};
use std::collections::HashSet;

const TRANSCRIPT_PAGE_HARD_BYTES: usize = 32 * 1024 * 1024;

pub(super) fn prepare_paged_snapshot(
    mut snapshot: Snapshot,
    already_sealed_through: usize,
) -> Result<(Snapshot, Vec<TranscriptPage>), String> {
    let base = snapshot.timeline_offset;
    let local_start = already_sealed_through
        .checked_sub(base)
        .ok_or("conversation snapshot is missing cloud transcript pages")?;
    let checkpoint = snapshot
        .model_context_checkpoint
        .as_ref()
        .map(|value| value.timeline_index);
    let active = snapshot.starting
        || snapshot.runs.values().any(|run| {
            matches!(
                run.status,
                agent_core::RunStatus::Queued
                    | agent_core::RunStatus::Running
                    | agent_core::RunStatus::AwaitingInput
            )
        });
    let compacted_end = checkpoint
        .unwrap_or_default()
        .saturating_sub(base)
        .min(snapshot.timeline.len());
    let seal_end = compacted_end.saturating_sub(agent_core::projection::TRANSCRIPT_TAIL_ITEMS);
    if local_start > snapshot.timeline.len() {
        return Err("cloud transcript is ahead of this conversation snapshot".into());
    }
    snapshot.timeline.drain(..local_start);
    snapshot.timeline_offset = already_sealed_through;
    if active || seal_end <= local_start {
        prune_head_references(&mut snapshot);
        return Ok((snapshot, Vec::new()));
    }
    let pages = snapshot.seal_transcript_pages();
    if pages.is_empty() {
        return Err("conversation page boundary changed during publication".into());
    }
    for page in &pages {
        if serde_json::to_vec(page)
            .map_err(|error| error.to_string())?
            .len()
            > TRANSCRIPT_PAGE_HARD_BYTES
        {
            return Err("one transcript record exceeds the immutable page boundary".into());
        }
    }
    prune_head_references(&mut snapshot);
    Ok((snapshot, pages))
}

fn prune_head_references(snapshot: &mut Snapshot) {
    let tool_ids: HashSet<_> = snapshot
        .timeline
        .iter()
        .filter_map(|item| match item {
            TimelineItem::ToolCall { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    let artifact_ids: HashSet<_> = snapshot
        .timeline
        .iter()
        .filter_map(|item| match item {
            TimelineItem::Artifact { id } => Some(id.clone()),
            _ => None,
        })
        .collect();
    let incident_ids: HashSet<_> = snapshot
        .timeline
        .iter()
        .filter_map(|item| match item {
            TimelineItem::ProviderIncident { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    snapshot.tool_calls.retain(|id, _| tool_ids.contains(id));
    snapshot
        .artifacts
        .retain(|artifact| artifact_ids.contains(&artifact.id));
    snapshot
        .provider_incidents
        .retain(|id, _| incident_ids.contains(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_publication_excludes_the_already_immutable_prefix() {
        let mut snapshot = Snapshot::new();
        snapshot.starting = true;
        snapshot.timeline = (0..400)
            .map(|index| TimelineItem::Message {
                run: agent_core::RunId::new("run"),
                role: agent_core::Role::Agent,
                blocks: vec![agent_core::ContentBlock::text(format!("token-{index}"))],
                phase: None,
                stream_boundary: true,
            })
            .collect();
        let (head, pages) = prepare_paged_snapshot(snapshot, 240).unwrap();
        assert_eq!(head.timeline_offset, 240);
        assert_eq!(head.timeline.len(), 160);
        assert!(pages.is_empty());
    }

    #[test]
    fn one_large_record_stays_whole_below_the_singleton_limit() {
        let mut snapshot = Snapshot::new();
        snapshot.timeline = (0..161)
            .map(|index| TimelineItem::Message {
                run: agent_core::RunId::new("run"),
                role: agent_core::Role::User,
                blocks: vec![agent_core::ContentBlock::Text {
                    text: if index == 0 {
                        "x".repeat(12 * 1024 * 1024)
                    } else {
                        "small".into()
                    },
                }],
                phase: None,
                stream_boundary: false,
            })
            .collect();
        snapshot.model_context_checkpoint = Some(agent_core::projection::ModelContextCheckpoint {
            transcript: agent_core::ResumeTranscript::default(),
            timeline_index: 161,
        });

        let (head, pages) = prepare_paged_snapshot(snapshot, 0).expect("singleton page");
        assert_eq!(head.timeline.len(), 160);
        assert_eq!(pages.len(), 1);
        let bytes = serde_json::to_vec(&pages[0]).unwrap().len();
        assert!(bytes > 8 * 1024 * 1024);
        assert!(bytes <= TRANSCRIPT_PAGE_HARD_BYTES);
    }
}
