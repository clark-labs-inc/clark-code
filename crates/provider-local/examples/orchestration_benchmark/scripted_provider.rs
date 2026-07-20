use std::path::{Path, PathBuf};

use agent_core::domain::{
    AgentEvent, ContentBlock, FsLocation, Role, RunOutcome, RunStatus, RunUsage, ToolCall,
    ToolCallPatch, ToolKind, ToolStatus,
};
use agent_core::error::{Error, Result};
use agent_core::ids::{ProviderId, RunId, SessionId, ToolCallId};
use agent_core::provider::{
    ClientResponse, EventStream, PromptInput, Provider, ProviderCapabilities, ProviderConfig,
    Session, SessionEnvironment, SessionOptions,
};
use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::lifecycle::ScriptedLifecycle;
use crate::model::{
    AgentStatus, AttemptRecord, ClaimEvidence, CommandEvidence, PermissionCeiling,
    StructuredHandoff, TaskContract, TaskStatus, TestEvidence,
};
use crate::scenarios::FileFixture;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ScriptedAction {
    Inspect {
        finding: String,
    },
    Apply {
        files: Vec<FileFixture>,
    },
    Review {
        accepted: bool,
        finding: Option<String>,
    },
    Verify,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptedFault {
    Crash,
    FalseHandoff,
    MissingHandoff,
    #[default]
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptedTaskEnvelope {
    pub task: TaskContract,
    pub attempt_id: String,
    pub baseline_checkpoint: Option<String>,
    pub action: ScriptedAction,
    #[serde(default)]
    pub fault: ScriptedFault,
}

#[derive(Clone, Debug)]
pub struct ScriptedProfile {
    pub provider: String,
    pub model: String,
    pub role: String,
    pub permission_ceiling: PermissionCeiling,
}

pub struct ScriptedProvider {
    profile: ScriptedProfile,
    cwd: Option<PathBuf>,
    session_id: Option<SessionId>,
    run_counter: u64,
}

impl ScriptedProvider {
    pub fn new(profile: ScriptedProfile) -> Self {
        Self {
            profile,
            cwd: None,
            session_id: None,
            run_counter: 0,
        }
    }

    fn session(&self) -> Result<&SessionId> {
        self.session_id.as_ref().ok_or(Error::NotConnected)
    }

    fn cwd(&self) -> Result<&Path> {
        self.cwd.as_deref().ok_or(Error::NotConnected)
    }

    fn execute(&self, envelope: &ScriptedTaskEnvelope) -> Result<(StructuredHandoff, Vec<String>)> {
        let cwd = self.cwd()?;
        let mut tool_calls = Vec::new();
        let mut changed_paths = std::collections::BTreeSet::new();
        let summary = match &envelope.action {
            ScriptedAction::Inspect { finding } => {
                tool_calls.push(
                    if self.profile.provider == "scripted-clark-cloud" {
                        "clark_research"
                    } else {
                        "read_file"
                    }
                    .into(),
                );
                format!("Inspection complete: {finding}")
            }
            ScriptedAction::Apply { files } => {
                for file in files {
                    let path = cwd.join(&file.path);
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(path, file.content.as_bytes())?;
                    changed_paths.insert(file.path.clone());
                    tool_calls.push("write_file".into());
                }
                format!("Applied {} expected file mutation(s)", files.len())
            }
            ScriptedAction::Review { accepted, finding } => {
                tool_calls.push("read_file".into());
                format!(
                    "Review {}{}",
                    if *accepted { "accepted" } else { "rejected" },
                    finding
                        .as_deref()
                        .map(|value| format!(": {value}"))
                        .unwrap_or_default()
                )
            }
            ScriptedAction::Verify => {
                tool_calls.push("check_diagnostics".into());
                "Final deterministic verification completed".into()
            }
        };
        if matches!(envelope.fault, ScriptedFault::FalseHandoff) {
            changed_paths.insert("fabricated/not-written.txt".into());
        }
        let handoff = StructuredHandoff {
            task_id: envelope.task.id.clone(),
            attempt_id: envelope.attempt_id.clone(),
            reported_status: TaskStatus::Reported,
            summary,
            changed_paths,
            baseline_checkpoint: envelope.baseline_checkpoint.clone(),
            result_checkpoint: None,
            commands: vec![CommandEvidence {
                command: "scripted-provider".into(),
                exit_code: Some(0),
                output_artifact: None,
            }],
            tests: vec![TestEvidence {
                name: "scripted contract".into(),
                passed: true,
                output_artifact: None,
            }],
            claims: vec![ClaimEvidence {
                claim: "scripted task reached its configured terminal action".into(),
                evidence_ref: format!("attempt:{}", envelope.attempt_id),
            }],
            unresolved: vec![],
            artifact_refs: vec![],
        };
        Ok((handoff, tool_calls))
    }

    fn usage(&self, action: &ScriptedAction) -> RunUsage {
        let cheap = self.profile.model.contains("cheap");
        let multiplier = match action {
            ScriptedAction::Inspect { .. } => 1,
            ScriptedAction::Apply { files } => files.len().max(1) as u64,
            ScriptedAction::Review { .. } | ScriptedAction::Verify => 1,
        };
        let input_tokens = if cheap { 90 } else { 220 } * multiplier;
        let output_tokens = if cheap { 30 } else { 70 } * multiplier;
        RunUsage {
            input_tokens,
            output_tokens,
            context_tokens: input_tokens,
            cost_usd: Some(if cheap { 0.0001 } else { 0.0005 } * multiplier as f64),
            context_limit: Some(100_000),
        }
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new(self.profile.provider.clone())
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            permissions: true,
            fs: true,
            terminal: true,
            load_session: false,
            modes: vec!["scripted".into()],
            collaboration_modes: Vec::new(),
        }
    }

    async fn connect(&mut self, _config: ProviderConfig) -> Result<()> {
        Ok(())
    }

    async fn new_session(&mut self, options: SessionOptions) -> Result<Session> {
        let cwd = options
            .cwd
            .map(PathBuf::from)
            .ok_or_else(|| Error::Unsupported("scripted provider requires cwd".into()))?;
        self.cwd = Some(cwd.clone());
        let id = SessionId::new(Uuid::new_v4().to_string());
        self.session_id = Some(id.clone());
        Ok(Session {
            id,
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: Some("scripted".into()),
            collaboration_mode: options.collaboration_mode.unwrap_or_default(),
            environment: Some(SessionEnvironment {
                checkout_root: Some(cwd.to_string_lossy().into_owned()),
                repository_root: Some(cwd.to_string_lossy().into_owned()),
                workspace_roots: vec![cwd.to_string_lossy().into_owned()],
                docs_root: None,
                remote: false,
            }),
        })
    }

    async fn load_session(&mut self, _id: SessionId) -> Result<Session> {
        Err(Error::Unsupported(
            "scripted provider does not load sessions".into(),
        ))
    }

    async fn prompt(&mut self, session: &SessionId, input: PromptInput) -> Result<EventStream> {
        if self.session()? != session {
            return Err(Error::SessionNotFound(session.to_string()));
        }
        let text = input
            .blocks
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let envelope: ScriptedTaskEnvelope = serde_json::from_str(&text)?;
        self.run_counter += 1;
        let run = RunId::new(format!("scripted-{}", self.run_counter));
        let usage = self.usage(&envelope.action);
        let mut events = vec![AgentEvent::RunStarted { run: run.clone() }];
        let lifecycle =
            ScriptedLifecycle::new(session, &run, &mut events).map_err(Error::Protocol)?;
        if matches!(envelope.fault, ScriptedFault::Crash) {
            events.push(AgentEvent::Error {
                code: "scripted_crash".into(),
                message: "injected worker crash".into(),
                run: Some(run.clone()),
            });
            let execution = lifecycle
                .finish(
                    &mut events,
                    RunStatus::Failed,
                    Some("injected worker crash"),
                )
                .map_err(Error::Protocol)?;
            events.push(AgentEvent::RunFinished {
                run,
                outcome: RunOutcome {
                    status: RunStatus::Failed,
                    stop_reason: Some("injected crash".into()),
                    error: Some("injected worker crash".into()),
                    failure_kind: None,
                    usage: Some(usage),
                    execution: Some(execution),
                },
            });
            return Ok(stream::iter(events).boxed());
        }

        let (handoff, tool_names) = self.execute(&envelope)?;
        for (index, name) in tool_names.iter().enumerate() {
            let id = ToolCallId::new(format!("{}-{index}", envelope.attempt_id));
            lifecycle
                .tool_started(&mut events, id.as_str(), name, name == "write_file")
                .map_err(Error::Protocol)?;
            events.push(AgentEvent::ToolCall {
                run: run.clone(),
                call: ToolCall {
                    id: id.clone(),
                    tool_name: Some(name.clone()),
                    title: format!("{name}: {}", envelope.task.id),
                    kind: if name == "write_file" {
                        ToolKind::Edit
                    } else {
                        ToolKind::Read
                    },
                    status: ToolStatus::InProgress,
                    locations: envelope
                        .task
                        .scope
                        .iter()
                        .map(|path| FsLocation {
                            path: path.clone(),
                            line: None,
                        })
                        .collect(),
                    content: vec![],
                    raw_input: Some(serde_json::json!({"task_id": envelope.task.id})),
                },
            });
            events.push(AgentEvent::ToolCallUpdate {
                run: run.clone(),
                id: id.clone(),
                patch: ToolCallPatch {
                    status: Some(ToolStatus::Completed),
                    ..Default::default()
                },
            });
            lifecycle
                .tool_finished(&mut events, id.as_str(), envelope.task.scope.clone())
                .map_err(Error::Protocol)?;
        }
        let final_message = if matches!(envelope.fault, ScriptedFault::MissingHandoff) {
            "Finished without a structured handoff".into()
        } else {
            serde_json::to_string(&handoff)?
        };
        events.push(AgentEvent::MessageChunk {
            run: run.clone(),
            role: Role::Agent,
            delta: ContentBlock::text(final_message),
        });
        let execution = lifecycle
            .finish_with_usage(&mut events, RunStatus::Done, usage)
            .map_err(Error::Protocol)?;
        events.push(AgentEvent::RunFinished {
            run,
            outcome: RunOutcome {
                status: RunStatus::Done,
                stop_reason: None,
                error: None,
                failure_kind: None,
                usage: Some(usage),
                execution: Some(execution),
            },
        });
        Ok(stream::iter(events).boxed())
    }

    async fn cancel(&mut self, _session: &SessionId, _run: &RunId) -> Result<()> {
        Ok(())
    }

    async fn respond(&mut self, _session: &SessionId, _response: ClientResponse) -> Result<()> {
        Ok(())
    }
}

pub fn attempt_from_events(
    profile: &ScriptedProfile,
    task: &TaskContract,
    attempt_id: &str,
    events: &[AgentEvent],
    duration_ms: u64,
) -> AttemptRecord {
    let mut status = AgentStatus::Running;
    let mut usage = RunUsage::default();
    let mut tool_calls = Vec::new();
    let mut execution = None;
    let mut final_message = String::new();
    let mut error = None;
    for event in events {
        match event {
            AgentEvent::ToolCall { call, .. } => {
                if let Some(name) = &call.tool_name {
                    tool_calls.push(name.clone());
                }
            }
            AgentEvent::MessageChunk {
                role: Role::Agent,
                delta: ContentBlock::Text { text },
                ..
            } => final_message.push_str(text),
            AgentEvent::Error { message, .. } => error = Some(message.clone()),
            AgentEvent::RunFinished { outcome, .. } => {
                usage = outcome.usage.unwrap_or_default();
                execution = outcome.execution.clone();
                status = match outcome.status {
                    RunStatus::Done => AgentStatus::Completed,
                    RunStatus::Cancelled => AgentStatus::Interrupted,
                    RunStatus::Failed => AgentStatus::Errored,
                    _ => AgentStatus::Idle,
                };
            }
            _ => {}
        }
    }
    let lifecycle_events = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Trace {
                source, payload, ..
            } if source == "execution_lifecycle" => serde_json::from_value(payload.clone()).ok(),
            _ => None,
        })
        .collect::<Vec<agent_orchestration::ExecutionEvent>>();
    let lifecycle_trace_replayable = !lifecycle_events.is_empty()
        && agent_orchestration::ExecutionLedger::replay(&lifecycle_events).is_ok();
    let mut terminal_tool_ids = std::collections::BTreeSet::new();
    let duplicate_tool_receipts = lifecycle_events
        .iter()
        .filter_map(|event| match &event.kind {
            agent_orchestration::ExecutionEventKind::ToolFinished { id, .. } => Some(id),
            _ => None,
        })
        .filter(|id| !terminal_tool_ids.insert((*id).clone()))
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let handoff = serde_json::from_str(&final_message).ok();
    AttemptRecord {
        attempt_id: attempt_id.into(),
        task_id: task.id.clone(),
        agent_path: task.logical_path.clone(),
        provider: profile.provider.clone(),
        model: profile.model.clone(),
        role: profile.role.clone(),
        permission_ceiling: profile.permission_ceiling,
        status,
        duration_ms,
        usage,
        execution,
        lifecycle_trace_replayable,
        duplicate_tool_receipts,
        tool_calls,
        final_message,
        handoff,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::provider::Provider;
    use futures::StreamExt;
    use std::collections::BTreeSet;

    fn task() -> TaskContract {
        TaskContract {
            id: "write".into(),
            logical_path: "/root/write".into(),
            mode: crate::model::TaskMode::Write,
            instruction: "write".into(),
            dependencies: vec![],
            scope: BTreeSet::from(["out.txt".into()]),
            acceptance: vec!["out.txt is correct".into()],
            permission_ceiling: PermissionCeiling::WorkspaceWrite,
            preferred_model_tier: "strong".into(),
        }
    }

    #[tokio::test]
    async fn scripted_provider_runs_through_normalized_provider_events() {
        let dir = tempfile::tempdir().unwrap();
        let profile = ScriptedProfile {
            provider: "scripted".into(),
            model: "scripted-strong".into(),
            role: "writer".into(),
            permission_ceiling: PermissionCeiling::WorkspaceWrite,
        };
        let mut provider = ScriptedProvider::new(profile.clone());
        provider.connect(ProviderConfig::default()).await.unwrap();
        let session = provider
            .new_session(SessionOptions {
                cwd: Some(dir.path().to_string_lossy().into_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        let envelope = ScriptedTaskEnvelope {
            task: task(),
            attempt_id: "attempt-1".into(),
            baseline_checkpoint: None,
            action: ScriptedAction::Apply {
                files: vec![FileFixture::new("out.txt", "done\n")],
            },
            fault: ScriptedFault::None,
        };
        let events: Vec<_> = provider
            .prompt(
                &session.id,
                PromptInput::text(serde_json::to_string(&envelope).unwrap()),
            )
            .await
            .unwrap()
            .collect()
            .await;
        let attempt = attempt_from_events(&profile, &task(), "attempt-1", &events, 1);
        assert_eq!(attempt.status, AgentStatus::Completed);
        assert!(attempt.handoff.is_some());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out.txt")).unwrap(),
            "done\n"
        );
    }
}
