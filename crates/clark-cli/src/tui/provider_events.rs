use agent_core::{AgentEvent, ArtifactKind, ContentBlock, PermissionRequest, RunId, RunUsage};

use super::render::TranscriptKind;
use super::specialists::CloudContinuity;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SteeringDisposition {
    Delivered,
    QueueFollowUp,
    RestoreInput(String),
}

pub(crate) fn classify_steering_result(result: agent_core::Result<()>) -> SteeringDisposition {
    match result {
        Ok(()) => SteeringDisposition::Delivered,
        Err(agent_core::Error::Unsupported(_)) => SteeringDisposition::QueueFollowUp,
        Err(error) => SteeringDisposition::RestoreInput(error.to_string()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TranscriptEffect {
    AppendClark(String),
    Push { kind: TranscriptKind, text: String },
}

#[derive(Debug)]
pub(crate) struct ProviderEventState {
    pub(crate) status: String,
    pub(crate) running: bool,
    pub(crate) current_run: Option<RunId>,
    pub(crate) pending_permission: Option<PermissionRequest>,
    pub(crate) usage: Option<RunUsage>,
    pub(crate) cloud_continuity: CloudContinuity,
    event_sequence: u64,
}

impl Default for ProviderEventState {
    fn default() -> Self {
        Self {
            status: "ready".into(),
            running: false,
            current_run: None,
            pending_permission: None,
            usage: None,
            cloud_continuity: CloudContinuity::default(),
            event_sequence: 0,
        }
    }
}

impl ProviderEventState {
    pub(crate) fn apply(&mut self, event: &AgentEvent) -> Vec<TranscriptEffect> {
        self.event_sequence = self.event_sequence.saturating_add(1);
        let mut effects = Vec::new();
        match event {
            AgentEvent::RunStarted { run } => {
                self.current_run = Some(run.clone());
                self.running = true;
                self.status = "working".into();
            }
            AgentEvent::Checkpoint { .. } => self.status = "checkpoint saved".into(),
            AgentEvent::MessageChunk {
                delta: ContentBlock::Text { text },
                ..
            } => effects.push(TranscriptEffect::AppendClark(text.clone())),
            AgentEvent::MessageChunk {
                delta: ContentBlock::Thinking { .. },
                ..
            } => self.status = "thinking".into(),
            AgentEvent::MessageChunk {
                delta: ContentBlock::Image { mime_type, uri, .. },
                ..
            } => effects.push(TranscriptEffect::Push {
                kind: TranscriptKind::Artifact,
                text: format!(
                    "Image · {mime_type}{}",
                    uri.as_deref()
                        .map(|uri| format!(" · {uri}"))
                        .unwrap_or_default()
                ),
            }),
            AgentEvent::MessageChunk {
                delta: ContentBlock::Audio { mime_type, .. },
                ..
            } => effects.push(TranscriptEffect::Push {
                kind: TranscriptKind::Artifact,
                text: format!("Audio · {mime_type}"),
            }),
            AgentEvent::MessageChunk {
                delta: ContentBlock::Resource { uri, mime_type, .. },
                ..
            } => effects.push(TranscriptEffect::Push {
                kind: TranscriptKind::Artifact,
                text: format!(
                    "Resource{} · {uri}",
                    mime_type
                        .as_deref()
                        .map(|mime| format!(" · {mime}"))
                        .unwrap_or_default()
                ),
            }),
            AgentEvent::MessageChunk {
                delta: ContentBlock::ResourceLink { uri, name },
                ..
            } => effects.push(TranscriptEffect::Push {
                kind: TranscriptKind::Artifact,
                text: format!("{} · {uri}", name.as_deref().unwrap_or("Resource link")),
            }),
            AgentEvent::MessageChunk {
                delta: ContentBlock::SkillReference { name, revision, .. },
                ..
            } => effects.push(TranscriptEffect::Push {
                kind: TranscriptKind::System,
                text: format!("Skill · {name} · revision {revision}"),
            }),
            AgentEvent::ToolCall { call, .. } => {
                self.status = format!("tool · {}", call.title);
                effects.push(TranscriptEffect::Push {
                    kind: TranscriptKind::Tool,
                    text: call.title.clone(),
                });
            }
            AgentEvent::ToolCallUpdate { patch, .. } => {
                if let Some(activity) = patch
                    .progress
                    .as_ref()
                    .and_then(|progress| progress.latest_activity.as_ref())
                {
                    self.status = activity.clone();
                }
            }
            AgentEvent::ExecutionChecklistUpdated { checklist, .. } => {
                let completed = checklist
                    .steps
                    .iter()
                    .filter(|step| step.status == agent_core::ChecklistStatus::Completed)
                    .count();
                self.status = format!("plan · {completed}/{}", checklist.steps.len());
            }
            AgentEvent::ProposedPlanUpdated { .. } => self.status = "plan updated".into(),
            AgentEvent::GoalUpdated { goal, .. } => {
                self.status = format!("goal · {:?}", goal.status).to_ascii_lowercase();
            }
            AgentEvent::RunUsageUpdated { usage, .. } => self.usage = Some(*usage),
            AgentEvent::SpecialistPresentation { presentation, .. } => {
                effects.push(TranscriptEffect::AppendClark(format!(
                    "\n{}\n\n{}",
                    presentation.summary, presentation.takeaway
                )));
            }
            AgentEvent::Artifact { artifact, .. } => {
                let location = artifact.uri.as_deref().unwrap_or("cloud artifact");
                effects.push(TranscriptEffect::Push {
                    kind: if artifact.kind == ArtifactKind::Diff {
                        TranscriptKind::Diff
                    } else {
                        TranscriptKind::Artifact
                    },
                    text: format!("{} · {location}", artifact.title),
                });
            }
            AgentEvent::PermissionRequest { request } => {
                self.pending_permission = Some(request.clone());
                self.status = "permission required".into();
            }
            AgentEvent::FanOut { agent, .. } => {
                self.status =
                    format!("agents · {} · {:?}", agent.label, agent.status).to_ascii_lowercase();
            }
            AgentEvent::ProviderIncidentUpdated { incident, .. } => {
                self.status =
                    format!("provider incident · {:?}", incident.status).to_ascii_lowercase();
            }
            AgentEvent::ModeChanged { mode, .. } => self.status = format!("mode · {mode}"),
            AgentEvent::ContextCompacted { .. } => {
                self.status = "context compacted".into();
                effects.push(TranscriptEffect::Push {
                    kind: TranscriptKind::System,
                    text: "Model context compacted; visible transcript retained.".into(),
                });
            }
            AgentEvent::Trace {
                source, payload, ..
            } if source == "clark_specialist_projection" => {
                match self.cloud_continuity.apply_projection(payload) {
                    Ok(Some(summary)) => {
                        self.status = "cloud synchronized".into();
                        effects.push(TranscriptEffect::Push {
                            kind: TranscriptKind::System,
                            text: summary,
                        });
                    }
                    Ok(None) => {
                        self.status = "cloud receipt rejected".into();
                        effects.push(TranscriptEffect::Push {
                            kind: TranscriptKind::Error,
                            text: "Clark specialist completion omitted its required cloud synchronization receipt".into(),
                        });
                    }
                    Err(error) => {
                        self.status = "cloud receipt rejected".into();
                        effects.push(TranscriptEffect::Push {
                            kind: TranscriptKind::Error,
                            text: error,
                        });
                    }
                }
            }
            AgentEvent::RunFinished { outcome, .. } => {
                self.status = match outcome.status {
                    agent_core::RunStatus::Queued => "queued",
                    agent_core::RunStatus::Running => "working",
                    agent_core::RunStatus::AwaitingInput => "awaiting input",
                    agent_core::RunStatus::Done => "complete",
                    agent_core::RunStatus::Cancelled => "cancelled",
                    agent_core::RunStatus::Failed => "failed",
                }
                .into();
                self.running = false;
                self.current_run = None;
                if outcome.usage.is_some() {
                    self.usage = outcome.usage;
                }
            }
            AgentEvent::Error { message, .. } => {
                effects.push(TranscriptEffect::Push {
                    kind: TranscriptKind::Error,
                    text: message.clone(),
                });
                self.status = "failed".into();
                self.running = false;
                self.current_run = None;
            }
            AgentEvent::MessagePhase { .. }
            | AgentEvent::Surface { .. }
            | AgentEvent::Trace { .. } => {}
        }
        effects
    }

    pub(crate) fn mark_starting(&mut self) {
        self.running = true;
        self.status = "starting".into();
    }

    pub(crate) fn mark_cancelling(&mut self) {
        self.status = "cancelling".into();
    }

    pub(crate) fn mark_stream_closed(&mut self) {
        self.running = false;
        self.current_run = None;
        if self.status == "working" {
            self.status = "complete".into();
        }
    }

    pub(crate) fn usage_label(&self) -> Option<String> {
        let usage = self.usage?;
        let mut label = format!(
            "{} in · {} out · {} ctx",
            compact_number(usage.input_tokens),
            compact_number(usage.output_tokens),
            compact_number(usage.context_tokens)
        );
        if let Some(cost) = usage.cost_usd {
            label.push_str(&format!(" · ${cost:.4}"));
        }
        Some(label)
    }

    #[cfg(test)]
    fn event_sequence(&self) -> u64 {
        self.event_sequence
    }
}

fn compact_number(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use agent_core::{ContentBlock, Role, RunOutcome, RunStatus};

    use super::*;

    #[test]
    fn ordered_streaming_events_project_without_a_second_protocol() {
        let run = RunId::new("run-1");
        let mut state = ProviderEventState::default();
        assert!(state
            .apply(&AgentEvent::RunStarted { run: run.clone() })
            .is_empty());
        let first = state.apply(&AgentEvent::MessageChunk {
            run: run.clone(),
            role: Role::Agent,
            delta: ContentBlock::Text { text: "one".into() },
        });
        let second = state.apply(&AgentEvent::MessageChunk {
            run: run.clone(),
            role: Role::Agent,
            delta: ContentBlock::Text {
                text: " two".into(),
            },
        });
        assert_eq!(first, vec![TranscriptEffect::AppendClark("one".into())]);
        assert_eq!(second, vec![TranscriptEffect::AppendClark(" two".into())]);
        assert_eq!(state.event_sequence(), 3);
        assert!(state.running);
        assert_eq!(state.current_run, Some(run));
    }

    #[test]
    fn cumulative_usage_and_terminal_outcome_are_visible() {
        let run = RunId::new("run-usage");
        let usage = RunUsage {
            input_tokens: 1_250,
            output_tokens: 80,
            context_tokens: 9_500,
            cost_usd: Some(0.0123),
            context_limit: Some(100_000),
        };
        let mut state = ProviderEventState::default();
        state.apply(&AgentEvent::RunStarted { run: run.clone() });
        state.apply(&AgentEvent::RunUsageUpdated {
            run: run.clone(),
            usage,
        });
        assert_eq!(
            state.usage_label().as_deref(),
            Some("1.2k in · 80 out · 9.5k ctx · $0.0123")
        );
        state.apply(&AgentEvent::RunFinished {
            run,
            outcome: RunOutcome {
                status: RunStatus::Done,
                stop_reason: None,
                error: None,
                failure_kind: None,
                usage: Some(usage),
                execution: None,
            },
        });
        assert!(!state.running);
        assert_eq!(state.current_run, None);
        assert_eq!(state.status, "complete");
    }

    #[test]
    fn error_is_terminal_and_preserves_the_first_failure_text() {
        let mut state = ProviderEventState::default();
        state.mark_starting();
        let effects = state.apply(&AgentEvent::Error {
            code: "provider".into(),
            message: "first failure".into(),
            run: Some(RunId::new("run-error")),
        });
        assert_eq!(
            effects,
            vec![TranscriptEffect::Push {
                kind: TranscriptKind::Error,
                text: "first failure".into(),
            }]
        );
        assert!(!state.running);
        assert_eq!(state.status, "failed");
    }

    #[test]
    fn steering_fallback_only_queues_on_explicit_unsupported() {
        assert_eq!(
            classify_steering_result(Ok(())),
            SteeringDisposition::Delivered
        );
        assert_eq!(
            classify_steering_result(Err(agent_core::Error::Unsupported("no live steer".into()))),
            SteeringDisposition::QueueFollowUp
        );
        assert_eq!(
            classify_steering_result(Err(agent_core::Error::Transport("offline".into()))),
            SteeringDisposition::RestoreInput("transport error: offline".into())
        );
    }

    #[test]
    fn specialist_projection_exposes_verified_cloud_continuity() {
        let mut state = ProviderEventState::default();
        let effects = state.apply(&AgentEvent::Trace {
            run: Some(RunId::new("run-specialist")),
            source: "clark_specialist_projection".into(),
            payload: serde_json::json!({
                "cloudSync": {
                    "scope_id": "specialist-session-7",
                    "file_count": 3,
                    "verified_segment_count": 5,
                    "total_bytes": 1200
                }
            }),
        });
        assert_eq!(state.status, "cloud synchronized");
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            TranscriptEffect::Push { kind: TranscriptKind::System, text }
                if text.contains("3 files") && text.contains("specialist-session-7")
        ));
    }

    #[test]
    fn specialist_projection_without_cloud_receipt_is_a_visible_error() {
        let mut state = ProviderEventState::default();
        let effects = state.apply(&AgentEvent::Trace {
            run: Some(RunId::new("run-specialist")),
            source: "clark_specialist_projection".into(),
            payload: serde_json::json!({"specialist": "scientist"}),
        });
        assert_eq!(state.status, "cloud receipt rejected");
        assert!(matches!(
            &effects[0],
            TranscriptEffect::Push { kind: TranscriptKind::Error, text }
                if text.contains("omitted")
        ));
    }
}
