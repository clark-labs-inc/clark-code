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
    }
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
            json!({"command": "printf first; sleep 0.05; printf second"}),
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
        .invoke(json!({"command": "echo hello"}), &ctx(dir.path()))
        .await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("exit_code: 0"));
    assert!(out.content.contains("hello"));
}

#[tokio::test]
async fn directs_git_commits_to_the_attributed_commit_tool() {
    let dir = tempfile::tempdir().unwrap();
    for command in [
        "git commit -m test",
        "cd nested && git -c commit.gpgsign=false commit -m test",
        "/usr/bin/git --no-pager commit --amend --no-edit",
    ] {
        let out = Bash
            .invoke(json!({"command": command}), &ctx(dir.path()))
            .await;
        assert!(out.is_error, "{command}");
        assert!(out.content.contains("git_commit"), "{}", out.content);
    }
}

#[tokio::test]
async fn does_not_confuse_git_commit_text_with_a_commit_command() {
    let dir = tempfile::tempdir().unwrap();
    for command in ["printf 'git commit'", "echo git commit"] {
        let out = Bash
            .invoke(json!({"command": command}), &ctx(dir.path()))
            .await;
        assert!(!out.is_error, "{command}: {}", out.content);
    }
}

#[tokio::test]
async fn nonzero_exit_is_flagged_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = Bash
        .invoke(json!({"command": "exit 3"}), &ctx(dir.path()))
        .await;
    assert!(out.is_error);
    assert!(out.content.contains("exit_code: 3"));
}

#[tokio::test]
async fn runs_in_project_root() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("marker.txt"), "").unwrap();
    let out = Bash
        .invoke(json!({"command": "ls"}), &ctx(dir.path()))
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
            json!({"command": "ls", "workdir": "nested"}),
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
    let out = Bash.invoke(json!({"command": "sleep 5"}), &c).await;
    assert!(out.is_error);
    assert!(out.content.contains("cancel"));
}

#[tokio::test]
async fn run_in_background_returns_immediately_and_bash_output_polls_it() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(dir.path());
    let started = Bash
        .invoke(
            json!({"command": "echo bg-hi", "run_in_background": true}),
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
    for _ in 0..100 {
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
            json!({"command": "sleep 0.05; echo complete", "run_in_background": true}),
            &c,
        )
        .await;
    let task_id = started.content.split('`').nth(1).unwrap().to_string();
    let output = BashWait
        .invoke(
            json!({"task_id": task_id, "timeout_ms": 2_000, "poll_interval_ms": 20}),
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
            json!({"command": "echo SERVER_READY; sleep 2", "run_in_background": true}),
            &c,
        )
        .await;
    let task_id = started.content.split('`').nth(1).unwrap().to_string();
    let output = BashWait
        .invoke(
            json!({
                "task_id": task_id,
                "output_contains": "SERVER_READY",
                "timeout_ms": 1_000
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
                "command": "read value; printf 'value:%s' \"$value\"",
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
    for _ in 0..100 {
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
            json!({"command": "sleep 30", "run_in_background": true}),
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
            json!({"command": "echo failed; exit 7", "run_in_background": true}),
            &c,
        )
        .await;
    let task_id = started.content.split('`').nth(1).unwrap().to_string();
    for _ in 0..100 {
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
    async fn read_dir(
        &self,
        path: &std::path::Path,
    ) -> exec_core::ExecResult<Vec<exec_core::DirEntry>> {
        crate::exec::LocalExecutor.read_dir(path).await
    }
    async fn metadata(&self, path: &std::path::Path) -> exec_core::ExecResult<exec_core::FileMeta> {
        crate::exec::LocalExecutor.metadata(path).await
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
