use std::io;
use std::path::{Path, PathBuf};

use agent_core::{
    AgentEvent, ClientResponse, ContentBlock, GoalState, PermissionRequest, PromptInput, Role,
    Snapshot, TimelineItem,
};
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::attachments::AttachmentInput;
use super::command_palette::CommandPalette;
use super::command_runtime::{self, CommandOutcome};
use super::composer::Composer;
use super::goals::PlanGoalState;
use super::permission_profiles::PermissionProfileState;
use super::permissions::PermissionPicker;
use super::provider_events::{classify_steering_result, ProviderEventState, TranscriptEffect};
use super::provider_interaction::{ProviderInteractionSimulation, SteeringEffect};
use super::render::{Transcript, TranscriptKind, TranscriptViewport};
use super::screen;
use super::settings::ModelConfiguration;
use super::status::{StatusPanel, StatusValue};
use super::workspace::WorkspaceInitialization;
use super::TerminalGuard;
use crate::runtime::{ConnectedRuntime, RuntimeDiagnosticValue, RuntimeDiagnostics, Workspace};

pub(super) struct App {
    pub(super) workspace: Workspace,
    pub(super) cwd: String,
    pub(super) transcript: Transcript,
    pub(super) transcript_viewport: TranscriptViewport,
    pub(super) input: Composer,
    pub(super) palette: CommandPalette,
    pub(super) provider_events: ProviderEventState,
    pub(super) permission_picker: Option<PermissionPicker>,
    pub(super) permission_profiles: PermissionProfileState,
    pub(super) permission_profile_path: PathBuf,
    pub(super) pending_workspace_init: Option<WorkspaceInitialization>,
    pub(super) status_panel: StatusPanel,
    pub(super) model_configuration: ModelConfiguration,
    pub(super) model_configuration_path: PathBuf,
    pub(super) attachments: AttachmentInput,
    pub(super) plan_goal: PlanGoalState,
    interaction: ProviderInteractionSimulation,
}

impl App {
    fn new(
        workspace: Workspace,
        cwd: &Path,
        diagnostics: &RuntimeDiagnostics,
        model_configuration: ModelConfiguration,
        collaboration_mode: agent_core::CollaborationMode,
        goal: Option<GoalState>,
        snapshot: &Snapshot,
    ) -> Self {
        let specialist = workspace.paid_specialist_kind().is_some();
        let permission_profile_path = PermissionProfileState::path(cwd);
        let model_configuration_path = ModelConfiguration::path(cwd);
        let (permission_profiles, permission_notice) =
            match PermissionProfileState::load(&permission_profile_path) {
                Ok(state) => (state, None),
                Err(error) => (PermissionProfileState::default(), Some(error)),
            };
        let mut transcript = Transcript::with_system(if specialist {
            "Paid Clark specialist connected. Durable specialist state must synchronize with Clark Cloud before a turn can finish."
        } else {
            "Clark Cloud connected. Ask Clark to work in this project."
        });
        if let Some(notice) = permission_notice {
            transcript.push(TranscriptKind::Error, notice);
        }
        restore_cloud_transcript(&mut transcript, snapshot);
        Self {
            workspace,
            cwd: cwd.display().to_string(),
            transcript,
            transcript_viewport: TranscriptViewport::default(),
            input: Composer::default(),
            palette: CommandPalette::default(),
            provider_events: ProviderEventState::default(),
            permission_picker: None,
            permission_profiles,
            permission_profile_path,
            pending_workspace_init: None,
            status_panel: StatusPanel::new(
                status_value(&diagnostics.authentication),
                status_value(&diagnostics.organization),
                status_value(&diagnostics.plan),
                status_value(&diagnostics.workspace),
                status_value(&diagnostics.provider),
                status_value(&diagnostics.sync),
                diagnostics.configuration.iter().map(status_value).collect(),
            ),
            model_configuration,
            model_configuration_path,
            attachments: AttachmentInput::default(),
            plan_goal: PlanGoalState::with_goal(collaboration_mode, goal),
            interaction: ProviderInteractionSimulation::default(),
        }
    }

    fn apply(&mut self, event: &AgentEvent) {
        self.plan_goal.observe_event(event);
        for effect in self.provider_events.apply(event) {
            match effect {
                TranscriptEffect::AppendClark(text) => {
                    self.transcript.append(TranscriptKind::Clark, &text);
                }
                TranscriptEffect::Push { kind, text } => self.transcript.push(kind, text),
            }
        }
        self.transcript_viewport.follow_bottom();
    }

    pub(super) fn refresh_palette(&mut self) {
        self.palette.sync(self.input.slash_query());
    }

    fn complete_selected_command(&mut self) -> bool {
        let Some(name) = self.palette.selected().map(|spec| spec.name) else {
            return false;
        };
        self.input.replace_text(format!("/{name}"));
        self.refresh_palette();
        true
    }
}

pub async fn run(
    runtime: &mut ConnectedRuntime,
    workspace: Workspace,
    cwd: &Path,
) -> Result<(), String> {
    let goal = runtime
        .provider
        .goal_state(&runtime.session.id)
        .await
        .map_err(|error| format!("could not restore Clark goal state: {error}"))?;
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)
        .map_err(|error| format!("could not initialize Clark TUI: {error}"))?;
    let mut events = EventStream::new();
    let mut app = App::new(
        workspace,
        cwd,
        &runtime.diagnostics,
        runtime.model_configuration.clone(),
        runtime.session.collaboration_mode,
        goal,
        &runtime.conversation.snapshot,
    );
    loop {
        if !app.provider_events.running {
            if let Some(follow_up) = app.interaction.next_follow_up() {
                app.provider_events.mark_starting();
                run_turn(
                    &mut terminal,
                    &mut events,
                    runtime,
                    &mut app,
                    PromptInput::text(workspace.default_prompt(&follow_up)),
                )
                .await?;
                continue;
            }
        }
        screen::draw(&mut terminal, &app)?;
        let Some(event) = events.next().await else {
            break;
        };
        let event = event.map_err(|error| format!("terminal input failed: {error}"))?;
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                code if app.pending_workspace_init.is_some() => {
                    command_runtime::handle_pending_workspace_init(&mut app, code);
                }
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Char('c')
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    command_runtime::copy_transcript_selection(&mut app);
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if app.provider_events.running {
                        app.provider_events.mark_cancelling();
                        if let Some(request) = app.interaction.cancellation(
                            &runtime.session.id,
                            app.provider_events.current_run.as_ref(),
                        ) {
                            let _ = runtime
                                .provider
                                .cancel(&request.session, &request.run)
                                .await;
                        }
                    } else {
                        break;
                    }
                }
                KeyCode::Enter
                    if !app.provider_events.running
                        && key
                            .modifiers
                            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
                {
                    app.input.insert_newline();
                    app.refresh_palette();
                }
                KeyCode::Enter if !app.provider_events.running && app.palette.is_open() => {
                    if let Some(name) = app.palette.selected().map(|spec| spec.name) {
                        if app.input.slash_query() != Some(name) {
                            app.complete_selected_command();
                        } else if let Some(exit) =
                            command_runtime::apply_named_command(&mut app, runtime, name).await
                        {
                            if exit {
                                break;
                            }
                        } else {
                            app.provider_events.status = format!("/{name} has no Clark handler");
                        }
                    }
                }
                KeyCode::Enter if !app.provider_events.running => {
                    if let Some(input) = app.input.submit() {
                        match command_runtime::apply_command_line(&mut app, runtime, &input).await {
                            CommandOutcome::Handled => continue,
                            CommandOutcome::Exit => break,
                            CommandOutcome::NotCommand => {}
                        }
                        let label = app.attachments.submission_label();
                        app.transcript
                            .push(TranscriptKind::User, format!("{input}{label}"));
                        let prompt = app.attachments.prompt(workspace.default_prompt(&input));
                        app.provider_events.mark_starting();
                        run_turn(&mut terminal, &mut events, runtime, &mut app, prompt).await?;
                    }
                }
                KeyCode::Backspace if !app.provider_events.running => {
                    app.input.backspace();
                    app.refresh_palette();
                }
                KeyCode::Delete if !app.provider_events.running => {
                    app.input.delete();
                    app.refresh_palette();
                }
                KeyCode::Left if !app.provider_events.running => {
                    app.input.move_left();
                }
                KeyCode::Right if !app.provider_events.running => {
                    app.input.move_right();
                }
                KeyCode::Home if !app.provider_events.running => app.input.move_home(),
                KeyCode::End if !app.provider_events.running => app.input.move_end(),
                KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    extend_transcript_selection(&terminal, &mut app, -1)?;
                }
                KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    extend_transcript_selection(&terminal, &mut app, 1)?;
                }
                KeyCode::Up if !app.provider_events.running => {
                    if !app.palette.is_open() {
                        app.input.move_up_or_history();
                    } else {
                        app.palette.select_previous();
                    }
                }
                KeyCode::Down if !app.provider_events.running => {
                    if !app.palette.is_open() {
                        app.input.move_down_or_history();
                    } else {
                        app.palette.select_next();
                    }
                }
                KeyCode::Tab if !app.provider_events.running && app.palette.is_open() => {
                    app.complete_selected_command();
                }
                KeyCode::Esc if !app.provider_events.running && app.palette.is_open() => {
                    app.input.replace_text(String::new());
                    app.refresh_palette();
                }
                KeyCode::Char(character)
                    if !app.provider_events.running
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    app.input.insert_char(character);
                    app.refresh_palette();
                }
                KeyCode::PageUp => app.transcript_viewport.scroll_up(8),
                KeyCode::PageDown => app.transcript_viewport.scroll_down(8),
                _ => {}
            },
            Event::Paste(text) if !app.provider_events.running => {
                app.input.insert_text(&text);
                app.refresh_palette();
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => app.transcript_viewport.scroll_up(3),
                MouseEventKind::ScrollDown => app.transcript_viewport.scroll_down(3),
                MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::Drag(MouseButton::Left) => {
                    let area = terminal
                        .size()
                        .map_err(|error| format!("could not read terminal size: {error}"))?;
                    let transcript_area = screen::areas(area.into(), &app).transcript;
                    if mouse.column >= transcript_area.x
                        && mouse.column < transcript_area.x.saturating_add(transcript_area.width)
                        && mouse.row >= transcript_area.y
                        && mouse.row < transcript_area.y.saturating_add(transcript_area.height)
                    {
                        let lines = app.transcript.render(false);
                        let source = screen::transcript_source_at_row(
                            transcript_area,
                            &app,
                            usize::from(mouse.row - transcript_area.y),
                        );
                        if source.is_some_and(|source| {
                            app.transcript_viewport.select_source(
                                source,
                                lines.len(),
                                matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left)),
                            )
                        }) {
                            app.provider_events.status =
                                "transcript selection updated · Ctrl+Shift+C copy".into();
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    runtime
        .provider
        .close_session(&runtime.session.id)
        .await
        .map_err(|error| format!("could not close Clark session: {error}"))?;
    Ok(())
}

async fn run_turn(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    events: &mut EventStream,
    runtime: &mut ConnectedRuntime,
    app: &mut App,
    prompt: PromptInput,
) -> Result<(), String> {
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
    app.attachments.clear_after_start();
    while app.provider_events.running {
        screen::draw(terminal, app)?;
        tokio::select! {
            model = stream.next() => {
                let Some(event) = model else {
                    app.provider_events.mark_stream_closed();
                    break;
                };
                runtime.record_event(&event).await?;
                app.apply(&event);
                if let Some(request) = app.provider_events.pending_permission.take() {
                    let option = permission_dialog(terminal, events, app, &request).await?;
                    runtime.provider.respond(
                        &runtime.session.id,
                        ClientResponse::Permission {
                            request: request.id,
                            option,
                            feedback: None,
                        }
                    ).await.map_err(|error| format!("could not answer permission request: {error}"))?;
                    app.provider_events.status = "working".into();
                }
            }
            input = events.next() => {
                let Some(input) = input else { continue };
                let input = input.map_err(|error| format!("terminal input failed: {error}"))?;
                match input {
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press
                            && key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.modifiers.contains(KeyModifiers::SHIFT) =>
                    {
                        command_runtime::copy_transcript_selection(app);
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press
                            && key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        app.provider_events.mark_cancelling();
                        if let Some(request) = app.interaction.cancellation(
                            &runtime.session.id,
                            app.provider_events.current_run.as_ref(),
                        ) {
                            let _ = runtime
                                .provider
                                .cancel(&request.session, &request.run)
                                .await;
                        }
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press
                            && key.code == KeyCode::Enter
                            && key
                                .modifiers
                                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
                    {
                        app.input.insert_newline();
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press && key.code == KeyCode::Enter =>
                    {
                        if let Some(steering) = app.input.submit() {
                            let result = runtime
                                .provider
                                .steer(&runtime.session.id, PromptInput::text(&steering))
                                .await;
                            let disposition = classify_steering_result(result);
                            match app.interaction.resolve_steering(steering, disposition) {
                                SteeringEffect::Delivered(message) => {
                                    app.transcript.push(
                                        TranscriptKind::User,
                                        format!("Steering current turn:\n{message}"),
                                    );
                                    app.provider_events.status = "steering delivered".into();
                                }
                                SteeringEffect::Queued(message) => {
                                    app.transcript.push(
                                        TranscriptKind::User,
                                        format!("Queued follow-up:\n{message}"),
                                    );
                                    app.transcript.push(
                                        TranscriptKind::System,
                                        "This provider rejected mid-run steering; Clark queued the message as the next turn.",
                                    );
                                    app.provider_events.status = "follow-up queued".into();
                                }
                                SteeringEffect::Restore { message, error } => {
                                    app.input.replace_text(message);
                                    app.transcript.push(
                                        TranscriptKind::Error,
                                        format!("Steering failed: {error}"),
                                    );
                                    app.provider_events.status =
                                        "steering failed · message restored".into();
                                }
                            }
                        }
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press && key.code == KeyCode::Backspace =>
                    {
                        app.input.backspace();
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press && key.code == KeyCode::Delete =>
                    {
                        app.input.delete();
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press && key.code == KeyCode::Left =>
                    {
                        app.input.move_left();
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press && key.code == KeyCode::Right =>
                    {
                        app.input.move_right();
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press && key.code == KeyCode::Home =>
                    {
                        app.input.move_home();
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press && key.code == KeyCode::End =>
                    {
                        app.input.move_end();
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press
                            && key.code == KeyCode::Up
                            && key.modifiers.contains(KeyModifiers::SHIFT) =>
                    {
                        extend_transcript_selection(terminal, app, -1)?;
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press
                            && key.code == KeyCode::Down
                            && key.modifiers.contains(KeyModifiers::SHIFT) =>
                    {
                        extend_transcript_selection(terminal, app, 1)?;
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press && key.code == KeyCode::PageUp =>
                    {
                        app.transcript_viewport.scroll_up(8);
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press && key.code == KeyCode::PageDown =>
                    {
                        app.transcript_viewport.scroll_down(8);
                    }
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press
                            && matches!(key.code, KeyCode::Char(_))
                            && !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        if let KeyCode::Char(character) = key.code {
                            app.input.insert_char(character);
                        }
                    }
                    Event::Paste(text) => app.input.insert_text(&text),
                    Event::Mouse(mouse) if mouse.kind == MouseEventKind::ScrollUp => {
                        app.transcript_viewport.scroll_up(3);
                    }
                    Event::Mouse(mouse) if mouse.kind == MouseEventKind::ScrollDown => {
                        app.transcript_viewport.scroll_down(3);
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(receipt) = runtime.sync_after_finish().await? {
        app.status_panel.mark_synchronized(&receipt);
        app.transcript.push(TranscriptKind::System, receipt);
        app.provider_events.status = "cloud synchronized".into();
        screen::draw(terminal, app)?;
    }
    Ok(())
}

fn restore_cloud_transcript(transcript: &mut Transcript, snapshot: &Snapshot) {
    if !snapshot.has_conversation_content() {
        return;
    }
    transcript.push(
        TranscriptKind::System,
        "Restored this account-scoped conversation from Clark Cloud.",
    );
    for item in &snapshot.timeline {
        match item {
            TimelineItem::Message { role, blocks, .. } => {
                let text = blocks
                    .iter()
                    .filter_map(cloud_block_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.is_empty() {
                    transcript.push(
                        if *role == Role::User {
                            TranscriptKind::User
                        } else {
                            TranscriptKind::Clark
                        },
                        text,
                    );
                }
            }
            TimelineItem::SpecialistPresentation { presentation, .. } => transcript.push(
                TranscriptKind::Clark,
                format!("{}\n\n{}", presentation.summary, presentation.takeaway),
            ),
            TimelineItem::ToolCall { id, .. } => {
                if let Some(tool) = snapshot.tool_calls.get(id) {
                    transcript.push(TranscriptKind::Tool, tool.title.clone());
                }
            }
            TimelineItem::Artifact { id } => {
                if let Some(artifact) = snapshot
                    .artifacts
                    .iter()
                    .find(|artifact| &artifact.id == id)
                {
                    transcript.push(
                        TranscriptKind::Artifact,
                        format!(
                            "{} · {}",
                            artifact.title,
                            artifact.uri.as_deref().unwrap_or("cloud artifact")
                        ),
                    );
                }
            }
            TimelineItem::ProviderIncident { id, .. } => {
                if let Some(incident) = snapshot.provider_incidents.get(id) {
                    transcript.push(TranscriptKind::Error, incident.message.clone());
                }
            }
            TimelineItem::ExecutionChecklist { .. } | TimelineItem::ProposedPlan { .. } => {}
        }
    }
}

fn cloud_block_text(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text { text } => Some(text.clone()),
        ContentBlock::Image { uri, .. } => Some(format!(
            "Image{}",
            uri.as_deref()
                .map(|uri| format!(" · {uri}"))
                .unwrap_or_default()
        )),
        ContentBlock::Audio { .. } => Some("Audio".into()),
        ContentBlock::Resource { uri, .. } => Some(format!("Resource · {uri}")),
        ContentBlock::ResourceLink { uri, name } => Some(format!(
            "{} · {uri}",
            name.as_deref().unwrap_or("Resource link")
        )),
        ContentBlock::SkillReference { name, revision, .. } => {
            Some(format!("Skill · {name} · revision {revision}"))
        }
        ContentBlock::Thinking { .. } => None,
    }
}

fn status_value(value: &RuntimeDiagnosticValue) -> StatusValue {
    StatusValue::new(&value.label, &value.value, &value.source)
}

async fn permission_dialog(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    events: &mut EventStream,
    app: &mut App,
    request: &PermissionRequest,
) -> Result<String, String> {
    app.permission_picker = Some(PermissionPicker::from_request(request)?);
    app.transcript.push(
        TranscriptKind::System,
        format!(
            "Permission required: {}\n{}",
            request.title,
            request.detail.as_deref().unwrap_or_default()
        ),
    );
    let result = loop {
        screen::draw(terminal, app)?;
        let Some(event) = events.next().await else {
            break Err("terminal closed while Clark was waiting for permission".into());
        };
        let event = event.map_err(|error| format!("terminal input failed: {error}"))?;
        let Event::Key(key) = event else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let picker = app
            .permission_picker
            .as_mut()
            .expect("permission picker exists while dialog is active");
        match key.code {
            KeyCode::Up => {
                picker.select_previous();
            }
            KeyCode::Down => {
                picker.select_next();
            }
            KeyCode::Enter => break Ok(picker.selected_id()),
            KeyCode::Char('y') | KeyCode::Char('Y') => match picker.allow_once_id() {
                Some(id) => break Ok(id),
                None => continue,
            },
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                match picker.reject_once_id() {
                    Some(id) => break Ok(id),
                    None => continue,
                }
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match picker.reject_once_id() {
                    Some(id) => break Ok(id),
                    None => continue,
                }
            }
            _ => {}
        }
    };
    app.permission_picker = None;
    result
}

fn extend_transcript_selection(
    terminal: &Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    direction: i8,
) -> Result<(), String> {
    let area = terminal
        .size()
        .map_err(|error| format!("could not read terminal size: {error}"))?;
    let transcript_area = screen::areas(area.into(), app).transcript;
    let lines = app.transcript.render(false);
    let default_source = screen::last_visible_transcript_source(transcript_area, app);
    if default_source.is_some_and(|source| {
        app.transcript_viewport
            .extend_selection_from(direction, lines.len(), source)
    }) {
        app.provider_events.status = "transcript selection updated · Ctrl+Shift+C copy".into();
    }
    Ok(())
}
