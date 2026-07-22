use std::{path::Path, time::Duration};

use provider_local::Executor;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

const GIT_CONTEXT_TIMEOUT: Duration = Duration::from_secs(5);
const ACTIVITY_TIMEOUT: Duration = Duration::from_secs(3);
const AGENT_ACTIVITY_WINDOW_SECONDS: u64 = 5 * 60;
const GIT_CONTEXT_COMMAND: &str = "git rev-parse --is-inside-work-tree && \
git rev-parse --show-toplevel && \
(git symbolic-ref --quiet --short HEAD || { printf 'detached:'; git rev-parse --short HEAD; }) && \
git rev-parse --path-format=absolute --git-dir && \
git rev-parse --path-format=absolute --git-common-dir";

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectContext {
    pub branch: String,
    pub detached: bool,
    pub is_worktree: bool,
    pub worktree_root: String,
    pub activity: ProjectActivity,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectActivity {
    pub changed_files: u32,
    pub untracked_files: u32,
    pub conflicted_files: u32,
    pub external_agents: Vec<ExternalAgentActivity>,
    pub detected_at_ms: u64,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentActivity {
    pub id: String,
    pub title: String,
    pub agent_nickname: Option<String>,
    pub updated_at_ms: u64,
}

pub async fn inspect_project_context(
    executor: &dyn Executor,
    cwd: &Path,
) -> Result<Option<ProjectContext>, String> {
    let output = executor
        .exec(
            GIT_CONTEXT_COMMAND,
            cwd,
            GIT_CONTEXT_TIMEOUT,
            &CancellationToken::new(),
        )
        .await?;
    if output.code != Some(0) {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    if lines.next() != Some("true") {
        return Ok(None);
    }
    let Some(worktree_root) = lines.next().filter(|line| !line.is_empty()) else {
        return Ok(None);
    };
    let Some(branch_line) = lines.next().filter(|line| !line.is_empty()) else {
        return Ok(None);
    };
    let Some(git_dir) = lines.next().filter(|line| !line.is_empty()) else {
        return Ok(None);
    };
    let Some(git_common_dir) = lines.next().filter(|line| !line.is_empty()) else {
        return Ok(None);
    };
    let (detached, branch) = branch_line
        .strip_prefix("detached:")
        .map_or((false, branch_line), |commit| (true, commit));

    let activity = inspect_activity(executor, Path::new(worktree_root), branch, detached).await;

    Ok(Some(ProjectContext {
        branch: branch.to_string(),
        detached,
        is_worktree: git_dir != git_common_dir,
        worktree_root: worktree_root.to_string(),
        activity,
    }))
}

async fn inspect_activity(
    executor: &dyn Executor,
    worktree_root: &Path,
    branch: &str,
    detached: bool,
) -> ProjectActivity {
    let cancel = CancellationToken::new();
    let tree = executor.exec(
        "git status --porcelain=v1 --untracked-files=normal",
        worktree_root,
        ACTIVITY_TIMEOUT,
        &cancel,
    );
    let agent_command = external_agent_activity_command(worktree_root, branch, detached);
    let agents = executor.exec(&agent_command, worktree_root, ACTIVITY_TIMEOUT, &cancel);
    let (tree, agents) = tokio::join!(tree, agents);

    let (changed_files, untracked_files, conflicted_files) = tree
        .ok()
        .filter(|output| output.code == Some(0))
        .map(|output| parse_git_status(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();
    let external_agents = agents
        .ok()
        .filter(|output| output.code == Some(0))
        .map(|output| parse_external_agent_activity(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();

    ProjectActivity {
        changed_files,
        untracked_files,
        conflicted_files,
        external_agents,
        detected_at_ms: unix_time_ms(),
    }
}

fn parse_git_status(status: &str) -> (u32, u32, u32) {
    let mut changed = 0;
    let mut untracked = 0;
    let mut conflicted = 0;
    for line in status.lines().filter(|line| line.len() >= 2) {
        let code = &line[..2];
        if code == "??" {
            untracked += 1;
        } else if matches!(code, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU") {
            conflicted += 1;
        } else {
            changed += 1;
        }
    }
    (changed, untracked, conflicted)
}

/// The external local thread index is an intentionally best-effort signal: when
/// its database is absent (including many remote hosts), the command prints no
/// rows and Clark simply reports no externally observed agents. Text fields
/// are hex-encoded so task titles containing tabs/newlines stay parseable.
fn external_agent_activity_command(worktree_root: &Path, branch: &str, detached: bool) -> String {
    let root = sql_hex(&worktree_root.to_string_lossy());
    let branch_filter = if detached {
        String::new()
    } else {
        format!("AND t.git_branch = CAST(X'{0}' AS TEXT) ", sql_hex(branch))
    };
    let query = format!(
        "SELECT hex(t.id), hex(t.title), t.updated_at, hex(COALESCE(t.agent_nickname, '')) \
         FROM threads t LEFT JOIN thread_spawn_edges e ON e.child_thread_id = t.id \
         WHERE t.archived = 0 \
         AND t.updated_at >= CAST(strftime('%s','now') AS INTEGER) - {AGENT_ACTIVITY_WINDOW_SECONDS} \
         AND (t.cwd = CAST(X'{root}' AS TEXT) \
              OR substr(t.cwd, 1, length(CAST(X'{root}' AS TEXT)) + 1) \
                 = CAST(X'{root}' AS TEXT) || '/') \
         {branch_filter}\
         AND (e.status IS NULL OR e.status NOT IN ('completed', 'cancelled', 'failed')) \
         ORDER BY t.updated_at DESC LIMIT 8;"
    );
    format!(
        "if command -v sqlite3 >/dev/null 2>&1 && [ -r \"$HOME/.codex/state_5.sqlite\" ]; \
         then sqlite3 -readonly -tabs \"$HOME/.codex/state_5.sqlite\" \"{query}\" 2>/dev/null; fi"
    )
}

fn sql_hex(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

fn parse_external_agent_activity(stdout: &str) -> Vec<ExternalAgentActivity> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let id = decode_hex(fields.next()?)?;
            let title = decode_hex(fields.next()?)?;
            let updated_at_ms = fields.next()?.parse::<u64>().ok()?.saturating_mul(1_000);
            let nickname = decode_hex(fields.next()?)?;
            Some(ExternalAgentActivity {
                id,
                title,
                agent_nickname: (!nickname.is_empty()).then_some(nickname),
                updated_at_ms,
            })
        })
        .collect()
}

fn decode_hex(value: &str) -> Option<String> {
    if value.len() % 2 != 0 {
        return None;
    }
    let bytes = value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(pair, 16).ok()
        })
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{
        decode_hex, inspect_project_context, parse_external_agent_activity, parse_git_status,
    };
    use provider_local::LocalExecutor;
    use std::{path::Path, process::Command};

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn distinguishes_the_main_checkout_from_a_linked_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("project");
        let linked = temp.path().join("project-feature");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "test@clark.local"]);
        git(&repo, &["config", "user.name", "Clark Test"]);
        std::fs::write(repo.join("README.md"), "fixture\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-qm", "initial"]);
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature/context-bar",
                linked.to_str().unwrap(),
            ],
        );

        let main = inspect_project_context(&LocalExecutor, &repo)
            .await
            .unwrap()
            .unwrap();
        let worktree = inspect_project_context(&LocalExecutor, &linked)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(main.branch, "main");
        assert!(!main.is_worktree);
        assert_eq!(worktree.branch, "feature/context-bar");
        assert!(worktree.is_worktree);
        assert_eq!(
            worktree.worktree_root,
            linked.canonicalize().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn parses_working_tree_counts_without_double_counting_conflicts() {
        assert_eq!(
            parse_git_status(" M app.tsx\nA  new.rs\n?? notes.md\nUU conflict.txt\n"),
            (2, 1, 1)
        );
    }

    #[test]
    fn parses_hex_encoded_external_agent_rows() {
        let rows = parse_external_agent_activity(
            "7468726561642D31\t46697820636F6D706F736572\t123\t416461\n",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "thread-1");
        assert_eq!(rows[0].title, "Fix composer");
        assert_eq!(rows[0].agent_nickname.as_deref(), Some("Ada"));
        assert_eq!(rows[0].updated_at_ms, 123_000);
        assert_eq!(decode_hex("E29883").as_deref(), Some("☃"));
    }
}
