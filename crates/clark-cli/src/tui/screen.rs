use std::io;

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;

use super::render::RenderStyle;
use super::session::App;
use super::terminal_layout::wrap_line;

pub(super) struct ScreenAreas {
    pub(super) transcript: Rect,
}

pub(super) fn areas(area: Rect, app: &App) -> ScreenAreas {
    let composer = app.input.viewport(8, area.width.saturating_sub(2));
    let palette_rows = if app.provider_events.running {
        0
    } else {
        app.palette.rows(5).len()
    };
    let palette_height = if palette_rows == 0 {
        0
    } else {
        u16::try_from(palette_rows).unwrap_or(5).saturating_add(2)
    };
    let permission_height = app
        .permission_picker
        .as_ref()
        .map_or(0, |picker| picker.desired_height());
    let workspace_init_height = app
        .pending_workspace_init
        .as_ref()
        .map_or(0, |initialization| initialization.desired_height());
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(app.plan_goal.desired_height()),
        Constraint::Length(permission_height),
        Constraint::Length(workspace_init_height),
        Constraint::Length(palette_height),
        Constraint::Length(composer.height + 2),
        Constraint::Length(1),
    ])
    .split(area);
    ScreenAreas {
        transcript: rows[1],
    }
}

pub(super) fn draw(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &App,
) -> Result<(), String> {
    terminal
        .draw(|frame| {
            let colors = ThemeColors::default();
            let composer = app.input.viewport(8, frame.area().width.saturating_sub(2));
            let palette = if app.provider_events.running {
                Vec::new()
            } else {
                app.palette.rows(5)
            };
            let palette_height = if palette.is_empty() {
                0
            } else {
                u16::try_from(palette.len()).unwrap_or(5).saturating_add(2)
            };
            let permission_height = app
                .permission_picker
                .as_ref()
                .map_or(0, |picker| picker.desired_height());
            let workspace_init_height = app
                .pending_workspace_init
                .as_ref()
                .map_or(0, |initialization| initialization.desired_height());
            let rows = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(4),
                Constraint::Length(app.plan_goal.desired_height()),
                Constraint::Length(permission_height),
                Constraint::Length(workspace_init_height),
                Constraint::Length(palette_height),
                Constraint::Length(composer.height + 2),
                Constraint::Length(1),
            ])
            .split(frame.area());

            let header = Line::from(vec![
                Span::styled(
                    " Clark ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(colors.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {} ", app.workspace.label()),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(&app.cwd, Style::default().fg(colors.muted)),
            ]);
            frame.render_widget(
                Paragraph::new(header).block(Block::default().borders(Borders::BOTTOM)),
                rows[0],
            );

            let rendered = app.transcript.render(false);
            let visual = visual_transcript(&rendered, rows[1].width);
            let (visible_start, visible_end) = app
                .transcript_viewport
                .visible_range(visual.len(), usize::from(rows[1].height));
            let transcript = Text::from(
                visual[visible_start..visible_end]
                    .iter()
                    .map(|line| {
                        let mut style = transcript_style(line.style, colors);
                        if app.transcript_viewport.is_selected(line.source) {
                            style = style.bg(Color::DarkGray);
                        }
                        Line::from(Span::styled(line.text.as_str(), style))
                    })
                    .collect::<Vec<_>>(),
            );
            frame.render_widget(Paragraph::new(transcript), rows[1]);

            let plan_goal_lines = app.plan_goal.panel_lines();
            if !plan_goal_lines.is_empty() {
                frame.render_widget(
                    Paragraph::new(
                        plan_goal_lines
                            .into_iter()
                            .take(4)
                            .map(Line::from)
                            .collect::<Vec<_>>(),
                    )
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Clark goal state "),
                    ),
                    rows[2],
                );
            }

            if let Some(picker) = &app.permission_picker {
                let mut lines = Vec::new();
                if let Some(detail) = &picker.detail {
                    lines.push(Line::from(detail.as_str()));
                }
                if picker.risk.is_some() || picker.reason.is_some() {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "Risk: {}{}",
                            picker.risk.as_deref().unwrap_or("unspecified"),
                            picker
                                .reason
                                .as_deref()
                                .map(|reason| format!(" · {reason}"))
                                .unwrap_or_default()
                        ),
                        Style::default().fg(match picker.risk.as_deref() {
                            Some("danger") => Color::Red,
                            Some("caution") => Color::Yellow,
                            _ => Color::DarkGray,
                        }),
                    )));
                }
                lines.extend(picker.rows().into_iter().map(|row| {
                    Line::from(vec![
                        Span::styled(
                            format!("{} {}", if row.selected { ">" } else { " " }, row.label),
                            if row.selected {
                                Style::default()
                                    .fg(colors.accent)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            },
                        ),
                        Span::styled(
                            format!(" · {}", row.consequence),
                            Style::default().fg(colors.muted),
                        ),
                    ])
                }));
                frame.render_widget(
                    Paragraph::new(lines).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" Permission · {} ", picker.title)),
                    ),
                    rows[3],
                );
            }

            if let Some(initialization) = &app.pending_workspace_init {
                let mut lines = vec![
                    Line::from(Span::styled(
                        format!("Create: {}", initialization.path.display()),
                        Style::default().fg(colors.accent),
                    )),
                    Line::from("Operation: create new file; existing files are never overwritten"),
                ];
                lines.extend(initialization.preview_lines(6).into_iter().map(Line::from));
                frame.render_widget(
                    Paragraph::new(lines).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Initialize project · y/Enter confirm · n/Esc cancel "),
                    ),
                    rows[4],
                );
            }

            if !palette.is_empty() {
                let lines = palette
                    .iter()
                    .map(|row| {
                        Line::from(vec![
                            Span::styled(
                                format!(
                                    "{} /{:<12}",
                                    if row.selected { ">" } else { " " },
                                    row.spec.name
                                ),
                                if row.selected {
                                    Style::default()
                                        .fg(colors.accent)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Color::White)
                                },
                            ),
                            Span::styled(row.spec.description, Style::default().fg(colors.muted)),
                        ])
                    })
                    .collect::<Vec<_>>();
                frame.render_widget(
                    Paragraph::new(lines).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Clark commands · ↑/↓ select · Tab complete "),
                    ),
                    rows[5],
                );
            }

            let attachment_label = app.attachments.submission_label();
            let title = if app.provider_events.running {
                " Steer running turn ".to_string()
            } else if app.input.slash_query().is_some() {
                " Clark command ".to_string()
            } else {
                format!(" Message{attachment_label} ")
            };
            let input = if app.provider_events.running {
                "Clark is working…"
            } else {
                composer.text.as_str()
            };
            frame.render_widget(
                Paragraph::new(input).block(Block::default().borders(Borders::ALL).title(title)),
                rows[6],
            );
            if !app.provider_events.running {
                frame.set_cursor_position((
                    rows[6]
                        .x
                        .saturating_add(1)
                        .saturating_add(composer.cursor_column),
                    rows[6]
                        .y
                        .saturating_add(1)
                        .saturating_add(composer.cursor_row),
                ));
            }

            frame.render_widget(
                Paragraph::new(format!(
                    "{} · Enter send · Shift+Enter newline · Ctrl+C stop · PgUp/PgDn scroll",
                    format_status(app)
                ))
                .style(Style::default().fg(colors.muted)),
                rows[7],
            );
        })
        .map(|_| ())
        .map_err(|error| format!("could not draw Clark TUI: {error}"))
}

fn format_status(app: &App) -> String {
    match app.provider_events.usage_label() {
        Some(usage) => format!("{} · {usage}", app.provider_events.status),
        None => app.provider_events.status.clone(),
    }
}

#[derive(Clone, Debug)]
struct VisualTranscriptLine {
    source: usize,
    text: String,
    style: RenderStyle,
}

fn visual_transcript(
    rendered: &[super::render::RenderLine],
    width: u16,
) -> Vec<VisualTranscriptLine> {
    rendered
        .iter()
        .enumerate()
        .flat_map(|(source, line)| {
            wrap_line(source, &line.text, usize::from(width))
                .into_iter()
                .map(move |wrapped| VisualTranscriptLine {
                    source,
                    text: wrapped.text,
                    style: line.style,
                })
        })
        .collect()
}

pub(super) fn transcript_source_at_row(area: Rect, app: &App, row: usize) -> Option<usize> {
    let rendered = app.transcript.render(false);
    let visual = visual_transcript(&rendered, area.width);
    let (start, end) = app
        .transcript_viewport
        .visible_range(visual.len(), usize::from(area.height));
    let index = start.saturating_add(row);
    (index < end).then(|| visual[index].source)
}

pub(super) fn last_visible_transcript_source(area: Rect, app: &App) -> Option<usize> {
    let rendered = app.transcript.render(false);
    let visual = visual_transcript(&rendered, area.width);
    let (start, end) = app
        .transcript_viewport
        .visible_range(visual.len(), usize::from(area.height));
    (start < end).then(|| visual[end - 1].source)
}

#[derive(Clone, Copy)]
struct ThemeColors {
    accent: Color,
    muted: Color,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            accent: Color::Cyan,
            muted: Color::DarkGray,
        }
    }
}

fn transcript_style(style: RenderStyle, colors: ThemeColors) -> Style {
    match style {
        RenderStyle::UserLabel => Style::default()
            .fg(colors.accent)
            .add_modifier(Modifier::BOLD),
        RenderStyle::ClarkLabel => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        RenderStyle::SystemLabel => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        RenderStyle::Tool => Style::default().fg(Color::Magenta),
        RenderStyle::Artifact => Style::default().fg(colors.accent),
        RenderStyle::DiffHeader | RenderStyle::DiffHunk | RenderStyle::Code => {
            Style::default().fg(colors.accent)
        }
        RenderStyle::Heading => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        RenderStyle::DiffAdd => Style::default().fg(Color::Green),
        RenderStyle::DiffRemove | RenderStyle::Error => Style::default().fg(Color::Red),
        RenderStyle::Body | RenderStyle::Spacer => Style::default(),
    }
}
