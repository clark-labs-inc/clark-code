use std::path::Path;

use agent_core::domain::{AgentEvent, ContentBlock, RunStatus};
use agent_core::ids::SessionId;
use agent_core::provider::{PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_local::{LocalAgentProvider, SkillCatalogEntry};
use serde_json::{json, Value};

use crate::model::{error, require, DynError};
use crate::model_server;

#[derive(Clone)]
pub struct RemoteSpec {
    pub ws_url: String,
    pub token: String,
    pub cwd: String,
}

pub struct ActiveProvider {
    provider: LocalAgentProvider,
    session: SessionId,
}

pub struct ProviderTurn {
    pub request: Value,
    pub request_text: String,
    pub event_count: usize,
    pub final_text: String,
}

pub async fn launch_and_prompt(
    project: &Path,
    remote: Option<&RemoteSpec>,
    skill: &SkillCatalogEntry,
    expected_fragments: &[&str],
) -> Result<(ActiveProvider, ProviderTurn), DynError> {
    let (base_url, request_handle) = model_server::one_shot("SIMULATED_SKILL_ACK").await?;
    let mut extra = json!({
        "base_url": base_url,
        "model": "benchmark-scripted-model",
        "max_iterations": 2,
        "memories": false,
        "research": false,
        "auto_compact": false,
        // This benchmark talks only to its in-process scripted model and never
        // executes a model-authored tool call. Make that host boundary explicit
        // so a CI host that forbids bubblewrap user namespaces does not turn a
        // provider-wire contract test into a platform sandbox preflight.
        "sandbox_mode": "disabled",
        "orchestration": {"enabled": false}
    });
    if let Some(remote) = remote {
        extra["remote"] = json!({
            "ws_url": remote.ws_url,
            "token": remote.token,
            "cwd": remote.cwd
        });
    }

    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("benchmark-not-a-real-key".into()),
            extra,
            ..Default::default()
        })
        .await?;
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(project.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await?;
    let mut active = ActiveProvider {
        provider,
        session: session.id,
    };
    let (event_count, final_text) = run_turn(&mut active, skill).await?;
    let request = tokio::time::timeout(std::time::Duration::from_secs(5), request_handle)
        .await
        .map_err(|_| error("scripted model did not receive the provider request"))???;
    let request_text = model_server::all_message_text(&request);

    for fragment in expected_fragments {
        require(
            request_text.contains(fragment),
            format!("provider request omitted expected fragment `{fragment}`"),
        )?;
    }
    require(
        request_text.contains(&format!("<id>{}</id>", skill.id)),
        "provider request omitted exact skill id",
    )?;
    require(
        request_text.contains(&format!("<revision>{}</revision>", skill.revision)),
        "provider request omitted exact skill revision",
    )?;
    require(
        !request.to_string().contains("\"skill_reference\""),
        "typed UI control block leaked into the model wire request",
    )?;

    Ok((
        active,
        ProviderTurn {
            request,
            request_text,
            event_count,
            final_text,
        },
    ))
}

pub async fn expect_binding_rejected(
    active: &mut ActiveProvider,
    skill: &SkillCatalogEntry,
    expected: &str,
) -> Result<String, DynError> {
    let input = selected_prompt(skill);
    match active.provider.prompt(&active.session, input).await {
        Ok(_) => Err(error(
            "provider accepted a stale or removed skill binding before model dispatch",
        )),
        Err(cause) => {
            let message = cause.to_string();
            require(
                message.contains(expected),
                format!("unexpected binding rejection: {message}"),
            )?;
            Ok(message)
        }
    }
}

async fn run_turn(
    active: &mut ActiveProvider,
    skill: &SkillCatalogEntry,
) -> Result<(usize, String), DynError> {
    let mut stream = active
        .provider
        .prompt(&active.session, selected_prompt(skill))
        .await?;
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        let done = matches!(&event, AgentEvent::RunFinished { .. });
        events.push(event);
        if done {
            break;
        }
    }
    let outcome = events.iter().find_map(|event| match event {
        AgentEvent::RunFinished { outcome, .. } => Some(outcome),
        _ => None,
    });
    require(
        outcome.is_some_and(|outcome| outcome.status == RunStatus::Done),
        format!(
            "provider run did not finish successfully: status={:?}, error={:?}",
            outcome.map(|outcome| outcome.status),
            outcome.and_then(|outcome| outcome.error.as_deref())
        ),
    )?;
    let final_text = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::MessageChunk {
                delta: ContentBlock::Text { text },
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    require(
        final_text.contains("SIMULATED_SKILL_ACK"),
        "scripted provider response was not projected",
    )?;
    Ok((events.len(), final_text))
}

fn selected_prompt(skill: &SkillCatalogEntry) -> PromptInput {
    PromptInput {
        blocks: vec![
            ContentBlock::text("Use the selected brainstorming playbook for this request."),
            ContentBlock::skill_reference(
                skill.id.clone(),
                skill.revision.clone(),
                skill.invocation_name.clone(),
            ),
        ],
        attachments: Vec::new(),
    }
}
