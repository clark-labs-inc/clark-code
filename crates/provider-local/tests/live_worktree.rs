//! One paid, opt-in acceptance test against an actual model and an actual
//! temporary Git repository with linked worktrees. No model/provider default is
//! embedded here: the caller must explicitly supply all live configuration.

mod support;

use std::time::Duration;

use agent_core::domain::{AgentEvent, RunStatus};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use provider_local::LocalAgentProvider;
use serde_json::json;

use support::GitFixture;

struct LiveConfig {
    base_url: String,
    model: String,
    api_key: String,
}

fn live_config() -> Option<LiveConfig> {
    if std::env::var("CLARK_CODE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping: set CLARK_CODE_LIVE=1 after explicitly approving a provider/model");
        return None;
    }
    let required = |name: &str| -> String {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| panic!("{name} must be set when CLARK_CODE_LIVE=1"))
    };
    Some(LiveConfig {
        base_url: required("CLARK_CODE_BASE_URL"),
        model: required("CLARK_CODE_MODEL"),
        api_key: required("CLARK_CODE_API_KEY"),
    })
}

#[tokio::test]
#[ignore = "requires an explicitly approved live provider/model and incurs cost"]
async fn live_model_edits_only_the_selected_linked_worktree() {
    let Some(config) = live_config() else { return };
    let fixture = GitFixture::new();
    #[cfg(unix)]
    let helpers = fixture.install_hostile_helpers();
    std::fs::write(
        fixture.detached.join("AGENTS.md"),
        "Use apply_patch for requested file changes. Never change another linked worktree.\n",
    )
    .unwrap();

    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some(config.api_key),
            extra: json!({
                "base_url": config.base_url,
                "model": config.model,
                "memories": false,
                "max_iterations": 20
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(fixture.detached.to_string_lossy().into_owned()),
            mode: None,
            resume: None,
        })
        .await
        .unwrap();

    let mut stream = provider
        .prompt(
            &session.id,
            PromptInput::text(
                "Read tracked.txt. Use apply_patch to replace its contents with exactly `live worktree edit\\n`. Then run `git status --short` and report what changed.",
            ),
        )
        .await
        .unwrap();
    let collect = async {
        let mut tools = Vec::new();
        let mut checkpoint = false;
        let mut status = None;
        while let Some(event) = stream.next().await {
            match event {
                AgentEvent::ToolCall { call, .. } => {
                    let name = call.tool_name.unwrap_or(call.title);
                    eprintln!("live tool call: {name} input={:?}", call.raw_input);
                    tools.push(name);
                }
                AgentEvent::ToolCallUpdate { id, patch, .. } => {
                    if patch.status.is_some() || patch.replace_content.is_some() {
                        eprintln!(
                            "live tool result: id={id:?} status={:?} content={:?}",
                            patch.status, patch.replace_content
                        );
                    }
                }
                AgentEvent::PermissionRequest { request } => {
                    provider
                        .respond(
                            &session.id,
                            ClientResponse::Permission {
                                request: request.id,
                                option: "allow_once".into(),
                                feedback: None,
                            },
                        )
                        .await
                        .unwrap();
                }
                AgentEvent::Checkpoint { .. } => checkpoint = true,
                AgentEvent::RunFinished { outcome, .. } => {
                    status = Some(outcome.status);
                    break;
                }
                _ => {}
            }
        }
        (tools, checkpoint, status)
    };
    let (tools, checkpoint, status) = tokio::time::timeout(Duration::from_secs(180), collect)
        .await
        .expect("live worktree turn timed out");

    eprintln!(
        "live worktree receipt: model={} status={status:?} checkpoint={checkpoint} tools={tools:?}",
        config.model
    );

    assert_eq!(status, Some(RunStatus::Done), "tools: {tools:?}");
    assert!(checkpoint, "run did not create a checkpoint");
    assert!(tools.iter().any(|tool| tool == "read_file"), "{tools:?}");
    assert!(tools.iter().any(|tool| tool == "apply_patch"), "{tools:?}");
    assert!(tools.iter().any(|tool| tool == "bash"), "{tools:?}");
    assert_eq!(
        std::fs::read_to_string(fixture.detached.join("tracked.txt")).unwrap(),
        "live worktree edit\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.main.join("tracked.txt")).unwrap(),
        "main\n"
    );
    #[cfg(unix)]
    {
        assert!(
            !helpers.fsmonitor_marker.exists(),
            "model-issued Git executed the configured fsmonitor helper"
        );
        assert!(
            !helpers.credential_marker.exists(),
            "live workflow executed the configured credential helper"
        );
    }
}
