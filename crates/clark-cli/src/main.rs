mod args;
mod auth;
mod cloud;
mod conversation;
mod output;
mod runtime;
mod science_cloud;
mod tui;
mod update;

use std::io::IsTerminal;
use std::path::PathBuf;

use clap::Parser;

use args::{joined_prompt, AuthCommand, Cli, Command};
use runtime::Workspace;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("clark: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Login(login)) => {
            let source = auth::login(login.method()).await?;
            println!("Signed in to Clark. Credential stored in {source}.");
            Ok(())
        }
        Some(Command::Auth {
            command: AuthCommand::Status,
        }) => {
            let (source, email) = auth::status().await?;
            println!(
                "Signed in{} via {source}. Clark Cloud is reachable.",
                email
                    .as_deref()
                    .map(|email| format!(" as {email}"))
                    .unwrap_or_default()
            );
            Ok(())
        }
        Some(Command::Logout) => {
            match auth::logout().await? {
                auth::LogoutResult::NotSignedIn => {
                    println!("No stored Clark credential was found.");
                }
                auth::LogoutResult::RemovedExistingApiKey => {
                    println!("Signed out. The existing Platform API key was removed from this machine but was not revoked.");
                }
                auth::LogoutResult::RevokedMachineCredential => {
                    println!("Signed out and revoked this machine's Clark CLI credential.");
                }
            }
            Ok(())
        }
        Some(Command::Update(args)) => update::run(args.release.as_deref()).await,
        Some(Command::Doctor) => {
            let cwd = resolve_cwd(cli.cwd)?;
            doctor(&cwd, cli.organization.as_deref()).await
        }
        command => {
            let cwd = resolve_cwd(cli.cwd)?;
            let bare_tui =
                command.is_none() && terminal_is_interactive() && !cli.plain && !cli.json;
            let (mut workspace, prompt, create_scout_workspace) = workspace_command(command);
            let credential = match auth::require_credential() {
                Ok(credential) => credential,
                Err(error) if terminal_is_interactive() => {
                    eprintln!("{error}\n");
                    let method = tui::select_login_method().await?;
                    let source = auth::login(method).await?;
                    eprintln!("Signed in. Credential stored in {source}.\n");
                    auth::require_credential()?
                }
                Err(error) => return Err(error),
            };
            let mut context =
                cloud::load_context(&credential.api_key, cli.organization.as_deref()).await?;
            if bare_tui {
                match tui::select_workspace(&context).await? {
                    tui::WorkspaceSelection::Ready(selected) => workspace = selected,
                    tui::WorkspaceSelection::ChooseOrganization(selected) => {
                        let organization_id =
                            tui::select_organization(&context.organization_choices()).await?;
                        context = cloud::load_context(&credential.api_key, Some(&organization_id))
                            .await?;
                        workspace = selected;
                    }
                }
            }
            let scope = cloud::prepare_runtime_scope(
                &context,
                workspace,
                &credential.api_key,
                cli.workspace.as_deref(),
                create_scout_workspace.as_deref(),
                &cwd,
            )
            .await?;
            runtime::sync_before_start(workspace, &scope, &credential.api_key).await?;
            let conversation_cloud = conversation::ConversationCloud::connect(
                &credential.api_key,
                workspace,
                &cwd,
                &scope,
            )?;
            let available_conversations = conversation_cloud.list().await?;
            let selected_conversation = if let Some(id) = cli.conversation.as_deref() {
                Some(id.to_string())
            } else if prompt.is_none() && terminal_is_interactive() && !cli.plain && !cli.json {
                let choices = conversation_cloud.choices(available_conversations);
                tui::select_conversation(workspace, &choices).await?
            } else {
                None
            };
            let conversation = conversation_cloud
                .open(selected_conversation.as_deref())
                .await?;
            let mut connected = runtime::connect(
                workspace,
                &cwd,
                &credential.api_key,
                &credential.created_by,
                &scope,
                conversation,
            )
            .await?;
            if let Some(prompt) = prompt {
                output::run_once(&mut connected, workspace, &prompt, cli.json).await
            } else if terminal_is_interactive() && !cli.plain && !cli.json {
                tui::run(&mut connected, workspace, &cwd).await
            } else {
                Err("No prompt was provided and no interactive terminal is available. Pass a prompt, for example: 'clark code fix the failing test'.".into())
            }
        }
    }
}

fn workspace_command(command: Option<Command>) -> (Workspace, Option<String>, Option<String>) {
    match command {
        None => (Workspace::Code, None, None),
        Some(Command::Code(args)) => (Workspace::Code, joined_prompt(&args.prompt), None),
        Some(Command::Scout(args)) => (
            Workspace::Scout,
            joined_prompt(&args.prompt),
            args.create_workspace,
        ),
        Some(Command::Security(args)) => {
            let workspace = match args.workflow {
                args::SecurityWorkflow::Scan => Workspace::SecurityScan,
                args::SecurityWorkflow::Diff => Workspace::SecurityDiff,
                args::SecurityWorkflow::Deep => Workspace::SecurityDeep,
            };
            (workspace, joined_prompt(&args.prompt), None)
        }
        Some(Command::Scientist(args)) => {
            let workspace = match args.workflow {
                args::ScientistWorkflow::Discover => Workspace::ScientistDiscover,
                args::ScientistWorkflow::Replicate => Workspace::ScientistReplicate,
            };
            (workspace, joined_prompt(&args.prompt), None)
        }
        Some(Command::Rsi(args)) => {
            let workspace = match args.workflow {
                args::RsiWorkflow::Research => Workspace::RsiResearch,
                args::RsiWorkflow::CreateEvals => Workspace::RsiCreateEvals,
                args::RsiWorkflow::BuildWorld => Workspace::RsiBuildWorld,
                args::RsiWorkflow::StressTest => Workspace::RsiStressTest,
                args::RsiWorkflow::Regression => Workspace::RsiRegression,
            };
            (workspace, joined_prompt(&args.prompt), None)
        }
        Some(
            Command::Login(_)
            | Command::Auth { .. }
            | Command::Logout
            | Command::Update(_)
            | Command::Doctor,
        ) => {
            unreachable!("handled before workspace dispatch")
        }
    }
}

fn resolve_cwd(cwd: Option<PathBuf>) -> Result<PathBuf, String> {
    let cwd = match cwd {
        Some(cwd) => cwd,
        None => std::env::current_dir()
            .map_err(|error| format!("could not read the current directory: {error}"))?,
    };
    cwd.canonicalize()
        .map_err(|error| format!("could not open project {}: {error}", cwd.display()))
}

fn terminal_is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

async fn doctor(cwd: &std::path::Path, organization_id: Option<&str>) -> Result<(), String> {
    let mut failed = false;
    println!("Clark CLI {}", env!("CARGO_PKG_VERSION"));
    println!("✓ Project: {}", cwd.display());
    let context = match auth::status().await {
        Ok((source, email)) => {
            println!(
                "✓ Clark Cloud: connected{} via {source}",
                email
                    .map(|email| format!(" as {email}"))
                    .unwrap_or_default()
            );
            let credential = auth::require_credential()?;
            match conversation::probe(&credential.api_key).await {
                Ok(count) => println!(
                    "✓ Conversation sync: account-scoped Clark Cloud store reachable ({count} conversation(s))"
                ),
                Err(error) => {
                    println!("✗ Conversation sync: {error}");
                    failed = true;
                }
            }
            match cloud::load_context(&credential.api_key, organization_id).await {
                Ok(context) => Some(context),
                Err(error) => {
                    println!("✗ Product access: {error}");
                    failed = true;
                    None
                }
            }
        }
        Err(error) => {
            println!("✗ Clark Cloud: {error}");
            failed = true;
            None
        }
    };
    if let Some(context) = &context {
        for product in context.product_statuses()? {
            if product.allowed {
                println!("✓ {}: available", product.label);
            } else if product.label == "Code" {
                println!("✗ Code: {}", product.state);
                failed = true;
            } else {
                let detail = match product.state.as_str() {
                    "subscription_required" => "paid plan required".to_string(),
                    "action_needed" => "billing action required".to_string(),
                    "organization_selection_required" => {
                        "choose a paid workspace with --organization".to_string()
                    }
                    "organization_required" => {
                        "an active Clark organization is required".to_string()
                    }
                    state => state.replace('_', " "),
                };
                println!("· {}: {detail}", product.label);
            }
        }
        let required = context.native_specialist_worker_required();
        match runtime::specialist_worker() {
            Ok(worker) => match runtime::worker_sha256(&worker) {
                Ok(digest) => println!(
                    "✓ Scientist/RSI worker: {} (sha256 {})",
                    worker.display(),
                    &digest[..12]
                ),
                Err(error) if required => {
                    println!("✗ Scientist/RSI worker: {error}");
                    failed = true;
                }
                Err(error) => println!("· Scientist/RSI worker: {error}"),
            },
            Err(error) if required => {
                println!("✗ Scientist/RSI worker: {error}");
                failed = true;
            }
            Err(_) => println!("· Scientist/RSI worker: not required on the current plan"),
        }
    }
    println!(
        "{} Interactive terminal",
        if terminal_is_interactive() {
            "✓"
        } else {
            "·"
        }
    );
    if failed {
        Err("one or more Clark doctor checks failed".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_clark_is_code() {
        assert_eq!(workspace_command(None), (Workspace::Code, None, None));
    }
}
