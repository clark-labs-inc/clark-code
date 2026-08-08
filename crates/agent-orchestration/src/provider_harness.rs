use std::sync::Arc;
use std::time::Duration;

use agent_core::domain::{
    AgentEvent, ContentBlock, PermissionOptionKind, Role, RunStatus, RunUsage,
};
use agent_core::ids::RunId;
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::budget::UsageCharge;
use crate::contract::{AgentRole, HarnessKind, StructuredReport};
use crate::harness::{
    AttemptContext, HarnessAttempt, HarnessError, HarnessEvent, HarnessEventSink, ReadOnlyHarness,
};

pub trait ProviderFactory: Send + Sync {
    fn create(&self) -> Box<dyn Provider>;
}

impl<F> ProviderFactory for F
where
    F: Fn() -> Box<dyn Provider> + Send + Sync,
{
    fn create(&self) -> Box<dyn Provider> {
        self()
    }
}

#[async_trait]
pub trait WorkspaceGuard: Send + Sync {
    /// A stable digest of all workspace state the delegated provider could alter.
    async fn snapshot(&self) -> Result<String, HarnessError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOnlyEnforcement {
    /// Agent Desktop's local host refuses every mutating tool before invocation.
    HostToolGate,
    /// The child process is wrapped in an OS-enforced read-only sandbox.
    OsSandbox,
    /// The child receives a disposable checkout whose output is never merged.
    DisposableCheckout,
}

#[derive(Clone, Debug)]
pub struct ProviderHarnessConfig {
    pub id: String,
    pub kind: HarnessKind,
    pub provider: String,
    pub model: String,
    pub provider_config: ProviderConfig,
    pub cwd: String,
    pub timeout: Duration,
    pub enforcement: ReadOnlyEnforcement,
}

pub struct ProviderHarness {
    config: ProviderHarnessConfig,
    factory: Arc<dyn ProviderFactory>,
    workspace: Arc<dyn WorkspaceGuard>,
}

impl ProviderHarness {
    pub fn new(
        config: ProviderHarnessConfig,
        factory: Arc<dyn ProviderFactory>,
        workspace: Arc<dyn WorkspaceGuard>,
    ) -> Result<Self, String> {
        if config.id.trim().is_empty() {
            return Err("provider harness id must not be empty".to_string());
        }
        if config.cwd.trim().is_empty() {
            return Err("provider harness cwd must not be empty".to_string());
        }
        if config.timeout.is_zero() {
            return Err("provider harness timeout must be greater than zero".to_string());
        }
        if config.kind == HarnessKind::Acp
            && config.enforcement == ReadOnlyEnforcement::HostToolGate
        {
            return Err(
                "ACP harnesses require an OS sandbox or disposable checkout; Agent Desktop cannot prove an external process obeys its host tool gate"
                    .to_string(),
            );
        }
        if config.kind == HarnessKind::BrokeredCloud {
            return Err(
                "product cloud research uses a dedicated product tool boundary, not a coding provider harness"
                    .to_string(),
            );
        }
        Ok(Self {
            config,
            factory,
            workspace,
        })
    }
}

#[async_trait]
impl ReadOnlyHarness for ProviderHarness {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> HarnessKind {
        self.config.kind
    }

    async fn run(
        &self,
        context: AttemptContext,
        events: HarnessEventSink,
    ) -> Result<HarnessAttempt, HarnessError> {
        if context.cancel.is_cancelled() {
            return Err(HarnessError::Cancelled);
        }
        let before = self.workspace.snapshot().await?;
        let mut provider = self.factory.create();
        let collected = self.run_provider(provider.as_mut(), &context, events).await;
        let after = self.workspace.snapshot().await?;
        if before != after {
            return Ok(HarnessAttempt {
                provider: self.config.provider.clone(),
                model: self.config.model.clone(),
                report: None,
                final_message: collected
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default(),
                usage: UsageCharge::default(),
                observed_write: true,
            });
        }
        let collected = collected?;
        Ok(HarnessAttempt {
            provider: self.config.provider.clone(),
            model: self.config.model.clone(),
            report: extract_report(&collected.final_message),
            final_message: collected.final_message,
            usage: usage_charge(collected.usage),
            observed_write: false,
        })
    }
}

impl ProviderHarness {
    async fn run_provider(
        &self,
        provider: &mut dyn Provider,
        context: &AttemptContext,
        events: HarnessEventSink,
    ) -> Result<Collected, HarnessError> {
        provider
            .connect(self.config.provider_config.clone())
            .await
            .map_err(|error| HarnessError::Failed(error.to_string()))?;
        let session = provider
            .new_session(SessionOptions {
                cwd: Some(self.config.cwd.clone()),
                mode: Some("auto".to_string()),
                ..Default::default()
            })
            .await
            .map_err(|error| HarnessError::Failed(error.to_string()))?;
        let stream = match provider
            .prompt(&session.id, PromptInput::text(structured_prompt(context)))
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                let _ = provider.close_session(&session.id).await;
                return Err(HarnessError::Failed(error.to_string()));
            }
        };
        let collected = tokio::time::timeout(
            self.config.timeout,
            collect_events(
                provider,
                &session.id,
                stream,
                context.cancel.clone(),
                events,
            ),
        )
        .await;
        let collected = match collected {
            Ok(result) => result,
            Err(_) => {
                let _ = provider
                    .cancel(&session.id, &RunId::new("orchestration-timeout"))
                    .await;
                Err(HarnessError::TimedOut(self.config.id.clone()))
            }
        };
        let _ = provider.close_session(&session.id).await;
        collected
    }
}

struct Collected {
    final_message: String,
    usage: RunUsage,
}

async fn collect_events(
    provider: &mut dyn Provider,
    session_id: &agent_core::ids::SessionId,
    mut stream: agent_core::provider::EventStream,
    cancel: tokio_util::sync::CancellationToken,
    events: HarnessEventSink,
) -> Result<Collected, HarnessError> {
    let mut final_message = String::new();
    let mut usage = RunUsage::default();
    let mut failure = None;
    while let Some(event) = stream.next().await {
        if cancel.is_cancelled() {
            return Err(HarnessError::Cancelled);
        }
        match event {
            AgentEvent::MessageChunk {
                role: Role::Agent,
                delta: ContentBlock::Text { text },
                ..
            } => final_message.push_str(&text),
            AgentEvent::ToolCall { call, .. } => events(HarnessEvent::Tool {
                name: call.tool_name.unwrap_or_else(|| "tool".to_string()),
                summary: call.title,
            }),
            AgentEvent::PermissionRequest { request } => {
                let option = request
                    .options
                    .iter()
                    .find(|option| option.kind == PermissionOptionKind::RejectAlways)
                    .or_else(|| {
                        request
                            .options
                            .iter()
                            .find(|option| option.kind == PermissionOptionKind::RejectOnce)
                    })
                    .ok_or_else(|| {
                        HarnessError::Rejected(
                            "permission request had no fail-closed rejection option".to_string(),
                        )
                    })?;
                provider
                    .respond(
                        session_id,
                        ClientResponse::Permission {
                            request: request.id,
                            option: option.id.clone(),
                            feedback: Some(
                                "delegated Agent Desktop agents are structurally read-only"
                                    .to_string(),
                            ),
                        },
                    )
                    .await
                    .map_err(|error| HarnessError::Failed(error.to_string()))?;
                events(HarnessEvent::Warning {
                    message: format!("rejected mutating request: {}", request.title),
                });
            }
            AgentEvent::RunFinished { outcome, .. } => {
                if let Some(run_usage) = outcome.usage {
                    usage = run_usage;
                }
                if outcome.status != RunStatus::Done {
                    failure = Some(
                        outcome
                            .error
                            .or(outcome.stop_reason)
                            .unwrap_or_else(|| format!("run ended with {:?}", outcome.status)),
                    );
                }
            }
            AgentEvent::Error { message, .. } => failure = Some(message),
            _ => {}
        }
    }
    if let Some(error) = failure {
        return Err(HarnessError::Failed(error));
    }
    Ok(Collected {
        final_message,
        usage,
    })
}

fn usage_charge(usage: RunUsage) -> UsageCharge {
    UsageCharge {
        input_tokens: usage.input_tokens,
        cached_input_tokens: 0,
        output_tokens: usage.output_tokens,
        cost_usd: usage.cost_usd.unwrap_or(0.0),
    }
}

fn structured_prompt(context: &AttemptContext) -> String {
    let role = match context.task.role {
        AgentRole::Explorer => "repository explorer",
        AgentRole::Reviewer => "independent code reviewer",
        AgentRole::Verifier => "independent verifier",
        AgentRole::ExternalResearcher => "external research specialist",
    };
    let feedback = context
        .feedback
        .as_ref()
        .map(|feedback| format!("\nRework feedback:\n{feedback}\n"))
        .unwrap_or_default();
    format!(
        "You are a bounded, read-only {role}. You cannot edit files, run mutating commands, or widen permissions.\n\
         Parent objective:\n{}\n\n\
         Your task:\n{}\n\
         Allowed repository scopes: {:?}\n\
         Acceptance criteria: {:?}\n\
         {feedback}\
         Inspect evidence only inside the allowed scopes. Do not duplicate unrelated work.\n\
         Finish with exactly one JSON object and no prose after it, using this shape:\n\
         {{\"task_id\":\"{}\",\"attempt\":{},\"status\":\"reported\",\"changed_paths\":[],\"commands\":[],\"tests\":[],\"claims\":[{{\"evidence_ref\":\"path:line or command\",\"claim\":\"finding\"}}],\"unresolved\":[],\"summary\":\"concise result\"}}\n\
         Never claim evidence, commands, or tests you did not actually inspect or run.",
        context.parent_context,
        context.task.objective,
        context.task.scopes,
        context.task.acceptance,
        context.task.id,
        context.attempt,
    )
}

fn extract_report(text: &str) -> Option<StructuredReport> {
    let trimmed = text.trim();
    if let Ok(report) = serde_json::from_str(trimmed) {
        return Some(report);
    }
    for fenced in trimmed.split("```").skip(1).step_by(2) {
        let candidate = fenced.strip_prefix("json").unwrap_or(fenced).trim();
        if let Ok(report) = serde_json::from_str(candidate) {
            return Some(report);
        }
    }
    let end = trimmed.rfind('}')?;
    trimmed[..=end]
        .char_indices()
        .rev()
        .filter(|(_, character)| *character == '{')
        .find_map(|(start, _)| serde_json::from_str(&trimmed[start..=end]).ok())
}

#[cfg(test)]
#[path = "provider_harness_tests.rs"]
mod tests;
