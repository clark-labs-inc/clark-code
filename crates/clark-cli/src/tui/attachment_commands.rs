use std::path::Path;

use super::render::TranscriptKind;
use super::session::App;
use crate::runtime::ConnectedRuntime;

pub(crate) fn apply_command(app: &mut App, runtime: &ConnectedRuntime, line: &str) -> bool {
    if line.trim() == "/attach" {
        app.input.replace_text("/attach ".into());
        app.refresh_palette();
        app.provider_events.status =
            "enter a project file path, fuzzy query, or --ide, then press Enter".into();
        return true;
    }
    let Some(result) =
        app.attachments
            .execute(line, Path::new(&app.cwd), &runtime.session.capabilities)
    else {
        return false;
    };
    app.input.replace_text(String::new());
    app.refresh_palette();
    match result {
        Ok(report) => {
            app.transcript.push(TranscriptKind::System, report);
            app.provider_events.status = format!(
                "{} attachment{} staged",
                app.attachments.count(),
                if app.attachments.count() == 1 {
                    ""
                } else {
                    "s"
                }
            );
        }
        Err(error) => {
            app.transcript.push(TranscriptKind::Error, error);
            app.provider_events.status = "attachment unchanged".into();
        }
    }
    app.transcript_viewport.follow_bottom();
    true
}
