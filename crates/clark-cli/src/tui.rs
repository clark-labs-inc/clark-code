use std::io;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Terminal;

use crate::runtime::Workspace;

mod attachment_commands;
mod attachment_search;
mod attachments;
mod command_palette;
mod command_runtime;
mod composer;
mod goal_commands;
mod goals;
pub(crate) mod permission_profiles;
mod permissions;
mod provider_events;
mod provider_interaction;
mod render;
mod screen;
mod session;
pub(crate) mod settings;
pub(crate) mod specialists;
mod status;
mod terminal_layout;
mod workspace;

pub use session::run;

#[derive(Clone, Copy)]
struct LoginChoice {
    method: crate::args::LoginMethod,
    label: &'static str,
    description: &'static str,
}

const LOGIN_CHOICES: [LoginChoice; 3] = [
    LoginChoice {
        method: crate::args::LoginMethod::Browser,
        label: "Sign in with your browser",
        description: "Open Clark in this machine's browser",
    },
    LoginChoice {
        method: crate::args::LoginMethod::DeviceCode,
        label: "Sign in with Device Code",
        description: "Approve a one-time code from another device · ideal for SSH",
    },
    LoginChoice {
        method: crate::args::LoginMethod::ApiKey,
        label: "Provide an existing Clark API key",
        description: "Secure entry · Code usage is metered; specialist access follows your plan",
    },
];

struct WorkspaceChoice {
    workspace: Workspace,
    label: &'static str,
    description: &'static str,
    allowed: bool,
    state: String,
}

pub enum WorkspaceSelection {
    Ready(Workspace),
    ChooseOrganization(Workspace),
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|error| format!("could not enter terminal mode: {error}"))?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(format!("could not open Clark TUI: {error}"));
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    }
}

pub async fn select_login_method() -> Result<crate::args::LoginMethod, String> {
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|error| format!("could not initialize Clark sign-in: {error}"))?;
    let mut events = EventStream::new();
    let mut selected = 0usize;
    loop {
        draw_login_picker(&mut terminal, selected)?;
        let Some(event) = events.next().await else {
            return Err("Clark terminal input closed during sign-in".into());
        };
        let event = event.map_err(|error| format!("terminal input failed: {error}"))?;
        let Event::Key(key) = event else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = (selected + 1).min(LOGIN_CHOICES.len() - 1),
            KeyCode::Enter => return Ok(LOGIN_CHOICES[selected].method),
            KeyCode::Esc => return Err("sign-in cancelled".into()),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Err("sign-in cancelled".into())
            }
            _ => {}
        }
    }
}

fn draw_login_picker(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    selected: usize,
) -> Result<(), String> {
    terminal
        .draw(|frame| {
            let chunks = Layout::vertical([
                Constraint::Length(6),
                Constraint::Length((LOGIN_CHOICES.len() * 3 + 1) as u16),
                Constraint::Min(2),
            ])
            .margin(2)
            .split(frame.area());
            frame.render_widget(
                Paragraph::new(Text::from(vec![
                    Line::from(Span::styled(
                        "Clark",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from("Your human-facing terminal agent"),
                    Line::default(),
                    Line::from("Sign in to continue"),
                    Line::from(Span::styled(
                        "The credential identifies this machine. Paid specialist access is checked separately.",
                        Style::default().fg(Color::DarkGray),
                    )),
                ])),
                chunks[0],
            );
            let mut lines = Vec::new();
            for (index, choice) in LOGIN_CHOICES.iter().enumerate() {
                let active = index == selected;
                lines.push(Line::from(Span::styled(
                    format!(
                        "{} {}. {}",
                        if active { ">" } else { " " },
                        index + 1,
                        choice.label
                    ),
                    if active {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                )));
                lines.push(Line::from(Span::styled(
                    format!("     {}", choice.description),
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::default());
            }
            frame.render_widget(Paragraph::new(Text::from(lines)), chunks[1]);
            frame.render_widget(
                Paragraph::new("↑/↓ choose · Enter continue · Esc quit")
                    .style(Style::default().fg(Color::DarkGray)),
                chunks[2],
            );
        })
        .map(|_| ())
        .map_err(|error| format!("could not draw Clark sign-in: {error}"))
}

pub async fn select_workspace(
    context: &crate::cloud::CliContext,
) -> Result<WorkspaceSelection, String> {
    let statuses = context.product_statuses()?;
    let workspace_values = [
        Workspace::Code,
        Workspace::Scout,
        Workspace::SecurityScan,
        Workspace::ScientistDiscover,
        Workspace::RsiCreateEvals,
    ];
    let code_description = if context.uses_metered_platform_key() {
        "General coding agent · metered Platform API usage"
    } else {
        "General coding agent · included on Free"
    };
    let descriptions = [
        code_description,
        "Bounded system cartography · paid",
        "Evidence-backed security scans · paid",
        "Experiments and synchronized journals · paid",
        "Evaluation research and world building · paid",
    ];
    let choices = statuses
        .into_iter()
        .zip(workspace_values)
        .zip(descriptions)
        .map(|((status, workspace), description)| WorkspaceChoice {
            workspace,
            label: status.label,
            description,
            allowed: status.allowed,
            state: status.state,
        })
        .collect::<Vec<_>>();
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|error| format!("could not initialize Clark workspace picker: {error}"))?;
    let mut events = EventStream::new();
    let mut selected = 0usize;
    let mut notice = String::new();
    let access_summary = if context.uses_metered_platform_key() {
        "Code is metered with this Platform API key. All four specialists require a paid plan."
    } else {
        "Code is included. All four specialists require a paid plan."
    };
    loop {
        draw_workspace_picker(&mut terminal, &choices, selected, &notice, access_summary)?;
        let Some(event) = events.next().await else {
            return Err("Clark terminal input closed during workspace selection".into());
        };
        let event = event.map_err(|error| format!("terminal input failed: {error}"))?;
        let Event::Key(key) = event else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
                notice.clear();
            }
            KeyCode::Down => {
                selected = (selected + 1).min(choices.len() - 1);
                notice.clear();
            }
            KeyCode::Enter if choices[selected].allowed => {
                return Ok(WorkspaceSelection::Ready(choices[selected].workspace));
            }
            KeyCode::Enter if choices[selected].state == "organization_selection_required" => {
                return Ok(WorkspaceSelection::ChooseOrganization(
                    choices[selected].workspace,
                ));
            }
            KeyCode::Enter => {
                notice = match choices[selected].state.as_str() {
                    "subscription_required" => format!(
                        "{} is available on paid plans. Nothing was started. https://www.clarkchat.com/billing",
                        choices[selected].label
                    ),
                    "action_needed" => format!(
                        "{} is paused until billing is restored. Nothing was started.",
                        choices[selected].label
                    ),
                    "organization_selection_required" => format!(
                        "{} needs --organization because this account has multiple paid workspaces.",
                        choices[selected].label
                    ),
                    "organization_required" => format!(
                        "{} needs an active Clark organization for durable cloud data.",
                        choices[selected].label
                    ),
                    state => format!(
                        "{} is unavailable ({state}). Nothing was started.",
                        choices[selected].label
                    ),
                };
            }
            KeyCode::Esc => return Err("workspace selection cancelled".into()),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Err("workspace selection cancelled".into())
            }
            _ => {}
        }
    }
}

fn draw_workspace_picker(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    choices: &[WorkspaceChoice],
    selected: usize,
    notice: &str,
    access_summary: &str,
) -> Result<(), String> {
    terminal
        .draw(|frame| {
            let area = frame.area();
            let chunks = Layout::vertical([
                Constraint::Length(5),
                Constraint::Length((choices.len() * 3 + 2) as u16),
                Constraint::Min(3),
            ])
            .margin(2)
            .split(area);
            frame.render_widget(
                Paragraph::new(Text::from(vec![
                    Line::from(Span::styled(
                        "Clark",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from("Choose a workspace"),
                    Line::from(Span::styled(
                        access_summary,
                        Style::default().fg(Color::DarkGray),
                    )),
                ])),
                chunks[0],
            );
            let mut lines = Vec::new();
            for (index, choice) in choices.iter().enumerate() {
                let active = index == selected;
                let marker = if active { ">" } else { " " };
                let access = if choice.allowed {
                    "available"
                } else if choice.state == "organization_selection_required" {
                    "choose workspace"
                } else {
                    "locked"
                };
                let label_style = if active {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if choice.allowed {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{marker} {}", choice.label), label_style),
                    Span::styled(
                        format!("  {access}"),
                        Style::default().fg(if choice.allowed {
                            Color::Green
                        } else {
                            Color::DarkGray
                        }),
                    ),
                ]));
                lines.push(Line::from(Span::styled(
                    format!("    {}", choice.description),
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::default());
            }
            frame.render_widget(Paragraph::new(Text::from(lines)), chunks[1]);
            let footer = if notice.is_empty() {
                "↑/↓ choose · Enter open · Esc quit"
            } else {
                notice
            };
            frame.render_widget(
                Paragraph::new(footer)
                    .style(Style::default().fg(if notice.is_empty() {
                        Color::DarkGray
                    } else {
                        Color::Yellow
                    }))
                    .wrap(Wrap { trim: false }),
                chunks[2],
            );
        })
        .map_err(|error| format!("could not draw Clark workspace picker: {error}"))?;
    Ok(())
}

pub async fn select_conversation(
    workspace: Workspace,
    choices: &[conversation_cloud::ConversationSummary],
) -> Result<Option<String>, String> {
    if choices.is_empty() {
        return Ok(None);
    }
    let visible = choices.iter().take(20).collect::<Vec<_>>();
    let item_count = visible.len() + 1;
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|error| format!("could not initialize Clark conversation picker: {error}"))?;
    let mut events = EventStream::new();
    let mut selected = 0usize;
    loop {
        terminal
            .draw(|frame| {
                let list_height = u16::try_from(item_count.saturating_mul(2).saturating_add(1))
                    .unwrap_or(u16::MAX)
                    .min(frame.area().height.saturating_sub(9));
                let chunks = Layout::vertical([
                    Constraint::Length(5),
                    Constraint::Length(list_height),
                    Constraint::Min(2),
                ])
                .margin(2)
                .split(frame.area());
                frame.render_widget(
                    Paragraph::new(Text::from(vec![
                        Line::from(Span::styled(
                            workspace.label(),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Line::from("Choose a Clark Cloud conversation"),
                        Line::from(Span::styled(
                            "The same account-scoped history is available in Desktop and headless Clark.",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ])),
                    chunks[0],
                );
                let mut lines = vec![
                    Line::from(Span::styled(
                        format!("{} New conversation", if selected == 0 { ">" } else { " " }),
                        if selected == 0 {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        },
                    )),
                    Line::default(),
                ];
                for (index, conversation) in visible.iter().enumerate() {
                    let item_index = index + 1;
                    let active = selected == item_index;
                    lines.push(Line::from(Span::styled(
                        format!(
                            "{} {}",
                            if active { ">" } else { " " },
                            conversation.title
                        ),
                        if active {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        },
                    )));
                    lines.push(Line::from(Span::styled(
                        format!("    {} · rev {}", conversation.id, conversation.rev),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                frame.render_widget(
                    Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
                    chunks[1],
                );
                frame.render_widget(
                    Paragraph::new("↑/↓ choose · Enter open · Esc quit")
                        .style(Style::default().fg(Color::DarkGray)),
                    chunks[2],
                );
            })
            .map_err(|error| format!("could not draw Clark conversation picker: {error}"))?;

        let Some(event) = events.next().await else {
            return Err("Clark terminal input closed during conversation selection".into());
        };
        let event = event.map_err(|error| format!("terminal input failed: {error}"))?;
        let Event::Key(key) = event else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = (selected + 1).min(item_count - 1),
            KeyCode::Enter if selected == 0 => return Ok(None),
            KeyCode::Enter => return Ok(Some(visible[selected - 1].id.clone())),
            KeyCode::Esc => return Err("conversation selection cancelled".into()),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Err("conversation selection cancelled".into());
            }
            _ => {}
        }
    }
}

pub async fn select_organization(
    choices: &[crate::cloud::OrganizationChoice],
) -> Result<String, String> {
    if choices.is_empty() {
        return Err("Clark returned no eligible paid organization to choose".into());
    }
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|error| format!("could not initialize Clark organization picker: {error}"))?;
    let mut events = EventStream::new();
    let mut selected = 0usize;
    loop {
        terminal
            .draw(|frame| {
                let chunks = Layout::vertical([
                    Constraint::Length(5),
                    Constraint::Length((choices.len() * 2 + 1) as u16),
                    Constraint::Min(2),
                ])
                .margin(2)
                .split(frame.area());
                frame.render_widget(
                    Paragraph::new(Text::from(vec![
                        Line::from(Span::styled(
                            "Clark",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Line::from("Choose where this specialist stores its cloud data"),
                        Line::from(Span::styled(
                            "Your API key already identifies you; this selects data ownership only.",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ])),
                    chunks[0],
                );
                let lines = choices
                    .iter()
                    .enumerate()
                    .flat_map(|(index, choice)| {
                        let active = index == selected;
                        [
                            Line::from(Span::styled(
                                format!("{} {}", if active { ">" } else { " " }, choice.name),
                                if active {
                                    Style::default()
                                        .fg(Color::Cyan)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Color::White)
                                },
                            )),
                            Line::from(Span::styled(
                                format!("    {}", choice.id),
                                Style::default().fg(Color::DarkGray),
                            )),
                        ]
                    })
                    .collect::<Vec<_>>();
                frame.render_widget(Paragraph::new(Text::from(lines)), chunks[1]);
                frame.render_widget(
                    Paragraph::new("↑/↓ choose · Enter continue · Esc quit")
                        .style(Style::default().fg(Color::DarkGray)),
                    chunks[2],
                );
            })
            .map_err(|error| format!("could not draw Clark organization picker: {error}"))?;

        let Some(event) = events.next().await else {
            return Err("Clark terminal input closed during organization selection".into());
        };
        let event = event.map_err(|error| format!("terminal input failed: {error}"))?;
        let Event::Key(key) = event else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = (selected + 1).min(choices.len() - 1),
            KeyCode::Enter => return Ok(choices[selected].id.clone()),
            KeyCode::Esc => return Err("organization selection cancelled".into()),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Err("organization selection cancelled".into());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_picker_offers_browser_device_code_and_secret_stdin() {
        assert_eq!(
            LOGIN_CHOICES.map(|choice| choice.method),
            [
                crate::args::LoginMethod::Browser,
                crate::args::LoginMethod::DeviceCode,
                crate::args::LoginMethod::ApiKey,
            ]
        );
        assert!(LOGIN_CHOICES[1].description.contains("SSH"));
        assert!(LOGIN_CHOICES[2].label.contains("API key"));
        assert!(LOGIN_CHOICES[2].description.contains("metered"));
    }
}
