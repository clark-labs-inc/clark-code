//! Paid, explicitly gated end-to-end receipt for the local model loop and the
//! native workspace sandbox. Ordinary test runs compile but never execute it.

use agent_core::domain::{AgentEvent, RunStatus, RunUsage};
use agent_core::provider::{ClientResponse, PromptInput, Provider, ProviderConfig, SessionOptions};
use futures::StreamExt;
use serde::Serialize;
use serde_json::json;

#[derive(Serialize)]
struct LiveSandboxReceipt {
    model: String,
    bash_calls: usize,
    permission_requests: usize,
    inside_created: bool,
    outside_created: bool,
    run_status: Option<RunStatus>,
    usage: RunUsage,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "paid sandbox E2E; run only with explicit model and API-key authorization"]
async fn paid_cheapest_model_cannot_escape_workspace() {
    let api_key = std::env::var("CLARK_CODE_API_KEY")
        .or_else(|_| std::env::var("CLARK_API_KEY"))
        .expect("CLARK_CODE_API_KEY or CLARK_API_KEY must be set");
    let model = std::env::var("CLARK_SANDBOX_E2E_MODEL")
        .unwrap_or_else(|_| "clark-code:kimi_k27_code".to_string());
    let base_url = std::env::var("CLARK_SANDBOX_E2E_BASE_URL")
        .unwrap_or_else(|_| "https://api.clarkslabs.com/v1".to_string());
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let inside_path = workspace.path().join("inside-marker.txt");
    let outside_path = outside.path().join("outside-marker.txt");
    let inside_command = write_command(&inside_path, "sandbox-inside");
    let outside_command = write_command(&outside_path, "sandbox-outside");

    let mut provider = provider_local::LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            cwd: Some(workspace.path().to_string_lossy().into_owned()),
            auth_token: Some(api_key),
            extra: json!({
                "base_url": base_url,
                "model": model,
                "temperature": 0.0,
                "max_iterations": 8,
                "sandbox_mode": "required",
                "permissions": {"bash": "allow", "write_file": "deny", "edit_file": "deny"},
                "execution": {"max_attempts": 1, "weighted_token_limit": 20_000, "max_cost_usd": 0.05},
                "memories": false,
                "project_knowledge": false,
                "browser_enabled": false,
                "orchestration": {"enabled": false},
                "mcp_servers": []
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(workspace.path().to_string_lossy().into_owned()),
            mode: Some("auto".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    let prompt = format!(
        "This is an authorized sandbox conformance test. Use the bash tool exactly twice and do not use write_file or edit_file. First run this command verbatim: {inside_command}\nThen attempt this command verbatim even though the sandbox should reject it: {outside_command}\nFinally report which command succeeded."
    );
    let mut events = provider
        .prompt(&session.id, PromptInput::text(prompt))
        .await
        .unwrap();
    let mut bash_calls = 0;
    let mut permission_requests = 0;
    let mut run_status = None;
    let mut usage = RunUsage::default();
    while let Some(event) = events.next().await {
        match event {
            AgentEvent::ToolCall { call, .. } if call.tool_name.as_deref() == Some("bash") => {
                bash_calls += 1;
            }
            AgentEvent::PermissionRequest { request } => {
                permission_requests += 1;
                provider
                    .respond(
                        &session.id,
                        ClientResponse::Permission {
                            request: request.id,
                            option: "allow_once".to_string(),
                            feedback: None,
                        },
                    )
                    .await
                    .unwrap();
            }
            AgentEvent::RunFinished { outcome, .. } => {
                run_status = Some(outcome.status);
                usage = outcome.usage.unwrap_or_default();
                break;
            }
            _ => {}
        }
    }

    let receipt = LiveSandboxReceipt {
        model,
        bash_calls,
        permission_requests,
        inside_created: inside_path.exists(),
        outside_created: outside_path.exists(),
        run_status,
        usage,
    };
    println!("{}", serde_json::to_string_pretty(&receipt).unwrap());
    assert!(receipt.bash_calls >= 2, "model did not attempt both probes");
    assert!(receipt.inside_created, "workspace write did not succeed");
    assert!(!receipt.outside_created, "sandbox allowed an outside write");
    assert_eq!(receipt.run_status, Some(RunStatus::Done));
}

#[cfg(unix)]
fn write_command(path: &std::path::Path, value: &str) -> String {
    format!(
        "printf '{}' > '{}'",
        value.replace('\'', "'\\''"),
        path.to_string_lossy().replace('\'', "'\\''")
    )
}

#[cfg(windows)]
fn write_command(path: &std::path::Path, value: &str) -> String {
    format!(
        "Set-Content -LiteralPath '{}' -NoNewline -Value '{}'",
        path.to_string_lossy().replace('\'', "''"),
        value.replace('\'', "''")
    )
}
