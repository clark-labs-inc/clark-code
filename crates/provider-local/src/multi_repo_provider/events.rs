use std::path::Path;
use std::time::Duration;

use agent_core::domain::{
    AgentEvent, ContentBlock, PermissionOptionKind, Role, RunStatus, RunUsage,
};
use agent_core::ids::RunId;
use agent_core::provider::{ClientResponse, PromptInput, ProviderConfig, SessionOptions};
use agent_orchestration::{ProviderFactory, UsageCharge};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

pub(super) struct CollectedProviderRun {
    pub(super) final_message: String,
    pub(super) usage: UsageCharge,
}

pub(super) struct ProviderRunFailure {
    pub(super) message: String,
    pub(super) usage: UsageCharge,
}

pub(super) async fn run_provider(
    factory: &dyn ProviderFactory,
    config: ProviderConfig,
    cwd: &Path,
    input: PromptInput,
    response_timeout: Option<Duration>,
    cancel: CancellationToken,
) -> Result<CollectedProviderRun, ProviderRunFailure> {
    if cancel.is_cancelled() {
        return Err(failure(
            "provider run cancelled before start",
            RunUsage::default(),
        ));
    }
    let mut provider = factory.create();
    provider
        .connect(config)
        .await
        .map_err(|error| failure(error.to_string(), RunUsage::default()))?;
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(cwd.to_string_lossy().into_owned()),
            mode: Some("auto".into()),
            ..Default::default()
        })
        .await
        .map_err(|error| failure(error.to_string(), RunUsage::default()))?;
    let stream = match provider.prompt(&session.id, input).await {
        Ok(stream) => stream,
        Err(error) => {
            let _ = provider.close_session(&session.id).await;
            return Err(failure(error.to_string(), RunUsage::default()));
        }
    };
    let collect = collect_events(provider.as_mut(), &session.id, stream, cancel);
    let collected = match response_timeout {
        Some(timeout) => match tokio::time::timeout(timeout, collect).await {
            Ok(result) => result,
            Err(_) => {
                let _ = provider
                    .cancel(&session.id, &RunId::new("multi-repo-timeout"))
                    .await;
                Err(failure(
                    "isolated provider run timed out",
                    RunUsage::default(),
                ))
            }
        },
        None => collect.await,
    };
    let _ = provider.close_session(&session.id).await;
    collected
}

async fn collect_events(
    provider: &mut dyn agent_core::provider::Provider,
    session_id: &agent_core::ids::SessionId,
    mut stream: agent_core::provider::EventStream,
    cancel: CancellationToken,
) -> Result<CollectedProviderRun, ProviderRunFailure> {
    let mut final_message = String::new();
    let mut usage = RunUsage::default();
    let mut terminal = false;
    let mut error = None;
    loop {
        let event = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(failure("isolated provider run was cancelled", usage));
            }
            event = stream.next() => event,
        };
        let Some(event) = event else {
            break;
        };
        match event {
            AgentEvent::MessageChunk {
                role: Role::Agent,
                delta: ContentBlock::Text { text },
                ..
            } => final_message.push_str(&text),
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
                        failure(
                            "unexpected permission request had no fail-closed rejection",
                            usage,
                        )
                    })?;
                provider
                    .respond(
                        session_id,
                        ClientResponse::Permission {
                            request: request.id,
                            option: option.id.clone(),
                            feedback: Some(
                                "isolated orchestration does not widen permissions".into(),
                            ),
                        },
                    )
                    .await
                    .map_err(|response_error| failure(response_error.to_string(), usage))?;
            }
            AgentEvent::RunFinished { outcome, .. } => {
                terminal = true;
                if let Some(run_usage) = outcome.usage {
                    usage = run_usage;
                }
                if outcome.status != RunStatus::Done {
                    error = Some(
                        outcome
                            .error
                            .or(outcome.stop_reason)
                            .unwrap_or_else(|| format!("run ended with {:?}", outcome.status)),
                    );
                }
            }
            AgentEvent::Error { message, .. } => error = Some(message),
            _ => {}
        }
    }
    if !terminal {
        return Err(failure(
            "provider stream ended without a terminal run receipt",
            usage,
        ));
    }
    if let Some(message) = error {
        return Err(failure(message, usage));
    }
    Ok(CollectedProviderRun {
        final_message,
        usage: usage_charge(usage),
    })
}

fn failure(message: impl Into<String>, usage: RunUsage) -> ProviderRunFailure {
    ProviderRunFailure {
        message: message.into(),
        usage: usage_charge(usage),
    }
}

fn usage_charge(usage: RunUsage) -> UsageCharge {
    UsageCharge {
        input_tokens: usage.input_tokens,
        cached_input_tokens: 0,
        output_tokens: usage.output_tokens,
        cost_usd: usage.cost_usd.unwrap_or(0.0),
    }
}
