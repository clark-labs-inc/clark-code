use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use futures::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::exec::Executor;
use crate::git_metadata::{
    optional as git_optional, required as git_required, succeeds as git_succeeds,
};

const MAX_HISTORY_BATCH: usize = 250;
const MAX_DISCOVERED_REPOSITORIES: usize = 100;
const MAX_DISCOVERY_DEPTH: usize = 8;
const MAX_DISCOVERY_DIRECTORIES: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRemote {
    pub name: String,
    pub url: String,
    pub canonical: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryIdentity {
    pub fingerprint: String,
    pub vcs: String,
    pub root: String,
    pub head_oid: Option<String>,
    pub current_branch: Option<String>,
    pub default_branch: Option<String>,
    pub canonical_remote: Option<String>,
    pub remotes: Vec<RepositoryRemote>,
    pub commit_count: u64,
    pub shallow: bool,
    pub dirty: bool,
    pub refs_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommitEvidence {
    pub oid: String,
    pub parent_oids: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub committed_at: String,
    pub subject: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHistoryBatch {
    pub repository: RepositoryIdentity,
    pub offset: usize,
    pub next_offset: usize,
    pub complete: bool,
    pub commits: Vec<GitCommitEvidence>,
}

pub async fn inspect_repository(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Option<RepositoryIdentity>, String> {
    let Some(top_level) = git_optional(exec, root, &["rev-parse", "--show-toplevel"]).await? else {
        return Ok(None);
    };
    let repo_root = PathBuf::from(top_level.trim());
    let repo_root = exec.canonicalize(&repo_root).await.unwrap_or(repo_root);
    let head_args = ["rev-parse", "--verify", "HEAD"];
    let branch_args = ["symbolic-ref", "--quiet", "--short", "HEAD"];
    let roots_args = ["rev-list", "--max-parents=0", "--all"];
    let count_args = ["rev-list", "--count", "--all"];
    let shallow_args = ["rev-parse", "--is-shallow-repository"];
    let dirty_args = ["status", "--porcelain=v1", "--untracked-files=no"];
    let refs_args = ["show-ref", "--head"];
    let (
        head_oid,
        current_branch,
        default_branch,
        remotes,
        roots,
        commit_count,
        shallow,
        dirty,
        refs,
    ) = tokio::join!(
        git_optional(exec, &repo_root, &head_args),
        git_optional(exec, &repo_root, &branch_args),
        default_branch(exec, &repo_root),
        repository_remotes(exec, &repo_root),
        git_optional(exec, &repo_root, &roots_args),
        git_optional(exec, &repo_root, &count_args),
        git_optional(exec, &repo_root, &shallow_args),
        git_optional(exec, &repo_root, &dirty_args),
        git_optional(exec, &repo_root, &refs_args),
    );
    let head_oid = head_oid?.filter(|value| is_oid(value));
    let current_branch = current_branch?.filter(|value| !value.is_empty());
    let default_branch = default_branch?;
    let remotes = remotes?;
    let canonical_remote = preferred_remote(&remotes).map(|remote| remote.canonical.clone());
    let roots = roots?.unwrap_or_default();
    let identity_seed = canonical_remote
        .as_ref()
        .map(|remote| format!("remote:{remote}"))
        .unwrap_or_else(|| format!("roots:{}", sorted_lines(&roots).join(",")));
    let fingerprint = format!("git:{}", sha256_hex(identity_seed.as_bytes()));
    let commit_count = commit_count?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let shallow = shallow?.is_some_and(|value| value == "true");
    let dirty = dirty?.is_some_and(|value| !value.is_empty());
    let refs = refs?.unwrap_or_default();

    Ok(Some(RepositoryIdentity {
        fingerprint,
        vcs: "git".to_string(),
        root: repo_root.to_string_lossy().to_string(),
        head_oid,
        current_branch,
        default_branch,
        canonical_remote,
        remotes,
        commit_count,
        shallow,
        dirty,
        refs_fingerprint: sha256_hex(refs.as_bytes()),
    }))
}

/// Maximum status entries listed in a working-tree snapshot before truncating.
const SNAPSHOT_MAX_ENTRIES: usize = 40;

/// A compact, per-turn `git status` snapshot for the model — the tree may be
/// shared with other agents, so a session-start view goes stale; this is
/// re-taken every message. `None` outside a git repo or on any git failure
/// (non-git projects just get no section).
pub async fn working_tree_snapshot(exec: &dyn Executor, root: &Path) -> Option<String> {
    let raw = git_optional(exec, root, &["status", "--porcelain=v1", "--branch"])
        .await
        .ok()??;
    let mut lines = raw.lines();
    let branch = lines
        .next()
        .unwrap_or("")
        .trim_start_matches("## ")
        .to_string();
    let entries: Vec<&str> = lines.filter(|l| !l.trim().is_empty()).collect();

    let mut s = format!(
        "[Working tree snapshot — fresh `git status` taken for this message]\nBranch: {branch}\n"
    );
    if entries.is_empty() {
        s.push_str("No uncommitted changes.\n");
    } else {
        s.push_str(
            "Uncommitted changes (entries you didn't make are someone else's in-progress \
work — leave them alone):\n",
        );
        for entry in entries.iter().take(SNAPSHOT_MAX_ENTRIES) {
            s.push_str(entry);
            s.push('\n');
        }
        if entries.len() > SNAPSHOT_MAX_ENTRIES {
            s.push_str(&format!(
                "… and {} more\n",
                entries.len() - SNAPSHOT_MAX_ENTRIES
            ));
        }
    }
    Some(s)
}

pub async fn load_git_history(
    exec: &dyn Executor,
    root: &Path,
    offset: usize,
    limit: usize,
) -> Result<Option<GitHistoryBatch>, String> {
    let Some(repository) = inspect_repository(exec, root).await? else {
        return Ok(None);
    };
    let limit = limit.clamp(1, MAX_HISTORY_BATCH);
    let requested = limit + 1;
    let skip = format!("--skip={offset}");
    let max_count = format!("--max-count={requested}");
    let raw = git_required(
        exec,
        Path::new(&repository.root),
        &[
            "log",
            "--all",
            "--topo-order",
            "--date-order",
            &skip,
            &max_count,
            "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%cI%x00%s%x00%b%x1e",
        ],
    )
    .await?;
    let mut commits = parse_history(&raw);
    let complete = commits.len() <= limit;
    commits.truncate(limit);
    let next_offset = offset.saturating_add(commits.len());
    Ok(Some(GitHistoryBatch {
        repository,
        offset,
        next_offset,
        complete,
        commits,
    }))
}

pub async fn discover_repositories(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Vec<RepositoryIdentity>, String> {
    let mut candidates = vec![root.to_path_buf()];
    let mut pending = VecDeque::from([(root.to_path_buf(), 0usize)]);
    let mut inspected = 0usize;
    while let Some((directory, depth)) = pending.pop_front() {
        if inspected >= MAX_DISCOVERY_DIRECTORIES || candidates.len() >= MAX_DISCOVERED_REPOSITORIES
        {
            break;
        }
        inspected += 1;
        let Ok(entries) = exec.read_dir(&directory).await else {
            continue;
        };
        for entry in entries {
            if entry.name == ".git" {
                candidates.push(directory.clone());
                break;
            }
            if depth >= MAX_DISCOVERY_DEPTH
                || !entry.is_dir
                || entry.is_symlink
                || matches!(
                    entry.name.as_str(),
                    "node_modules" | "target" | ".cache" | ".venv"
                )
            {
                continue;
            }
            pending.push_back((directory.join(entry.name), depth + 1));
        }
    }
    let linked = stream::iter(candidates.clone())
        .map(|candidate| async move {
            crate::git_metadata::linked_worktree_roots(exec, &candidate).await
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await?;
    for worktrees in linked.into_iter().flatten() {
        let remaining = MAX_DISCOVERED_REPOSITORIES.saturating_sub(candidates.len());
        candidates.extend(worktrees.into_iter().take(remaining));
        if candidates.len() >= MAX_DISCOVERED_REPOSITORIES {
            break;
        }
    }
    candidates.sort();
    candidates.dedup();

    let mut repositories = stream::iter(candidates)
        .map(|candidate| async move { inspect_repository(exec, &candidate).await })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    repositories.sort_by(|left, right| left.root.cmp(&right.root));
    repositories.dedup_by(|left, right| left.root == right.root);
    repositories.truncate(MAX_DISCOVERED_REPOSITORIES);
    Ok(repositories)
}

async fn default_branch(exec: &dyn Executor, root: &Path) -> Result<Option<String>, String> {
    if let Some(remote_head) = git_optional(
        exec,
        root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .await?
    {
        return Ok(remote_head
            .strip_prefix("origin/")
            .unwrap_or(&remote_head)
            .to_string()
            .into());
    }
    for candidate in ["main", "master"] {
        let reference = format!("refs/heads/{candidate}");
        if git_succeeds(exec, root, &["show-ref", "--verify", "--quiet", &reference]).await? {
            return Ok(Some(candidate.to_string()));
        }
    }
    Ok(None)
}

async fn repository_remotes(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Vec<RepositoryRemote>, String> {
    let raw = git_optional(
        exec,
        root,
        &["config", "--get-regexp", "^remote\\..*\\.url$"],
    )
    .await?
    .unwrap_or_default();
    let mut out = Vec::new();
    for line in raw.lines() {
        let Some((key, raw_url)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Some(name) = key
            .strip_prefix("remote.")
            .and_then(|value| value.strip_suffix(".url"))
        else {
            continue;
        };
        let Some((url, canonical)) = sanitize_remote(raw_url.trim()) else {
            continue;
        };
        out.push(RepositoryRemote {
            name: name.to_string(),
            url,
            canonical,
        });
    }
    out.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.canonical.cmp(&right.canonical))
    });
    out.dedup_by(|left, right| left.name == right.name && left.canonical == right.canonical);
    Ok(out)
}

fn preferred_remote(remotes: &[RepositoryRemote]) -> Option<&RepositoryRemote> {
    ["upstream", "origin"]
        .into_iter()
        .find_map(|name| remotes.iter().find(|remote| remote.name == name))
        .or_else(|| remotes.first())
}

fn sanitize_remote(raw: &str) -> Option<(String, String)> {
    if let Some((user_host, path)) = raw.split_once(':') {
        if !raw.contains("://") && user_host.contains('@') {
            let host = user_host.rsplit_once('@')?.1.to_ascii_lowercase();
            let path = normalize_repo_path(path)?;
            return Some((format!("ssh://{host}/{path}"), format!("{host}/{path}")));
        }
    }
    let mut url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https" | "ssh" | "git") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let path = normalize_repo_path(url.path())?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    Some((url.to_string(), format!("{host}/{path}")))
}

fn normalize_repo_path(path: &str) -> Option<String> {
    let path = path.trim().trim_matches('/').trim_end_matches(".git");
    (!path.is_empty()).then(|| path.to_ascii_lowercase())
}

fn parse_history(raw: &str) -> Vec<GitCommitEvidence> {
    raw.split('\u{1e}')
        .filter_map(|record| {
            let record = record.trim_matches(['\n', '\r']);
            if record.is_empty() {
                return None;
            }
            let mut fields = record.splitn(8, '\0');
            let oid = fields.next()?.trim().to_string();
            if !is_oid(&oid) {
                return None;
            }
            Some(GitCommitEvidence {
                oid,
                parent_oids: fields
                    .next()
                    .unwrap_or_default()
                    .split_whitespace()
                    .filter(|value| is_oid(value))
                    .map(ToString::to_string)
                    .collect(),
                author_name: fields.next().unwrap_or_default().trim().to_string(),
                author_email: fields.next().unwrap_or_default().trim().to_string(),
                authored_at: fields.next().unwrap_or_default().trim().to_string(),
                committed_at: fields.next().unwrap_or_default().trim().to_string(),
                subject: fields.next().unwrap_or_default().trim().to_string(),
                body: fields.next().unwrap_or_default().trim().to_string(),
            })
        })
        .collect()
}

fn sorted_lines(raw: &str) -> Vec<&str> {
    let mut values = raw
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort_unstable();
    values
}

fn is_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
