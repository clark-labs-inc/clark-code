use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::exec::Executor;

const GIT_TIMEOUT: Duration = Duration::from_secs(15);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HISTORY_BATCH: usize = 250;
const MAX_DISCOVERED_REPOSITORIES: usize = 100;

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
    let Some(top_level) = git_optional(exec, root, "git rev-parse --show-toplevel").await? else {
        return Ok(None);
    };
    let repo_root = PathBuf::from(top_level.trim());
    let head_oid = git_optional(exec, &repo_root, "git rev-parse --verify HEAD")
        .await?
        .filter(|value| is_oid(value));
    let current_branch = git_optional(exec, &repo_root, "git symbolic-ref --quiet --short HEAD")
        .await?
        .filter(|value| !value.is_empty());
    let default_branch = default_branch(exec, &repo_root).await?;
    let remotes = repository_remotes(exec, &repo_root).await?;
    let canonical_remote = preferred_remote(&remotes).map(|remote| remote.canonical.clone());
    let roots = git_optional(exec, &repo_root, "git rev-list --max-parents=0 --all")
        .await?
        .unwrap_or_default();
    let identity_seed = canonical_remote
        .as_ref()
        .map(|remote| format!("remote:{remote}"))
        .unwrap_or_else(|| format!("roots:{}", sorted_lines(&roots).join(",")));
    let fingerprint = format!("git:{}", sha256_hex(identity_seed.as_bytes()));
    let commit_count = git_optional(exec, &repo_root, "git rev-list --count --all")
        .await?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let shallow = git_optional(exec, &repo_root, "git rev-parse --is-shallow-repository")
        .await?
        .is_some_and(|value| value == "true");
    let dirty = git_optional(
        exec,
        &repo_root,
        "git status --porcelain=v1 --untracked-files=no",
    )
    .await?
    .is_some_and(|value| !value.is_empty());
    let refs = git_optional(exec, &repo_root, "git show-ref --head")
        .await?
        .unwrap_or_default();

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
    let command = format!(
        "git log --all --topo-order --date-order --skip={offset} --max-count={requested} \
         --format='%H%x00%P%x00%an%x00%ae%x00%aI%x00%cI%x00%s%x00%b%x1e'"
    );
    let raw = git_required(exec, Path::new(&repository.root), &command).await?;
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
    let command = r"find . -mindepth 1 -maxdepth 8 \
        \( -type d \( -name node_modules -o -name target -o -name .cache -o -name .venv \) -prune \) -o \
        \( \( -type d -o -type f \) -name .git -print \)";
    let output = exec
        .exec(command, root, DISCOVERY_TIMEOUT, &CancellationToken::new())
        .await?;
    let mut candidates = vec![root.to_path_buf()];
    if output.code == Some(0) {
        for path in String::from_utf8_lossy(&output.stdout).lines() {
            let path = path.trim();
            if path.is_empty() {
                continue;
            }
            let git_path = root.join(path);
            if let Some(repository_root) = git_path.parent() {
                candidates.push(repository_root.to_path_buf());
            }
            if candidates.len() > MAX_DISCOVERED_REPOSITORIES {
                break;
            }
        }
    }
    candidates.sort();
    candidates.dedup();

    let mut repositories = Vec::new();
    for candidate in candidates {
        if let Some(repository) = inspect_repository(exec, &candidate).await? {
            if !repositories
                .iter()
                .any(|known: &RepositoryIdentity| known.root == repository.root)
            {
                repositories.push(repository);
            }
        }
        if repositories.len() == MAX_DISCOVERED_REPOSITORIES {
            break;
        }
    }
    repositories.sort_by(|left, right| left.root.cmp(&right.root));
    Ok(repositories)
}

async fn default_branch(exec: &dyn Executor, root: &Path) -> Result<Option<String>, String> {
    if let Some(remote_head) = git_optional(
        exec,
        root,
        "git symbolic-ref --quiet --short refs/remotes/origin/HEAD",
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
        let command = format!("git show-ref --verify --quiet refs/heads/{candidate}");
        if git_succeeds(exec, root, &command).await? {
            return Ok(Some(candidate.to_string()));
        }
    }
    Ok(None)
}

async fn repository_remotes(
    exec: &dyn Executor,
    root: &Path,
) -> Result<Vec<RepositoryRemote>, String> {
    let raw = git_optional(exec, root, "git config --get-regexp '^remote\\..*\\.url$'")
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

async fn git_optional(
    exec: &dyn Executor,
    root: &Path,
    command: &str,
) -> Result<Option<String>, String> {
    let output = run(exec, root, command).await?;
    if output.code != Some(0) {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

async fn git_required(exec: &dyn Executor, root: &Path, command: &str) -> Result<String, String> {
    let output = run(exec, root, command).await?;
    if output.code != Some(0) {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn git_succeeds(exec: &dyn Executor, root: &Path, command: &str) -> Result<bool, String> {
    Ok(run(exec, root, command).await?.code == Some(0))
}

async fn run(
    exec: &dyn Executor,
    root: &Path,
    command: &str,
) -> Result<exec_core::ExecOutput, String> {
    exec.exec(command, root, GIT_TIMEOUT, &CancellationToken::new())
        .await
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
mod tests {
    use super::*;
    use crate::exec::LocalExecutor;

    async fn git(root: &Path, command: &str) {
        let output = tokio::process::Command::new("git")
            .args(command.split_whitespace())
            .current_dir(root)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn identifies_clone_equivalent_repository_by_remote() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), "init").await;
        git(dir.path(), "config user.name Clark").await;
        git(dir.path(), "config user.email clark@example.com").await;
        tokio::fs::write(dir.path().join("README.md"), "hello")
            .await
            .unwrap();
        git(dir.path(), "add README.md").await;
        git(dir.path(), "commit -m initial").await;
        git(
            dir.path(),
            "remote add origin git@github.com:Clark-Labs-Inc/Clark.git",
        )
        .await;

        let identity = inspect_repository(&LocalExecutor, dir.path())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            identity.canonical_remote.as_deref(),
            Some("github.com/clark-labs-inc/clark")
        );
        assert!(identity.fingerprint.starts_with("git:"));
        assert_eq!(identity.commit_count, 1);
    }

    #[tokio::test]
    async fn history_is_paged_and_preserves_commit_metadata() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), "init").await;
        git(dir.path(), "config user.name Clark").await;
        git(dir.path(), "config user.email clark@example.com").await;
        for index in 0..3 {
            tokio::fs::write(dir.path().join("value.txt"), index.to_string())
                .await
                .unwrap();
            git(dir.path(), "add value.txt").await;
            git(dir.path(), &format!("commit -m commit-{index}")).await;
        }

        let first = load_git_history(&LocalExecutor, dir.path(), 0, 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.commits.len(), 2);
        assert!(!first.complete);
        assert_eq!(first.next_offset, 2);
        assert_eq!(first.commits[0].author_name, "Clark");
        let second = load_git_history(&LocalExecutor, dir.path(), first.next_offset, 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.commits.len(), 1);
        assert!(second.complete);
    }

    #[tokio::test]
    async fn discovers_nested_git_repositories() {
        let parent = tempfile::tempdir().unwrap();
        for name in ["one", "two"] {
            let root = parent.path().join(name);
            tokio::fs::create_dir_all(&root).await.unwrap();
            git(&root, "init").await;
            git(&root, "config user.name Clark").await;
            git(&root, "config user.email clark@example.com").await;
            tokio::fs::write(root.join("README.md"), name)
                .await
                .unwrap();
            git(&root, "add README.md").await;
            git(&root, "commit -m initial").await;
        }

        let repositories = discover_repositories(&LocalExecutor, parent.path())
            .await
            .unwrap();
        assert_eq!(repositories.len(), 2);
    }

    #[test]
    fn remote_sanitization_removes_credentials_and_normalizes_identity() {
        let (url, canonical) = sanitize_remote("https://token@example.com/Org/Repo.git").unwrap();
        assert!(!url.contains("token"));
        assert_eq!(canonical, "example.com/org/repo");
    }
}
