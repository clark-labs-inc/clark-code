use std::io::{self, IsTerminal, Write};

use agent_core::{AgentEvent, ClientResponse, ContentBlock, PermissionOptionKind, PromptInput};
use futures::StreamExt;

use crate::runtime::{ConnectedRuntime, Workspace};
use crate::tui::specialists::CloudContinuityReceipt;

pub async fn run_once(
    runtime: &mut ConnectedRuntime,
    workspace: Workspace,
    prompt: &str,
    json: bool,
) -> Result<(), String> {
    let prompt = PromptInput::text(workspace.default_prompt(prompt));
    runtime.begin_turn(&prompt).await?;
    let mut stream = match runtime.provider.prompt(&runtime.session.id, prompt).await {
        Ok(stream) => stream,
        Err(error) => {
            let sync = runtime.sync_after_finish().await;
            return match sync {
                Ok(_) => Err(format!("Clark could not start the turn: {error}")),
                Err(sync_error) => Err(format!(
                    "Clark could not start the turn: {error}\n{sync_error}"
                )),
            };
        }
    };
    let mut wrote_text = false;
    let mut failed = None;
    while let Some(event) = stream.next().await {
        runtime.record_event(&event).await?;
        let specialist_receipt = match &event {
            AgentEvent::Trace {
                source, payload, ..
            } if source == "clark_specialist_projection" => {
                match CloudContinuityReceipt::required_from_projection(payload) {
                    Ok(receipt) => Some(receipt),
                    Err(error) => {
                        failed = Some(error);
                        None
                    }
                }
            }
            _ => None,
        };
        if json {
            println!(
                "{}",
                serde_json::to_string(&event)
                    .map_err(|error| format!("could not encode Clark event: {error}"))?
            );
        } else {
            match &event {
                AgentEvent::MessageChunk {
                    delta: ContentBlock::Text { text },
                    ..
                } => {
                    print!("{text}");
                    io::stdout().flush().ok();
                    wrote_text = true;
                }
                AgentEvent::ToolCall { call, .. } => eprintln!("\n› {}", call.title),
                AgentEvent::Artifact { artifact, .. } => {
                    eprintln!(
                        "\n✓ artifact: {}{}",
                        artifact.title,
                        artifact
                            .uri
                            .as_deref()
                            .map(|uri| format!(" ({uri})"))
                            .unwrap_or_default()
                    );
                }
                AgentEvent::PermissionRequest { request } => {
                    let option = choose_permission(request)?;
                    runtime
                        .provider
                        .respond(
                            &runtime.session.id,
                            ClientResponse::Permission {
                                request: request.id.clone(),
                                option,
                                feedback: None,
                            },
                        )
                        .await
                        .map_err(|error| format!("could not answer permission request: {error}"))?;
                }
                AgentEvent::Error { message, .. } => failed = Some(message.clone()),
                _ => {}
            }
            if let Some(receipt) = specialist_receipt {
                eprintln!("\n✓ {}", receipt.summary());
            }
        }
    }
    if wrote_text {
        println!();
    }
    let sync = runtime.sync_after_finish().await;
    if let Some(error) = failed {
        return match sync {
            Ok(_) => Err(error),
            Err(sync_error) => Err(format!("{error}\n{sync_error}")),
        };
    }
    if let Some(receipt) = sync? {
        eprintln!("✓ {receipt}");
    }
    Ok(())
}

fn choose_permission(request: &agent_core::PermissionRequest) -> Result<String, String> {
    if !io::stdin().is_terminal() {
        return request
            .options
            .iter()
            .find(|option| {
                matches!(
                    option.kind,
                    PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
                )
            })
            .map(|option| option.id.clone())
            .ok_or_else(|| {
                "Clark requires interactive permission, but stdin is not a terminal. Re-run interactively."
                    .into()
            });
    }
    eprintln!("\nPermission required: {}", request.title);
    if let Some(detail) = &request.detail {
        eprintln!("{detail}");
    }
    let allow = rpassword::prompt_password("Allow once? [y/N] ")
        .map_err(|error| format!("could not read permission decision: {error}"))?
        .trim()
        .eq_ignore_ascii_case("y");
    request
        .options
        .iter()
        .find(|option| {
            if allow {
                option.kind == PermissionOptionKind::AllowOnce
            } else {
                option.kind == PermissionOptionKind::RejectOnce
            }
        })
        .or_else(|| request.options.first())
        .map(|option| option.id.clone())
        .ok_or_else(|| "Clark returned a permission request with no choices".into())
}
