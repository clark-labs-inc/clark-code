use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, ToolCtx, ToolExecutor, ToolOutcome};

const CLARK_TRAILER: &str = "Co-authored-by: Clark Code <noreply@clarkchat.com>";
const COMMIT_TIMEOUT: Duration = Duration::from_secs(120);
static MESSAGE_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct GitCommit;

#[async_trait]
impl ToolExecutor for GitCommit {
    fn name(&self) -> &str {
        "git_commit"
    }

    fn description(&self) -> &str {
        "Create a Git commit from the changes already staged in the repository. This preserves the repository's configured human author and adds Clark Code as a co-author by default. Use bash with `git add -- <specific files>` first; never use `git commit` through bash."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {"type": "string", "description": "The commit subject and optional body, without the Clark Code co-author trailer."},
                "amend": {"type": "boolean", "description": "Amend HEAD instead of creating a new commit (default false)."},
                "allow_empty": {"type": "boolean", "description": "Allow a commit with no staged changes (default false)."},
                "omit_clark_coauthor": {"type": "boolean", "description": "Omit Clark Code attribution only when the user explicitly requested that (default false)."}
            },
            "required": ["message"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn mutating(&self) -> bool {
        true
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let message = match arg_str(&args, "message") {
            Ok(message) if !message.trim().is_empty() => message,
            Ok(_) => return ToolOutcome::error("commit message cannot be empty"),
            Err(error) => return ToolOutcome::error(error),
        };
        if message.contains('\0') {
            return ToolOutcome::error("commit message cannot contain a NUL byte");
        }

        let omit_coauthor = args
            .get("omit_clark_coauthor")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let message = commit_message(&message, omit_coauthor);
        let runtime_dir = ctx.sandbox.root().join(".clark").join("runtime");
        if let Err(error) = ctx.executor.create_dir_all(&runtime_dir).await {
            return ToolOutcome::error(format!("could not prepare commit message: {error}"));
        }
        let id = MESSAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let filename = format!("git-commit-message-{}-{id}", std::process::id());
        let message_path = runtime_dir.join(&filename);
        if let Err(error) = ctx.executor.write(&message_path, message.as_bytes()).await {
            return ToolOutcome::error(format!("could not write commit message: {error}"));
        }

        let mut command = String::from("git commit");
        if args.get("amend").and_then(Value::as_bool).unwrap_or(false) {
            command.push_str(" --amend");
        }
        if args
            .get("allow_empty")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            command.push_str(" --allow-empty");
        }
        command.push_str(" -F .clark/runtime/");
        command.push_str(&filename);

        let output = ctx
            .executor
            .exec_streaming_pty(
                &command,
                ctx.sandbox.root(),
                COMMIT_TIMEOUT,
                &ctx.cancel,
                &|_is_stderr, chunk| ctx.report(String::from_utf8_lossy(chunk).into_owned()),
            )
            .await;
        let cleanup_error = ctx.executor.remove_file(&message_path).await.err();

        let output = match output {
            Ok(output) => output,
            Err(error) => return ToolOutcome::error(format!("git commit failed: {error}")),
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut body = format!(
            "exit_code: {}\n",
            output
                .code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".into())
        );
        if !stdout.trim().is_empty() {
            body.push_str("--- stdout ---\n");
            body.push_str(&stdout);
            if !body.ends_with('\n') {
                body.push('\n');
            }
        }
        if !stderr.trim().is_empty() {
            body.push_str("--- stderr ---\n");
            body.push_str(&stderr);
            if !body.ends_with('\n') {
                body.push('\n');
            }
        }
        if !omit_coauthor && matches!(output.code, Some(0)) {
            body.push_str("Clark Code co-author attribution added.\n");
        }
        if let Some(error) = cleanup_error {
            body.push_str(&format!(
                "warning: could not remove temporary commit message: {error}"
            ));
        }
        let mut outcome = ToolOutcome::ok(body);
        outcome.is_error = !matches!(output.code, Some(0));
        outcome
    }
}

fn commit_message(message: &str, omit_coauthor: bool) -> String {
    let message = message.trim();
    if omit_coauthor {
        return format!("{message}\n");
    }
    let without_existing = message
        .lines()
        .filter(|line| {
            let lower = line.trim().to_ascii_lowercase();
            !(lower.starts_with("co-authored-by:") && lower.contains("noreply@clarkchat.com"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\n\n{CLARK_TRAILER}\n", without_existing.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;
    use crate::tools::ReadTracker;
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn run(dir: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn ctx(dir: &std::path::Path) -> ToolCtx {
        ToolCtx {
            sandbox: Arc::new(Sandbox::new(dir).unwrap()),
            reads: Arc::new(Mutex::new(ReadTracker::default())),
            cancel: CancellationToken::new(),
            executor: Arc::new(crate::exec::LocalExecutor),
            background: Arc::new(crate::background::BackgroundTasks::default()),
            session: Arc::new(tokio::sync::Mutex::new(
                crate::loop_state::SessionState::default(),
            )),
            progress: None,
            agent_progress: None,
        }
    }

    #[test]
    fn normalizes_existing_clark_trailer_to_one_exact_trailer() {
        let message = commit_message(
            "subject\n\nCo-Authored-By: Clark Code <noreply@clarkchat.com>",
            false,
        );
        assert_eq!(message.matches(CLARK_TRAILER).count(), 1);
        assert!(message.ends_with(&format!("\n\n{CLARK_TRAILER}\n")));
    }

    #[tokio::test]
    async fn commits_with_human_author_and_clark_coauthor() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]);
        run(dir.path(), &["config", "user.name", "Human Author"]);
        run(dir.path(), &["config", "user.email", "human@example.com"]);
        std::fs::write(dir.path().join("work.txt"), "done\n").unwrap();
        run(dir.path(), &["add", "--", "work.txt"]);

        let outcome = GitCommit
            .invoke(
                json!({"message": "test: attributed commit"}),
                &ctx(dir.path()),
            )
            .await;
        assert!(!outcome.is_error, "{}", outcome.content);

        let author = Command::new("git")
            .args(["show", "-s", "--format=%an <%ae>", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&author.stdout).trim(),
            "Human Author <human@example.com>"
        );
        let message = Command::new("git")
            .args(["show", "-s", "--format=%B", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let message = String::from_utf8_lossy(&message.stdout);
        assert!(message.contains(CLARK_TRAILER), "{message}");
        assert_eq!(
            std::fs::read_dir(dir.path().join(".clark/runtime"))
                .unwrap()
                .count(),
            0
        );
    }
}
