use std::path::Path;

use agent_core::{AgentEvent, PromptInput, Role, RunId, SessionId, Snapshot};
use conversation_cloud::{
    ConversationClient, ConversationDetail, ConversationSummary, ConversationWrite,
    CredentialSurface, SpecialistContext,
};
use uuid::Uuid;

use crate::cloud::RuntimeScope;
use crate::runtime::Workspace;

pub async fn probe(api_key: &str) -> Result<usize, String> {
    let client = ConversationClient::new(
        &crate::auth::platform_api_origin()?,
        api_key,
        CredentialSurface::CliApiKey,
        concat!("clark-cli/", env!("CARGO_PKG_VERSION")),
    )
    .map_err(|error| error.to_string())?;
    client
        .list()
        .await
        .map(|rows| rows.len())
        .map_err(|error| error.to_string())
}

pub struct ConversationCloud {
    client: ConversationClient,
    workspace: Workspace,
    project: String,
    specialist_context: Option<SpecialistContext>,
}

pub struct ActiveConversation {
    client: ConversationClient,
    pub id: String,
    pub snapshot: Snapshot,
    title: String,
    provider: String,
    project: String,
    mode: String,
    specialist_context: Option<SpecialistContext>,
    rev: i64,
    title_locked: bool,
}

impl ConversationCloud {
    pub fn connect(
        api_key: &str,
        workspace: Workspace,
        cwd: &Path,
        scope: &RuntimeScope,
    ) -> Result<Self, String> {
        scope
            .owner_scope
            .as_deref()
            .filter(|owner| !owner.trim().is_empty())
            .ok_or_else(|| {
                "Clark Cloud did not bind this credential to an account; conversation synchronization is unavailable"
                    .to_string()
            })?;
        let client = ConversationClient::new(
            &crate::auth::platform_api_origin()?,
            api_key,
            CredentialSurface::CliApiKey,
            concat!("clark-cli/", env!("CARGO_PKG_VERSION")),
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            workspace,
            project: cwd.to_string_lossy().into_owned(),
            specialist_context: specialist_context(workspace, scope)?,
        })
    }

    /// The list request is also the mandatory pre-execution cloud probe. A
    /// headless run never silently falls back to local-only history.
    pub async fn list(&self) -> Result<Vec<ConversationSummary>, String> {
        self.client.list().await.map_err(|error| {
            format!("Clark Cloud conversation synchronization is required but unavailable: {error}")
        })
    }

    pub fn choices(&self, conversations: Vec<ConversationSummary>) -> Vec<ConversationSummary> {
        conversations
            .into_iter()
            .filter(|summary| !summary.archived && self.matches_workspace(summary))
            .collect()
    }

    pub async fn open(&self, id: Option<&str>) -> Result<ActiveConversation, String> {
        match id {
            Some(id) => {
                let detail = self.client.get(id).await.map_err(|error| {
                    format!("Clark could not restore cloud conversation {id}: {error}")
                })?;
                if !self.matches_workspace(&detail.summary) {
                    return Err(format!(
                        "Clark conversation {id} belongs to a different specialist or workflow; reopen it from the matching Clark workspace"
                    ));
                }
                Ok(ActiveConversation::from_detail(
                    self.client.clone(),
                    detail,
                    self.project.clone(),
                    cloud_mode(self.workspace).into(),
                    self.specialist_context.clone(),
                ))
            }
            None => Ok(ActiveConversation {
                client: self.client.clone(),
                id: Uuid::new_v4().to_string(),
                snapshot: Snapshot::new(),
                title: "New conversation".into(),
                provider: cloud_provider(self.workspace).into(),
                project: self.project.clone(),
                mode: cloud_mode(self.workspace).into(),
                specialist_context: self.specialist_context.clone(),
                rev: 0,
                title_locked: false,
            }),
        }
    }

    fn matches_workspace(&self, summary: &ConversationSummary) -> bool {
        match (&self.specialist_context, &summary.specialist_context) {
            (Some(expected), Some(actual)) => {
                expected.kind == actual.kind
                    && summary
                        .mode
                        .as_deref()
                        .is_none_or(|mode| mode == cloud_mode(self.workspace))
            }
            (Some(_), None) => false,
            (None, Some(_)) => false,
            (None, None) => summary.provider != "specialist",
        }
    }
}

impl ActiveConversation {
    fn from_detail(
        client: ConversationClient,
        detail: ConversationDetail,
        current_project: String,
        current_mode: String,
        current_specialist_context: Option<SpecialistContext>,
    ) -> Self {
        let summary = detail.summary;
        Self {
            client,
            id: summary.id,
            snapshot: detail.snapshot,
            title: summary.title,
            provider: summary.provider,
            project: summary.project.unwrap_or(current_project),
            mode: summary.mode.unwrap_or(current_mode),
            specialist_context: summary.specialist_context.or(current_specialist_context),
            rev: summary.rev,
            title_locked: summary.title_locked,
        }
    }

    pub fn bind_session(&mut self, session: &SessionId) {
        self.snapshot.session = Some(session.clone());
    }

    pub async fn begin_turn(&mut self, prompt: &PromptInput) -> Result<(), String> {
        for block in prompt
            .blocks
            .iter()
            .cloned()
            .chain(prompt.attachments.iter().map(|upload| upload.echo_block()))
        {
            agent_core::apply(
                &mut self.snapshot,
                &AgentEvent::MessageChunk {
                    run: RunId::new("user"),
                    role: Role::User,
                    delta: block,
                },
            );
        }
        self.persist("running").await.map(|_| ())
    }

    pub async fn publish_ready(&mut self) -> Result<(), String> {
        self.persist("idle").await.map(|_| ())
    }

    pub async fn apply(&mut self, event: &AgentEvent) -> Result<(), String> {
        agent_core::apply(&mut self.snapshot, event);
        if checkpoint_event(event) {
            self.persist("running").await?;
        }
        Ok(())
    }

    pub async fn finish_turn(&mut self) -> Result<String, String> {
        self.persist("idle").await?;
        Ok(format!(
            "Conversation synchronized to Clark Cloud ({} · rev {}).",
            self.id, self.rev
        ))
    }

    async fn persist(&mut self, status: &str) -> Result<(), String> {
        if !self.title_locked && self.snapshot.has_conversation_content() {
            self.title = self.snapshot.derived_title();
        }
        let next_rev = self.rev.saturating_add(1);
        let write = ConversationWrite {
            id: self.id.clone(),
            title: self.title.clone(),
            provider: self.provider.clone(),
            project: Some(self.project.clone()),
            repository_fingerprint: None,
            remote_host: None,
            mode: Some(self.mode.clone()),
            title_locked: self.title_locked,
            specialist_context: self.specialist_context.clone(),
            rev: next_rev,
            snapshot: self.snapshot.clone(),
            status: Some(status.into()),
            base_rev: Some(self.rev),
            mutation_id: Some(Uuid::new_v4()),
        };
        let summary = self.client.put(&write).await.map_err(|error| {
            let action = if error.status() == Some(409) {
                "another Clark surface advanced this conversation"
            } else {
                "Clark Cloud did not acknowledge the snapshot"
            };
            format!(
                "Clark stopped because mandatory conversation synchronization failed: {action}: {error}"
            )
        })?;
        if summary.rev < next_rev {
            return Err(format!(
                "Clark stopped because cloud revision {} did not acknowledge local revision {next_rev}",
                summary.rev
            ));
        }
        self.rev = summary.rev;
        self.title = summary.title;
        self.title_locked = summary.title_locked;
        Ok(())
    }
}

fn checkpoint_event(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::Checkpoint { .. }
            | AgentEvent::ToolCall { .. }
            | AgentEvent::ToolCallUpdate { .. }
            | AgentEvent::ExecutionChecklistUpdated { .. }
            | AgentEvent::ProposedPlanUpdated { .. }
            | AgentEvent::GoalUpdated { .. }
            | AgentEvent::SpecialistPresentation { .. }
            | AgentEvent::Artifact { .. }
            | AgentEvent::RunFinished { .. }
            | AgentEvent::Error { .. }
    ) || matches!(
        event,
        AgentEvent::Trace { source, .. } if source == "clark_specialist_projection"
    )
}

fn cloud_provider(workspace: Workspace) -> &'static str {
    match workspace {
        Workspace::ScientistDiscover
        | Workspace::ScientistReplicate
        | Workspace::RsiResearch
        | Workspace::RsiCreateEvals
        | Workspace::RsiBuildWorld
        | Workspace::RsiStressTest
        | Workspace::RsiRegression => "specialist",
        _ => "local",
    }
}

pub fn cloud_mode(workspace: Workspace) -> &'static str {
    match workspace {
        Workspace::Code => "code",
        Workspace::Scout => "scout:scout",
        Workspace::SecurityScan => "security:security-scan",
        Workspace::SecurityDiff => "security:security-diff",
        Workspace::SecurityDeep => "security:security-deep",
        Workspace::ScientistDiscover => "scientist:discover",
        Workspace::ScientistReplicate => "scientist:replicate",
        Workspace::RsiResearch => "rsi:research",
        Workspace::RsiCreateEvals => "rsi:create-evals",
        Workspace::RsiBuildWorld => "rsi:build-world",
        Workspace::RsiStressTest => "rsi:stress-test",
        Workspace::RsiRegression => "rsi:regression",
    }
}

fn specialist_context(
    workspace: Workspace,
    scope: &RuntimeScope,
) -> Result<Option<SpecialistContext>, String> {
    let Some(kind) = workspace.paid_specialist_kind() else {
        return Ok(None);
    };
    let organization_id = scope.organization_id.clone().ok_or_else(|| {
        format!(
            "Clark {} organization binding is missing",
            workspace.label()
        )
    })?;
    Ok(Some(SpecialistContext {
        kind: kind.into(),
        organization_id: Some(organization_id),
        workspace_id: scope.workspace_id.clone(),
        repository_id: scope
            .security
            .as_ref()
            .map(|security| security.repository_id.clone()),
        workflow: Some(cloud_mode(workspace).into()),
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_specialist_has_a_distinct_cloud_workflow() {
        assert_eq!(cloud_mode(Workspace::Scout), "scout:scout");
        assert_eq!(
            cloud_mode(Workspace::SecurityDeep),
            "security:security-deep"
        );
        assert_eq!(
            cloud_mode(Workspace::ScientistReplicate),
            "scientist:replicate"
        );
        assert_eq!(cloud_mode(Workspace::RsiBuildWorld), "rsi:build-world");
    }

    #[test]
    fn artifacts_and_specialist_receipts_are_immediate_cloud_boundaries() {
        let artifact = AgentEvent::Artifact {
            run: agent_core::RunId::new("run-1"),
            artifact: agent_core::Artifact {
                id: "artifact-1".into(),
                title: "Experiment journal".into(),
                kind: agent_core::ArtifactKind::File,
                mime_type: Some("application/json".into()),
                uri: Some("clark://science/artifact-1".into()),
                tool_call: None,
            },
        };
        assert!(checkpoint_event(&artifact));
        assert!(checkpoint_event(&AgentEvent::Trace {
            run: None,
            source: "clark_specialist_projection".into(),
            payload: serde_json::json!({"cloudReceipt": "accepted"}),
        }));
    }
}
