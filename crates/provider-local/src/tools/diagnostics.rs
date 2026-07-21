//! `check_diagnostics` — run the project's configured check/lint/typecheck
//! command and report only *new* problems since the session's first call.
//! Manual-trigger only (not run automatically per-turn/per-checkpoint — a
//! `tsc`/`cargo check` can take seconds-to-minutes, too slow to run on every
//! turn on a large project) and rides the plain tool-result channel rather
//! than a new domain event, so it stays a self-contained tool with no
//! `agent-core`/cross-provider surface.

use std::collections::HashSet;
use std::time::Duration;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{ToolCtx, ToolExecutor, ToolOutcome};

const CHECK_TIMEOUT: Duration = Duration::from_secs(300);

pub struct CheckDiagnostics;

#[async_trait]
impl ToolExecutor for CheckDiagnostics {
    fn name(&self) -> &str {
        "check_diagnostics"
    }
    fn description(&self) -> &str {
        "Run the project's configured check command (lint/typecheck/build) and report problems. \
        The first call in a session returns everything; later calls report only *new* problems \
        introduced since then, so call it again after making changes to see what you fixed or \
        broke. Requires a check_command configured in .clark/settings.json — errors otherwise."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    async fn invoke(&self, _args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let command = {
            let s = ctx.session.lock().await;
            match &s.check_command {
                Some(c) => c.clone(),
                None => {
                    return ToolOutcome::error(
                        "no check_command configured — set `check_command` in \
                        .clark/settings.json (e.g. \"tsc --noEmit\" or \"cargo check\")",
                    )
                }
            }
        };

        let output = match ctx
            .executor
            .exec(&command, ctx.sandbox.root(), CHECK_TIMEOUT, &ctx.cancel)
            .await
        {
            Ok(o) => o,
            Err(e) => return ToolOutcome::error(e),
        };
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let lines: Vec<String> = combined
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();

        let mut s = ctx.session.lock().await;
        match s.diagnostics_baseline.take() {
            None => {
                s.diagnostics_baseline = Some(lines.clone());
                if lines.is_empty() {
                    ToolOutcome::ok(format!("`{command}` — clean, no output."))
                } else {
                    ToolOutcome::ok(format!(
                        "`{command}` — baseline captured ({} line{}). Call check_diagnostics \
                        again after changes to see what's new:\n\n{}",
                        lines.len(),
                        if lines.len() == 1 { "" } else { "s" },
                        lines.join("\n")
                    ))
                }
            }
            Some(baseline) => {
                let seen: HashSet<&str> = baseline.iter().map(String::as_str).collect();
                let new_lines: Vec<&str> = lines
                    .iter()
                    .map(String::as_str)
                    .filter(|l| !seen.contains(l))
                    .collect();
                s.diagnostics_baseline = Some(lines.clone());
                if new_lines.is_empty() {
                    ToolOutcome::ok(format!("`{command}` — no new problems since the baseline."))
                } else {
                    ToolOutcome::ok(format!(
                        "`{command}` — {} new problem line{} since the baseline:\n\n{}",
                        new_lines.len(),
                        if new_lines.len() == 1 { "" } else { "s" },
                        new_lines.join("\n")
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loop_state::SessionState;
    use crate::sandbox::Sandbox;
    use crate::tools::ReadTracker;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn ctx(dir: &std::path::Path, check_command: Option<&str>) -> ToolCtx {
        let session = SessionState {
            check_command: check_command.map(String::from),
            ..Default::default()
        };
        ToolCtx {
            sandbox: Arc::new(Sandbox::new(dir).unwrap()),
            executor: Arc::new(crate::exec::LocalExecutor),
            reads: Arc::new(Mutex::new(ReadTracker::default())),
            cancel: CancellationToken::new(),
            background: Arc::new(crate::background::BackgroundTasks::default()),
            session: Arc::new(tokio::sync::Mutex::new(session)),
            progress: None,
            agent_progress: None,
            call_progress: None,
        }
    }

    #[tokio::test]
    async fn errors_without_a_configured_check_command() {
        let dir = tempfile::tempdir().unwrap();
        let out = CheckDiagnostics
            .invoke(json!({}), &ctx(dir.path(), None))
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("check_command"));
    }

    #[tokio::test]
    async fn first_call_captures_baseline_and_reports_everything() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(
            dir.path(),
            Some("printf 'a.rs:1: error\\nb.rs:2: error\\n'"),
        );
        let out = CheckDiagnostics.invoke(json!({}), &c).await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("baseline captured"));
        assert!(out.content.contains("a.rs:1: error"));
        assert!(out.content.contains("b.rs:2: error"));
    }

    #[tokio::test]
    async fn second_call_reports_only_new_lines() {
        let dir = tempfile::tempdir().unwrap();
        let flag = dir.path().join("second_run");
        let command = format!(
            "if [ -f {0} ]; then printf 'a.rs:1: error\\nc.rs:3: error\\n'; else touch {0}; printf 'a.rs:1: error\\n'; fi",
            flag.display()
        );
        let c = ctx(dir.path(), Some(&command));
        let first = CheckDiagnostics.invoke(json!({}), &c).await;
        assert!(first.content.contains("a.rs:1: error"));

        let second = CheckDiagnostics.invoke(json!({}), &c).await;
        assert!(!second.is_error, "{}", second.content);
        // Check only the reported new-lines section, not the whole message —
        // the echoed `command` text itself contains "a.rs:1: error" as a shell
        // literal, so a whole-content substring check would false-fail here.
        let reported = second
            .content
            .split("since the baseline:\n\n")
            .nth(1)
            .unwrap();
        assert_eq!(reported, "c.rs:3: error");
    }

    #[tokio::test]
    async fn no_new_problems_is_reported_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path(), Some("printf 'a.rs:1: error\\n'"));
        let _ = CheckDiagnostics.invoke(json!({}), &c).await;
        let second = CheckDiagnostics.invoke(json!({}), &c).await;
        assert!(second.content.contains("no new problems"));
    }
}
