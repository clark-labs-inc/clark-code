use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use agent_core::domain::{
    PermissionOption, PermissionRequest, RunOutcome, ToolCall, ToolKind, ToolStatus,
};
use agent_core::error::{Error, Result as CoreResult};
use agent_core::ids::{PermissionRequestId, ProviderId, SessionId, ToolCallId};
use agent_core::provider::{
    EventStream, ProviderCapabilities, Session, SessionEnvironment, SessionOptions,
};
use futures::stream;

use crate::contract::{AgentPath, OrchestrationId, ReadOnlyTask, ReportStatus, TaskId};

use super::*;

#[derive(Default)]
struct FakeState {
    rejected: bool,
}

struct FakeProvider {
    shared: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl Provider for FakeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new("fake")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    async fn connect(&mut self, _config: ProviderConfig) -> CoreResult<()> {
        Ok(())
    }

    async fn new_session(&mut self, _options: SessionOptions) -> CoreResult<Session> {
        Ok(Session {
            id: SessionId::new("session"),
            provider: self.id(),
            capabilities: self.capabilities(),
            mode: None,
            environment: Some(SessionEnvironment::default()),
        })
    }

    async fn load_session(&mut self, _id: SessionId) -> CoreResult<Session> {
        Err(Error::Unsupported("no".to_string()))
    }

    async fn prompt(
        &mut self,
        _session: &SessionId,
        _input: PromptInput,
    ) -> CoreResult<EventStream> {
        let run = RunId::new("run");
        let report = serde_json::json!({
            "task_id": "reader",
            "attempt": 1,
            "status": "reported",
            "summary": "found it",
            "changed_paths": [],
            "commands": [],
            "tests": [],
            "claims": [{"claim":"found it","evidence_ref":"src/lib.rs:1"}],
            "unresolved": []
        })
        .to_string();
        Ok(Box::pin(stream::iter(vec![
            AgentEvent::RunStarted { run: run.clone() },
            AgentEvent::ToolCall {
                run: run.clone(),
                call: ToolCall {
                    id: ToolCallId::new("tool"),
                    tool_name: Some("edit_file".to_string()),
                    title: "try write".to_string(),
                    kind: ToolKind::Edit,
                    status: ToolStatus::Pending,
                    locations: vec![],
                    content: vec![],
                    raw_input: None,
                },
            },
            AgentEvent::PermissionRequest {
                request: PermissionRequest {
                    id: PermissionRequestId::new("permission"),
                    session: SessionId::new("session"),
                    tool_call: None,
                    title: "write".to_string(),
                    options: vec![PermissionOption {
                        id: "reject".to_string(),
                        label: "Reject".to_string(),
                        kind: PermissionOptionKind::RejectAlways,
                    }],
                    detail: None,
                    risk: None,
                    reason: None,
                },
            },
            AgentEvent::MessageChunk {
                run: run.clone(),
                role: Role::Agent,
                delta: ContentBlock::text(report),
            },
            AgentEvent::RunFinished {
                run,
                outcome: RunOutcome {
                    status: RunStatus::Done,
                    stop_reason: None,
                    error: None,
                    failure_kind: None,
                    usage: Some(RunUsage {
                        input_tokens: 10,
                        output_tokens: 5,
                        cost_usd: Some(0.01),
                        ..Default::default()
                    }),
                    execution: None,
                },
            },
        ])))
    }

    async fn cancel(&mut self, _session: &SessionId, _run: &RunId) -> CoreResult<()> {
        Ok(())
    }

    async fn respond(&mut self, _session: &SessionId, _response: ClientResponse) -> CoreResult<()> {
        self.shared.lock().expect("fake state lock").rejected = true;
        Ok(())
    }
}

struct StaticGuard(&'static str);

#[async_trait]
impl WorkspaceGuard for StaticGuard {
    async fn snapshot(&self) -> Result<String, HarnessError> {
        Ok(self.0.to_string())
    }
}

#[derive(Default)]
struct ChangingGuard(AtomicUsize);

#[async_trait]
impl WorkspaceGuard for ChangingGuard {
    async fn snapshot(&self) -> Result<String, HarnessError> {
        Ok(self.0.fetch_add(1, Ordering::SeqCst).to_string())
    }
}

#[tokio::test]
async fn provider_harness_rejects_permissions_and_extracts_report() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let factory_state = state.clone();
    let harness = ProviderHarness::new(
        ProviderHarnessConfig {
            id: "local".to_string(),
            kind: HarnessKind::Local,
            provider: "fake".to_string(),
            model: "fake".to_string(),
            provider_config: ProviderConfig::default(),
            cwd: "/tmp".to_string(),
            timeout: Duration::from_secs(1),
            enforcement: ReadOnlyEnforcement::HostToolGate,
        },
        Arc::new(move || {
            Box::new(FakeProvider {
                shared: factory_state.clone(),
            }) as Box<dyn Provider>
        }),
        Arc::new(StaticGuard("same")),
    )
    .unwrap();
    let attempt = harness
        .run(
            AttemptContext {
                orchestration_id: OrchestrationId::new("fanout").unwrap(),
                agent_path: AgentPath::parse("/root/reader").unwrap(),
                task: ReadOnlyTask {
                    id: TaskId::new("reader").unwrap(),
                    role: AgentRole::Explorer,
                    objective: "inspect".to_string(),
                    scopes: BTreeSet::from(["src".to_string()]),
                    acceptance: vec!["cite".to_string()],
                    harness: "local".to_string(),
                },
                attempt: 1,
                parent_context: "overall".to_string(),
                feedback: None,
                cancel: tokio_util::sync::CancellationToken::new(),
            },
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    assert!(state.lock().expect("fake state lock").rejected);
    assert_eq!(attempt.report.unwrap().status, ReportStatus::Reported);
    assert_eq!(attempt.usage.cost_usd, 0.01);
    assert!(!attempt.observed_write);
}

#[tokio::test]
async fn provider_harness_discards_a_report_when_the_workspace_digest_changes() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let factory_state = state.clone();
    let harness = ProviderHarness::new(
        ProviderHarnessConfig {
            id: "local".to_string(),
            kind: HarnessKind::Local,
            provider: "fake".to_string(),
            model: "fake".to_string(),
            provider_config: ProviderConfig::default(),
            cwd: "/tmp".to_string(),
            timeout: Duration::from_secs(1),
            enforcement: ReadOnlyEnforcement::HostToolGate,
        },
        Arc::new(move || {
            Box::new(FakeProvider {
                shared: factory_state.clone(),
            }) as Box<dyn Provider>
        }),
        Arc::new(ChangingGuard::default()),
    )
    .unwrap();
    let attempt = harness
        .run(
            AttemptContext {
                orchestration_id: OrchestrationId::new("fanout").unwrap(),
                agent_path: AgentPath::parse("/root/reader").unwrap(),
                task: ReadOnlyTask {
                    id: TaskId::new("reader").unwrap(),
                    role: AgentRole::Explorer,
                    objective: "inspect".to_string(),
                    scopes: BTreeSet::from(["src".to_string()]),
                    acceptance: vec!["cite".to_string()],
                    harness: "local".to_string(),
                },
                attempt: 1,
                parent_context: "overall".to_string(),
                feedback: None,
                cancel: tokio_util::sync::CancellationToken::new(),
            },
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    assert!(attempt.observed_write);
    assert!(attempt.report.is_none());
}

#[test]
fn acp_requires_stronger_boundary_than_host_tool_gating() {
    let result = ProviderHarness::new(
        ProviderHarnessConfig {
            id: "acp".to_string(),
            kind: HarnessKind::Acp,
            provider: "acp".to_string(),
            model: "external".to_string(),
            provider_config: ProviderConfig::default(),
            cwd: "/tmp".to_string(),
            timeout: Duration::from_secs(1),
            enforcement: ReadOnlyEnforcement::HostToolGate,
        },
        Arc::new(|| panic!("must not create provider")),
        Arc::new(StaticGuard("same")),
    );
    assert!(result.is_err());
}
