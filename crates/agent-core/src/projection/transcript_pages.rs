use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::domain::{Artifact, RunStatus, ToolCall};
use crate::ids::ToolCallId;
use crate::recovery::ProviderIncident;

use super::{Snapshot, TimelineItem};

/// Page size is aligned with the renderer's fixed history window. Pages are
/// immutable once sealed; the larger live tail absorbs streaming mutations.
pub const TRANSCRIPT_PAGE_ITEMS: usize = 80;
pub const TRANSCRIPT_TAIL_ITEMS: usize = TRANSCRIPT_PAGE_ITEMS * 2;
/// Leaves headroom below the service's 8 MiB hard row limit for referenced
/// tool/artifact metadata and JSON container overhead.
pub const TRANSCRIPT_PAGE_TARGET_BYTES: usize = 6 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPage {
    pub start_index: usize,
    pub items: Vec<TimelineItem>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub tool_calls: IndexMap<ToolCallId, ToolCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub provider_incidents: IndexMap<String, ProviderIncident>,
}

impl Snapshot {
    /// Move the model-compacted, immutable prefix into bounded pages.
    ///
    /// The provider checkpoint is the proof that removed rows are no longer
    /// needed to reconstruct model-visible context. Active runs never seal:
    /// their messages, tools, and presentation records can still change.
    pub fn seal_transcript_pages(&mut self) -> Vec<TranscriptPage> {
        if self.starting
            || self.runs.values().any(|run| {
                matches!(
                    run.status,
                    RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingInput
                )
            })
        {
            return Vec::new();
        }
        let Some(checkpoint) = self.model_context_checkpoint.as_ref() else {
            return Vec::new();
        };
        let compacted_local_end = checkpoint
            .timeline_index
            .saturating_sub(self.timeline_offset)
            .min(self.timeline.len());
        let seal_count = compacted_local_end.saturating_sub(TRANSCRIPT_TAIL_ITEMS);
        if seal_count == 0 {
            return Vec::new();
        }

        let page_start = self.timeline_offset;
        let sealed = self.timeline.drain(..seal_count).collect::<Vec<_>>();
        self.timeline_offset += seal_count;
        // Consume the drained items into pages. Cloning here would briefly
        // double the archived transcript and defeat the memory bound exactly
        // when a large history is first migrated.
        let mut pages = Vec::new();
        let mut page_items = Vec::new();
        let mut page_bytes = 0usize;
        let mut next_start = page_start;
        for item in sealed {
            let item_bytes = transcript_item_wire_bytes(self, &item);
            if !page_items.is_empty()
                && (page_items.len() == TRANSCRIPT_PAGE_ITEMS
                    || page_bytes.saturating_add(item_bytes) > TRANSCRIPT_PAGE_TARGET_BYTES)
            {
                next_start += page_items.len();
                pages.push(TranscriptPage {
                    start_index: next_start - page_items.len(),
                    items: std::mem::take(&mut page_items),
                    tool_calls: IndexMap::new(),
                    artifacts: Vec::new(),
                    provider_incidents: IndexMap::new(),
                });
                page_bytes = 0;
            }
            page_bytes = page_bytes.saturating_add(item_bytes);
            page_items.push(item);
        }
        if !page_items.is_empty() {
            pages.push(TranscriptPage {
                start_index: next_start,
                items: page_items,
                tool_calls: IndexMap::new(),
                artifacts: Vec::new(),
                provider_incidents: IndexMap::new(),
            });
        }

        for page in &mut pages {
            for item in &page.items {
                match item {
                    TimelineItem::ToolCall { id, .. }
                        if !timeline_references_tool(&self.timeline, id) =>
                    {
                        if let Some(call) = self.tool_calls.shift_remove(id) {
                            page.tool_calls.insert(id.clone(), call);
                        }
                    }
                    TimelineItem::Artifact { id }
                        if !timeline_references_artifact(&self.timeline, id) =>
                    {
                        if let Some(position) = self
                            .artifacts
                            .iter()
                            .position(|artifact| artifact.id == *id)
                        {
                            page.artifacts.push(self.artifacts.remove(position));
                        }
                    }
                    TimelineItem::ProviderIncident { id, .. }
                        if !timeline_references_incident(&self.timeline, id) =>
                    {
                        if let Some(incident) = self.provider_incidents.shift_remove(id) {
                            page.provider_incidents.insert(id.clone(), incident);
                        }
                    }
                    _ => {}
                }
            }
        }
        pages
    }
}

fn transcript_item_wire_bytes(snapshot: &Snapshot, item: &TimelineItem) -> usize {
    let mut bytes = serde_json::to_vec(item).map_or(0, |value| value.len());
    match item {
        TimelineItem::ToolCall { id, .. } => {
            if let Some(call) = snapshot.tool_calls.get(id) {
                bytes =
                    bytes.saturating_add(serde_json::to_vec(call).map_or(0, |value| value.len()));
            }
        }
        TimelineItem::Artifact { id } => {
            if let Some(artifact) = snapshot
                .artifacts
                .iter()
                .find(|artifact| artifact.id == *id)
            {
                bytes = bytes
                    .saturating_add(serde_json::to_vec(artifact).map_or(0, |value| value.len()));
            }
        }
        TimelineItem::ProviderIncident { id, .. } => {
            if let Some(incident) = snapshot.provider_incidents.get(id) {
                bytes = bytes
                    .saturating_add(serde_json::to_vec(incident).map_or(0, |value| value.len()));
            }
        }
        _ => {}
    }
    // Commas, keys, and braces are small but cumulative for tiny rows.
    bytes.saturating_add(64)
}

fn timeline_references_tool(timeline: &[TimelineItem], id: &ToolCallId) -> bool {
    timeline
        .iter()
        .any(|item| matches!(item, TimelineItem::ToolCall { id: current, .. } if current == id))
}

fn timeline_references_artifact(timeline: &[TimelineItem], id: &str) -> bool {
    timeline
        .iter()
        .any(|item| matches!(item, TimelineItem::Artifact { id: current } if current == id))
}

fn timeline_references_incident(timeline: &[TimelineItem], id: &str) -> bool {
    timeline.iter().any(
        |item| matches!(item, TimelineItem::ProviderIncident { id: current, .. } if current == id),
    )
}

#[cfg(test)]
mod tests {
    use crate::domain::{ContentBlock, Role, RunStatus};
    use crate::ids::RunId;
    use crate::projection::{ModelContextCheckpoint, RunView};
    use crate::provider::{ResumeItem, ResumeTranscript};

    use super::*;

    #[test]
    fn sealing_keeps_a_bounded_tail_and_absolute_context_boundary() {
        let run = RunId::new("run-1");
        let mut snapshot = Snapshot::new();
        snapshot.runs.insert(
            run.clone(),
            RunView {
                id: run.clone(),
                status: RunStatus::Done,
                usage: None,
                outcome: None,
                checkpoint: None,
            },
        );
        snapshot.timeline = (0..400)
            .map(|index| TimelineItem::Message {
                run: run.clone(),
                role: Role::User,
                blocks: vec![ContentBlock::Text {
                    text: format!("message-{index}"),
                }],
                phase: None,
                stream_boundary: false,
            })
            .collect();
        snapshot.model_context_checkpoint = Some(ModelContextCheckpoint {
            transcript: ResumeTranscript {
                items: vec![ResumeItem::Message {
                    role: Role::User,
                    blocks: vec![ContentBlock::Text {
                        text: "compacted history".into(),
                    }],
                }],
                truncated: true,
            },
            timeline_index: 400,
        });

        let pages = snapshot.seal_transcript_pages();

        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].start_index, 0);
        assert_eq!(pages[2].start_index, 160);
        assert!(pages
            .iter()
            .all(|page| page.items.len() == TRANSCRIPT_PAGE_ITEMS));
        assert_eq!(snapshot.timeline_offset, 240);
        assert_eq!(snapshot.timeline.len(), TRANSCRIPT_TAIL_ITEMS);
        assert_eq!(
            snapshot.resume_transcript().unwrap().items,
            snapshot
                .model_context_checkpoint
                .as_ref()
                .unwrap()
                .transcript
                .items
        );
    }

    #[test]
    fn active_or_uncompacted_history_never_seals() {
        let run = RunId::new("run-1");
        let mut snapshot = Snapshot::new();
        snapshot.timeline = (0..400)
            .map(|index| TimelineItem::Message {
                run: run.clone(),
                role: Role::User,
                blocks: vec![ContentBlock::Text {
                    text: format!("message-{index}"),
                }],
                phase: None,
                stream_boundary: false,
            })
            .collect();
        assert!(snapshot.seal_transcript_pages().is_empty());

        snapshot.runs.insert(
            run.clone(),
            RunView {
                id: run,
                status: RunStatus::Running,
                usage: None,
                outcome: None,
                checkpoint: None,
            },
        );
        snapshot.model_context_checkpoint = Some(ModelContextCheckpoint {
            transcript: ResumeTranscript::default(),
            timeline_index: 400,
        });
        assert!(snapshot.seal_transcript_pages().is_empty());
        assert_eq!(snapshot.timeline.len(), 400);
    }

    #[test]
    fn sealing_is_bounded_by_wire_bytes_as_well_as_item_count() {
        let run = RunId::new("run-1");
        let mut snapshot = Snapshot::new();
        snapshot.timeline = (0..170)
            .map(|_| TimelineItem::Message {
                run: run.clone(),
                role: Role::User,
                blocks: vec![ContentBlock::Text {
                    text: "x".repeat(1024 * 1024),
                }],
                phase: None,
                stream_boundary: false,
            })
            .collect();
        snapshot.model_context_checkpoint = Some(ModelContextCheckpoint {
            transcript: ResumeTranscript::default(),
            timeline_index: 170,
        });

        let pages = snapshot.seal_transcript_pages();

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].items.len(), 5);
        assert_eq!(pages[1].items.len(), 5);
        assert_eq!(pages[1].start_index, 5);
        assert_eq!(snapshot.timeline_offset, 10);
        assert_eq!(snapshot.timeline.len(), TRANSCRIPT_TAIL_ITEMS);
        assert!(pages
            .iter()
            .all(|page| { serde_json::to_vec(page).unwrap().len() < 8 * 1024 * 1024 }));
    }
}
