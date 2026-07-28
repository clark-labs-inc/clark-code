use super::*;
use crate::sandbox::Sandbox;
use crate::tools::ReadTracker;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

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
        call_progress: None,
        model_override: None,
    }
}

fn platform_command(posix: &'static str, powershell: &'static str) -> &'static str {
    if cfg!(windows) {
        powershell
    } else {
        posix
    }
}

#[test]
fn scoped_host_selection_only_crosses_a_managed_boundary() {
    use exec_core::ExecutionContainment::{External, Host, Managed};

    assert!(requires_scoped_host(
        &json!({"command": "gh pr view 123"}),
        "gh pr view 123",
        Managed,
    ));
    assert!(requires_scoped_host(
        &json!({
            "command": "custom-tool",
            "sandbox_permissions": "require_escalated",
        }),
        "custom-tool",
        Managed,
    ));
    assert!(!requires_scoped_host(
        &json!({"command": "cargo test"}),
        "cargo test",
        Managed,
    ));
    assert!(!requires_scoped_host(
        &json!({"command": "gh pr view 123"}),
        "gh pr view 123",
        Host,
    ));
    assert!(!requires_scoped_host(
        &json!({"command": "gh pr view 123"}),
        "gh pr view 123",
        External,
    ));
}

#[test]
fn only_opaque_mutations_across_the_host_boundary_require_effect_verification() {
    assert!(Bash
        .effect_intent(&json!({
            "command": "gh pr create --title test --body-file body.md",
            "effect": "create",
            "effect_target": "pull request"
        }))
        .is_some());
    assert!(Bash
        .effect_intent(&json!({"command": "gh pr view 123 --json body", "effect": "none"}))
        .is_none());
    assert!(Bash
        .effect_intent(&json!({"command": "cargo test"}))
        .is_none());
    assert!(Bash
        .effect_intent(&json!({
            "command": "generic-publisher create resource.json",
            "sandbox_permissions": "require_escalated",
            "effect": "publish"
        }))
        .is_some());
}

#[tokio::test]
async fn streams_progress_deltas_while_running() {
    let dir = tempfile::tempdir().unwrap();
    let deltas = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = deltas.clone();
    let mut ctx = ctx(dir.path());
    ctx.progress = Some(Arc::new(move |d: String| sink.lock().unwrap().push(d)));
    let out = Bash
        .invoke(
            json!({"command": platform_command(
                "printf first; sleep 0.05; printf second",
                "[Console]::Out.Write('first'); Start-Sleep -Milliseconds 150; [Console]::Out.Write('second')"
            )}),
            &ctx,
        )
        .await;
    assert!(!out.is_error, "{}", out.content);
    let streamed = deltas.lock().unwrap().join("");
    assert!(streamed.contains("first"), "streamed: {streamed:?}");
    assert!(streamed.contains("second"), "streamed: {streamed:?}");
    // The sleep between writes forces at least two separate chunks.
    assert!(deltas.lock().unwrap().len() >= 2);
}

#[tokio::test]
async fn runs_command_and_captures_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let out = Bash
        .invoke(
            json!({"command": platform_command("echo hello", "Write-Output 'hello'")}),
            &ctx(dir.path()),
        )
        .await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("exit_code: 0"));
    assert!(out.content.contains("hello"));
}

#[cfg(unix)]
#[tokio::test]
async fn ordinary_git_commit_heredoc_preserves_hooks_and_attribution() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["init", "-q"][..],
        &["config", "user.name", "Human Author"][..],
        &["config", "user.email", "human@example.com"][..],
    ] {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    std::fs::write(dir.path().join("work.txt"), "done\n").unwrap();
    let status = Command::new("git")
        .args(["add", "--", "work.txt"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    let hook = dir.path().join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nprintf hook-ran > hook-ran\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let out = Bash
        .invoke(
            json!({
                "command": "git commit -m \"$(cat <<'EOF'\ntest: attributed commit\n\nCo-Authored-By: Clark Code <noreply@clarkchat.com>\nEOF\n)\""
            }),
            &ctx(dir.path()),
        )
        .await;
    assert!(!out.is_error, "{}", out.content);
    assert!(dir.path().join("hook-ran").exists());
    let message = Command::new("git")
        .args(["show", "-s", "--format=%B", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&message.stdout)
            .contains("Co-Authored-By: Clark Code <noreply@clarkchat.com>"),
        "{}",
        String::from_utf8_lossy(&message.stdout)
    );
}

#[tokio::test]
async fn nonzero_exit_is_flagged_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = Bash
        .invoke(
            json!({"command": platform_command("exit 3", "exit 3")}),
            &ctx(dir.path()),
        )
        .await;
    assert!(out.is_error);
    assert!(out.content.contains("exit_code: 3"));
}

#[tokio::test]
async fn runs_in_project_root() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("marker.txt"), "").unwrap();
    let out = Bash
        .invoke(
            json!({"command": platform_command("ls", "Get-ChildItem -Name")}),
            &ctx(dir.path()),
        )
        .await;
    assert!(out.content.contains("marker.txt"));
}

#[tokio::test]
async fn honors_a_contained_workdir() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    std::fs::write(dir.path().join("nested/marker.txt"), "").unwrap();
    let out = Bash
        .invoke(
            json!({
                "command": platform_command("ls", "Get-ChildItem -Name"),
                "workdir": "nested"
            }),
            &ctx(dir.path()),
        )
        .await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("marker.txt"));
}

#[tokio::test]
async fn cancelled_command_reports_error() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    c.cancel.cancel();
    let out = Bash
        .invoke(
            json!({"command": platform_command("sleep 5", "Start-Sleep -Seconds 5")}),
            &c,
        )
        .await;
    assert!(out.is_error);
    assert!(out.content.contains("cancel"));
}

#[tokio::test]
async fn run_in_background_returns_immediately_and_bash_output_polls_it() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let started = Bash
        .invoke(
            json!({
                "command": platform_command("echo bg-hi", "Write-Output 'bg-hi'"),
                "run_in_background": true
            }),
            &c,
        )
        .await;
    assert!(!started.is_error, "{}", started.content);
    assert!(started.content.contains("bg-"));

    let task_id = started
        .content
        .split('`')
        .nth(1)
        .expect("task id in backticks")
        .to_string();

    // Poll until finished (background task races the assertion).
    let mut output = String::new();
    let attempts = if cfg!(windows) { 500 } else { 100 };
    for _ in 0..attempts {
        let out = BashOutput.invoke(json!({"task_id": task_id}), &c).await;
        output = out.content;
        if output.contains("finished") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(output.contains("bg-hi"), "{output}");
    assert!(output.contains("finished"), "{output}");
}

#[tokio::test]
async fn bash_wait_blocks_in_the_host_until_background_completion() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let started = Bash
        .invoke(
            json!({
                "command": platform_command(
                    "sleep 0.05; echo complete",
                    "Start-Sleep -Milliseconds 50; Write-Output 'complete'"
                ),
                "run_in_background": true
            }),
            &c,
        )
        .await;
    let task_id = started.content.split('`').nth(1).unwrap().to_string();
    let output = BashWait
        .invoke(
            json!({
                "task_id": task_id,
                "timeout_ms": if cfg!(windows) { 10_000 } else { 2_000 },
                "poll_interval_ms": 20
            }),
            &c,
        )
        .await;
    assert!(!output.is_error, "{}", output.content);
    assert!(
        output.content.contains("status: finished"),
        "{}",
        output.content
    );
    assert!(output.content.contains("complete"), "{}", output.content);
}

#[tokio::test]
async fn bash_wait_can_return_on_readiness_without_stopping_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let started = Bash
        .invoke(
            json!({
                "command": platform_command(
                    "echo SERVER_READY; sleep 2",
                    "Write-Output 'SERVER_READY'; Start-Sleep -Seconds 2"
                ),
                "run_in_background": true
            }),
            &c,
        )
        .await;
    let task_id = started.content.split('`').nth(1).unwrap().to_string();
    let output = BashWait
        .invoke(
            json!({
                "task_id": task_id,
                "output_contains": "SERVER_READY",
                "timeout_ms": if cfg!(windows) { 10_000 } else { 1_000 }
            }),
            &c,
        )
        .await;
    assert!(!output.is_error, "{}", output.content);
    assert!(
        output.content.contains("status: ready"),
        "{}",
        output.content
    );
    assert_eq!(output.details["process_finished"], false);
    c.background.kill(&task_id).await.unwrap();
}

#[tokio::test]
async fn bash_input_writes_and_closes_background_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let started = Bash
        .invoke(
            json!({
                "command": platform_command(
                    "read value; printf 'value:%s' \"$value\"",
                    r#"$value = [Console]::In.ReadLine(); [Console]::Out.Write("value:$value")"#
                ),
                "run_in_background": true
            }),
            &c,
        )
        .await;
    let task_id = started.content.split('`').nth(1).unwrap().to_string();
    let sent = BashInput
        .invoke(
            json!({"task_id": task_id, "text": "hello\n", "close": true}),
            &c,
        )
        .await;
    assert!(!sent.is_error, "{}", sent.content);
    let mut output = String::new();
    let attempts = if cfg!(windows) { 500 } else { 100 };
    for _ in 0..attempts {
        output = BashOutput
            .invoke(json!({"task_id": task_id}), &c)
            .await
            .content;
        if output.contains("finished") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(output.contains("value:hello"), "{output}");
}

#[tokio::test]
async fn bash_kill_stops_a_background_task() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let started = Bash
        .invoke(
            json!({
                "command": platform_command("sleep 30", "Start-Sleep -Seconds 30"),
                "run_in_background": true
            }),
            &c,
        )
        .await;
    let task_id = started
        .content
        .split('`')
        .nth(1)
        .expect("task id in backticks")
        .to_string();
    let killed = BashKill.invoke(json!({"task_id": task_id}), &c).await;
    assert!(!killed.is_error, "{}", killed.content);
}

#[tokio::test]
async fn bash_output_reports_unknown_task() {
    let dir = tempfile::tempdir().unwrap();
    let out = BashOutput
        .invoke(json!({"task_id": "bg-nope"}), &ctx(dir.path()))
        .await;
    assert!(out.is_error);
}

#[tokio::test]
async fn bash_output_marks_nonzero_background_exit_as_error() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let started = Bash
        .invoke(
            json!({
                "command": platform_command(
                    "echo failed; exit 7",
                    "Write-Output 'failed'; exit 7"
                ),
                "run_in_background": true
            }),
            &c,
        )
        .await;
    let task_id = started.content.split('`').nth(1).unwrap().to_string();
    let attempts = if cfg!(windows) { 500 } else { 100 };
    for _ in 0..attempts {
        let output = BashOutput.invoke(json!({"task_id": task_id}), &c).await;
        if output.content.contains("finished") {
            assert!(output.is_error, "{}", output.content);
            assert!(output.content.contains("exit_code: 7"));
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("background command never finished");
}

struct RemoteLikeExecutor;
#[async_trait::async_trait]
impl crate::exec::Executor for RemoteLikeExecutor {
    fn is_local(&self) -> bool {
        false
    }
    async fn read(&self, path: &std::path::Path) -> exec_core::ExecResult<Vec<u8>> {
        crate::exec::LocalExecutor.read(path).await
    }
    async fn write(&self, path: &std::path::Path, data: &[u8]) -> exec_core::ExecResult<()> {
        crate::exec::LocalExecutor.write(path, data).await
    }
    async fn create_dir_all(&self, path: &std::path::Path) -> exec_core::ExecResult<()> {
        crate::exec::LocalExecutor.create_dir_all(path).await
    }
    async fn remove_file(&self, path: &std::path::Path) -> exec_core::ExecResult<()> {
        crate::exec::LocalExecutor.remove_file(path).await
    }
    async fn remove_dir_all(&self, path: &std::path::Path) -> exec_core::ExecResult<()> {
        crate::exec::LocalExecutor.remove_dir_all(path).await
    }
    async fn rename(
        &self,
        from: &std::path::Path,
        to: &std::path::Path,
    ) -> exec_core::ExecResult<()> {
        crate::exec::LocalExecutor.rename(from, to).await
    }
    async fn read_dir(
        &self,
        path: &std::path::Path,
    ) -> exec_core::ExecResult<Vec<exec_core::DirEntry>> {
        crate::exec::LocalExecutor.read_dir(path).await
    }
    async fn metadata(&self, path: &std::path::Path) -> exec_core::ExecResult<exec_core::FileMeta> {
        crate::exec::LocalExecutor.metadata(path).await
    }
    async fn canonicalize(
        &self,
        path: &std::path::Path,
    ) -> exec_core::ExecResult<std::path::PathBuf> {
        crate::exec::LocalExecutor.canonicalize(path).await
    }
    async fn home_dir(&self, cwd: &std::path::Path) -> exec_core::ExecResult<std::path::PathBuf> {
        crate::exec::LocalExecutor.home_dir(cwd).await
    }
    async fn walk(
        &self,
        root: &std::path::Path,
    ) -> exec_core::ExecResult<Vec<exec_core::WalkEntry>> {
        crate::exec::LocalExecutor.walk(root).await
    }
    async fn exec(
        &self,
        command: &str,
        cwd: &std::path::Path,
        timeout: std::time::Duration,
        cancel: &CancellationToken,
    ) -> exec_core::ExecResult<exec_core::ExecOutput> {
        crate::exec::LocalExecutor
            .exec(command, cwd, timeout, cancel)
            .await
    }
}

#[tokio::test]
async fn unsupported_background_executor_returns_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = ctx(dir.path());
    c.executor = Arc::new(RemoteLikeExecutor);
    let out = Bash
        .invoke(json!({"command": "echo hi", "run_in_background": true}), &c)
        .await;
    assert!(out.is_error);
    assert!(out.content.contains("not supported"), "{}", out.content);
}
