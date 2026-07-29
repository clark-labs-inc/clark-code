use std::{path::Path, time::Duration};

use super::super::{git_output, git_output_with_timeout, repository_root};
use super::{
    types::{BaseResolution, SourceCheckout},
    ManagedWorktreeBase, ManagedWorktreeBaseOption, WorktreeChangeSummary,
};

const FRESH_DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) async fn source_checkout(project_path: &str) -> Result<SourceCheckout, String> {
    let root = repository_root(project_path).await?;
    let branch = git_output(
        &root,
        vec!["branch".into(), "--show-current".into()],
        "Read current branch",
    )
    .await?;
    let revision = checkout_revision(&root).await?;
    let changes = change_summary(&root).await?;
    Ok(SourceCheckout {
        root,
        branch: (!branch.is_empty()).then_some(branch),
        revision,
        changes,
    })
}

/// Resolve the commit currently checked out in a linked worktree. This is
/// intentionally separate from `revision_for`: a managed worktree is detached
/// and its private commits must be compared with its immutable base before any
/// cleanup decision is made.
pub(super) async fn checkout_revision(root: &Path) -> Result<String, String> {
    let revision = git_output(
        root,
        vec!["rev-parse".into(), "--verify".into(), "HEAD".into()],
        "Read worktree commit",
    )
    .await?;
    if !is_revision(&revision) {
        return Err("The selected checkout has no usable commit for a worktree.".into());
    }
    Ok(revision)
}

pub(super) async fn change_summary(root: &Path) -> Result<WorktreeChangeSummary, String> {
    let status = git_output(
        root,
        vec![
            "status".into(),
            "--porcelain=v1".into(),
            "--untracked-files=normal".into(),
        ],
        "Inspect working tree",
    )
    .await?;
    Ok(parse_change_summary(&status))
}

fn parse_change_summary(status: &str) -> WorktreeChangeSummary {
    let mut summary = WorktreeChangeSummary::default();
    for line in status.lines().filter(|line| line.len() >= 2) {
        let code = &line[..2];
        if code == "??" {
            summary.untracked_files += 1;
        } else if matches!(code, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU") {
            summary.conflicted_files += 1;
        } else {
            summary.changed_files += 1;
        }
    }
    summary
}

pub(super) async fn base_options(root: &Path) -> Result<Vec<ManagedWorktreeBaseOption>, String> {
    let current = resolve_current_base(root).await?;
    let default = resolve_default_base(root, &current).await;
    Ok([current, default]
        .into_iter()
        .map(|base| ManagedWorktreeBaseOption {
            id: base.base,
            label: base.label,
            reference: base.reference,
            revision: base.revision,
            fallback: base.fallback,
        })
        .collect())
}

pub(super) async fn resolve_base(
    root: &Path,
    base: ManagedWorktreeBase,
    target_branch: Option<&str>,
) -> Result<BaseResolution, String> {
    if let Some(target_branch) = target_branch
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
    {
        let Some(revision) = revision_for(root, target_branch).await else {
            return Err(format!("Local branch {target_branch} no longer exists."));
        };
        return Ok(BaseResolution {
            base,
            label: format!("Requested branch ({target_branch})"),
            reference: target_branch.to_string(),
            revision,
            fallback: false,
        });
    }
    let current = resolve_current_base(root).await?;
    match base {
        ManagedWorktreeBase::Current => Ok(current),
        // The picker may render before a network round-trip. Refresh only at
        // the moment Clark actually creates the isolated checkout, with a
        // short bounded fallback to the locally advertised default branch.
        ManagedWorktreeBase::Default => Ok(resolve_fresh_default_base(root, &current).await),
    }
}

async fn resolve_current_base(root: &Path) -> Result<BaseResolution, String> {
    let revision = checkout_revision(root).await?;
    let branch = git_output(
        root,
        vec!["branch".into(), "--show-current".into()],
        "Read current branch",
    )
    .await?;
    let reference = if branch.is_empty() {
        "HEAD".to_string()
    } else {
        branch
    };
    Ok(BaseResolution {
        base: ManagedWorktreeBase::Current,
        label: format!("Current checkout ({reference})"),
        reference,
        revision,
        fallback: false,
    })
}

async fn resolve_default_base(root: &Path, current: &BaseResolution) -> BaseResolution {
    let mut candidates = Vec::new();
    if let Ok(remote_head) = git_output(
        root,
        vec![
            "symbolic-ref".into(),
            "--quiet".into(),
            "--short".into(),
            "refs/remotes/origin/HEAD".into(),
        ],
        "Read default branch",
    )
    .await
    {
        if !remote_head.is_empty() {
            candidates.push(remote_head);
        }
    }
    candidates.extend(
        ["origin/main", "origin/master", "main", "master"]
            .into_iter()
            .map(str::to_string),
    );

    for candidate in candidates {
        if let Some(revision) = revision_for(root, &candidate).await {
            return BaseResolution {
                base: ManagedWorktreeBase::Default,
                label: format!("Default branch ({candidate})"),
                reference: candidate,
                revision,
                fallback: false,
            };
        }
    }

    BaseResolution {
        base: ManagedWorktreeBase::Default,
        label: "Default branch unavailable; use current checkout".into(),
        reference: current.reference.clone(),
        revision: current.revision.clone(),
        fallback: true,
    }
}

async fn resolve_fresh_default_base(root: &Path, current: &BaseResolution) -> BaseResolution {
    let fallback = resolve_default_base(root, current).await;
    let advertised = match git_output_with_timeout(
        root,
        vec![
            "ls-remote".into(),
            "--symref".into(),
            "origin".into(),
            "HEAD".into(),
        ],
        "Refresh default branch",
        FRESH_DEFAULT_TIMEOUT,
    )
    .await
    {
        Ok(output) => output,
        Err(_) => return fallback,
    };
    let Some((branch, revision)) = parse_remote_default(&advertised) else {
        return fallback;
    };

    // Fetch by immutable object ID: it downloads exactly the advertised
    // commit without moving the source checkout, its branch, FETCH_HEAD, or
    // local origin/<branch> tracking refs. Network failures retain the local
    // fallback rather than blocking a new session.
    if git_output_with_timeout(
        root,
        vec![
            "fetch".into(),
            "--quiet".into(),
            "--no-tags".into(),
            "--no-write-fetch-head".into(),
            "origin".into(),
            revision.clone().into(),
        ],
        "Fetch fresh default branch",
        FRESH_DEFAULT_TIMEOUT,
    )
    .await
    .is_err()
    {
        return fallback;
    }

    BaseResolution {
        base: ManagedWorktreeBase::Default,
        label: format!("Fresh default branch (origin/{branch})"),
        reference: format!("origin/{branch}"),
        revision,
        fallback: false,
    }
}

fn parse_remote_default(output: &str) -> Option<(String, String)> {
    let branch = output.lines().find_map(|line| {
        let reference = line.strip_prefix("ref: refs/heads/")?;
        let (branch, destination) = reference.split_once(char::is_whitespace)?;
        (destination.trim() == "HEAD" && !branch.trim().is_empty()).then(|| branch.to_string())
    })?;
    let revision = output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let revision = fields.next()?;
        let reference = fields.next()?;
        (reference == "HEAD" && is_revision(revision)).then(|| revision.to_string())
    })?;
    Some((branch, revision))
}

async fn revision_for(root: &Path, reference: &str) -> Option<String> {
    let revision = git_output(
        root,
        vec![
            "rev-parse".into(),
            "--verify".into(),
            format!("{reference}^{{commit}}").into(),
        ],
        "Resolve worktree base",
    )
    .await
    .ok()?;
    is_revision(&revision).then_some(revision)
}

fn is_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
