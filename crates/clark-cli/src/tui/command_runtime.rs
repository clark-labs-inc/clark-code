use std::io::{self, Write};
use std::path::Path;

use base64::Engine;
use crossterm::event::KeyCode;

use super::permission_profiles::PermissionProfileState;
use super::render::TranscriptKind;
use super::session::App;
use super::settings::{ConfigurationRequest, ConfigurationSection, ModelConfiguration};
use super::status::UsageSnapshot;
use super::workspace::{WorkspaceInitialization, WorkspaceInspection};
use crate::runtime::ConnectedRuntime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CommandOutcome {
    NotCommand,
    Handled,
    Exit,
}

pub(super) async fn apply_named_command(
    app: &mut App,
    runtime: &mut ConnectedRuntime,
    name: &str,
) -> Option<bool> {
    match name {
        "attach" => {
            app.input.replace_text("/attach ".into());
            app.refresh_palette();
            app.provider_events.status =
                "enter a project file path, fuzzy query, or --ide, then press Enter".into();
            Some(false)
        }
        "clear" => {
            clear_transcript(app);
            Some(false)
        }
        "goal" => {
            super::goal_commands::apply_command(app, runtime, "/goal").await;
            Some(false)
        }
        "init" => {
            begin_workspace_init(app);
            Some(false)
        }
        "model" => {
            apply_configuration_command(app, runtime, "/model").await;
            Some(false)
        }
        "permissions" => {
            apply_permission_command(app, "/permissions");
            Some(false)
        }
        "quit" => Some(true),
        "status" => {
            apply_status_command(app);
            Some(false)
        }
        _ => None,
    }
}

pub(super) async fn apply_command_line(
    app: &mut App,
    runtime: &mut ConnectedRuntime,
    line: &str,
) -> CommandOutcome {
    let Some(name) = command_name(line) else {
        return CommandOutcome::NotCommand;
    };
    match name {
        "attach" => {
            super::attachment_commands::apply_command(app, runtime, line);
            CommandOutcome::Handled
        }
        "clear" if no_arguments(line) => {
            clear_transcript(app);
            CommandOutcome::Handled
        }
        "goal" => {
            if !super::goal_commands::apply_command(app, runtime, line).await {
                command_usage_error(app, "Usage: /goal [status|resume [--tokens NEW_TOTAL]]");
            }
            CommandOutcome::Handled
        }
        "init" if no_arguments(line) => {
            begin_workspace_init(app);
            CommandOutcome::Handled
        }
        "model" => {
            apply_configuration_command(app, runtime, line).await;
            CommandOutcome::Handled
        }
        "permissions" => {
            apply_permission_command(app, line);
            CommandOutcome::Handled
        }
        "quit" if no_arguments(line) => CommandOutcome::Exit,
        "status" if no_arguments(line) => {
            apply_status_command(app);
            CommandOutcome::Handled
        }
        name if super::command_palette::is_tui_command(name) => {
            command_usage_error(app, &format!("Invalid arguments for /{name}."));
            CommandOutcome::Handled
        }
        name => {
            command_usage_error(
                app,
                &format!(
                    "Unknown Clark command /{name}. Type / to see the intentionally small command set."
                ),
            );
            CommandOutcome::Handled
        }
    }
}

fn command_name(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix('/')?
        .split_whitespace()
        .next()
        .filter(|name| !name.is_empty())
}

fn no_arguments(line: &str) -> bool {
    line.trim()
        .strip_prefix('/')
        .is_some_and(|command| command.split_whitespace().count() == 1)
}

fn show_configuration(app: &mut App, section: ConfigurationSection) {
    let mut report = app.model_configuration.report(section);
    report.push_str(&format!(
        "\nPersistence: {}",
        app.model_configuration_path.display()
    ));
    app.input.replace_text(String::new());
    app.refresh_palette();
    app.transcript.push(TranscriptKind::System, report);
    app.transcript_viewport.follow_bottom();
    app.provider_events.status = "Clark model configuration inspected".into();
}

async fn apply_configuration_command(app: &mut App, runtime: &mut ConnectedRuntime, line: &str) {
    if !ModelConfiguration::handles_line(line) {
        command_usage_error(app, "Usage: /model [MODEL_ID]");
        return;
    }
    let Some(request) = app.model_configuration.request(line) else {
        command_usage_error(app, "Usage: /model [MODEL_ID]");
        return;
    };
    match request {
        Ok(ConfigurationRequest::Inspect(section)) => show_configuration(app, section),
        Err(error) => configuration_error(app, error),
        Ok(ConfigurationRequest::Change(change)) => match runtime.configure(change).await {
            Ok(live) => {
                if let Some(model) = &live.model {
                    app.status_panel.set_configuration(
                        "Model",
                        model,
                        "live Clark provider capability",
                    );
                }
                app.model_configuration.replace_live(live);
                runtime.model_configuration = app.model_configuration.clone();
                let persistence = match app.model_configuration.save(&app.model_configuration_path)
                {
                    Ok(()) => format!(
                        "Applied to the active session and saved to {}.",
                        app.model_configuration_path.display()
                    ),
                    Err(error) => {
                        format!("Applied to this session, but persistence failed: {error}")
                    }
                };
                let mut report = app.model_configuration.report(ConfigurationSection::Model);
                report.push_str(&format!("\n{persistence}"));
                app.transcript.push(TranscriptKind::System, report);
                app.transcript_viewport.follow_bottom();
                app.provider_events.status = "Clark model applied".into();
            }
            Err(error) => configuration_error(app, error),
        },
    }
}

fn configuration_error(app: &mut App, error: String) {
    app.input.replace_text(String::new());
    app.refresh_palette();
    app.transcript
        .push(TranscriptKind::Error, format!("Model unchanged: {error}"));
    app.transcript_viewport.follow_bottom();
    app.provider_events.status = "Clark model unchanged".into();
}

fn apply_permission_command(app: &mut App, line: &str) {
    if !PermissionProfileState::handles_line(line)
        || command_name(line).is_some_and(|name| name != "permissions")
    {
        command_usage_error(
            app,
            "Usage: /permissions [prompt|read-only|workspace-write|add-read-dir PATH|reset-sandbox]",
        );
        return;
    }
    let previous = app.permission_profiles.clone();
    let Some(mut effect) = app.permission_profiles.execute(line, Path::new(&app.cwd)) else {
        return;
    };
    if effect.changed {
        if let Err(error) = app.permission_profiles.save(&app.permission_profile_path) {
            app.permission_profiles = previous;
            effect.status = "permission preference not changed".into();
            effect.transcript = format!(
                "Clark rolled back the permission change because it could not be saved: {error}"
            );
        } else {
            effect.transcript.push_str(&format!(
                "\nSaved to {}.",
                app.permission_profile_path.display()
            ));
        }
    }
    app.input.replace_text(String::new());
    app.refresh_palette();
    app.transcript
        .push(TranscriptKind::System, effect.transcript);
    app.transcript_viewport.follow_bottom();
    app.provider_events.status = effect.status;
}

pub(super) fn handle_pending_workspace_init(app: &mut App, key: KeyCode) {
    let Some(initialization) = app.pending_workspace_init.as_ref() else {
        return;
    };
    match key {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            let result = initialization.confirm();
            app.pending_workspace_init = None;
            match result {
                Ok(receipt) => {
                    app.transcript.push(TranscriptKind::System, receipt);
                    app.provider_events.status = "project guidance created".into();
                }
                Err(error) => {
                    app.transcript.push(TranscriptKind::Error, error);
                    app.provider_events.status = "project initialization failed".into();
                }
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.pending_workspace_init = None;
            app.provider_events.status = "project initialization cancelled".into();
        }
        _ => {
            app.provider_events.status =
                "confirm /init with y or Enter; cancel with n or Esc".into();
        }
    }
    app.transcript_viewport.follow_bottom();
}

fn begin_workspace_init(app: &mut App) {
    app.input.replace_text(String::new());
    app.refresh_palette();
    match WorkspaceInitialization::inspect(Path::new(&app.cwd)) {
        Ok(WorkspaceInspection::AlreadyExists(path)) => {
            app.transcript.push(
                TranscriptKind::System,
                format!(
                    "Project guidance already exists at {}; Clark did not modify it.",
                    path.display()
                ),
            );
            app.provider_events.status = "project guidance already exists".into();
        }
        Ok(WorkspaceInspection::Preview(initialization)) => {
            app.pending_workspace_init = Some(initialization);
            app.provider_events.status = "project initialization requires confirmation".into();
        }
        Err(error) => {
            app.transcript.push(TranscriptKind::Error, error);
            app.provider_events.status = "project initialization unavailable".into();
        }
    }
}

fn apply_status_command(app: &mut App) {
    let usage = app.provider_events.usage.map(|usage| UsageSnapshot {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        context_tokens: usage.context_tokens,
        context_limit: usage.context_limit,
        cost_usd: usage.cost_usd,
    });
    let report = app.status_panel.render(&app.provider_events.status, usage);
    app.input.replace_text(String::new());
    app.refresh_palette();
    app.transcript.push(TranscriptKind::System, report);
    app.transcript_viewport.follow_bottom();
    app.provider_events.status = "/status shown".into();
}

fn clear_transcript(app: &mut App) {
    app.transcript_viewport.clear_selection();
    app.transcript.clear_with_notice("Transcript cleared.");
    app.provider_events.status = "transcript cleared".into();
    app.transcript_viewport.follow_bottom();
}

fn command_usage_error(app: &mut App, message: &str) {
    app.input.replace_text(String::new());
    app.refresh_palette();
    app.transcript.push(TranscriptKind::Error, message);
    app.transcript_viewport.follow_bottom();
    app.provider_events.status = "command rejected locally".into();
}

pub(super) fn copy_transcript_selection(app: &mut App) {
    let lines = app.transcript.render(false);
    let selected = app.transcript_viewport.selected_text(&lines);
    let text = selected
        .as_deref()
        .or_else(|| app.transcript.last_text(TranscriptKind::Clark));
    app.provider_events.status = match text {
        Some(text) => request_terminal_clipboard_copy(text)
            .map(|()| {
                if selected.is_some() {
                    "selected transcript copy request sent to terminal".to_string()
                } else {
                    "last Clark response copy request sent to terminal".to_string()
                }
            })
            .unwrap_or_else(|error| format!("copy failed · {error}")),
        None => "nothing to copy · select transcript text or wait for Clark".into(),
    };
}

fn request_terminal_clipboard_copy(text: &str) -> Result<(), String> {
    const MAX_OSC52_BYTES: usize = 100_000;
    if text.len() > MAX_OSC52_BYTES {
        return Err(format!(
            "last response is {} bytes; terminal copy limit is {MAX_OSC52_BYTES}",
            text.len()
        ));
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut stdout = io::stdout();
    write!(stdout, "\x1b]52;c;{encoded}\x07").map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}
