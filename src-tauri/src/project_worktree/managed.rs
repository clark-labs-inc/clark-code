mod naming;
mod planning;
mod registry;
#[cfg(test)]
mod tests;
mod types;

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use tauri::State;

use crate::AppState;

use self::{
    naming::managed_identity,
    planning::{
        base_options, change_summary, checkout_revision, resolve_base, source_checkout,
        transition_decision,
    },
    registry::{
        acquire_registry_lock, attached_worktree_paths, common_git_dir,
        is_registered_managed_worktree, read_registry, registry_path, write_registry,
    },
    types::{public_record, ManagedWorktreeRecord},
};
use super::{git_output, local_branch_list, repository_root};

pub use types::{
    ManagedWorktree, ManagedWorktreeBase, ManagedWorktreeBaseOption, ManagedWorktreeBranchReceipt,
    ManagedWorktreeCleanupReceipt, ManagedWorktreeRequest, ManagedWorktreeState,
    ProjectWorktreeTransitionPlan, WorktreeChangeSummary, WorktreePreservation,
    WorktreeTransitionAction,
};

/// Build, but do not execute, the safe next Git/worktree action. The plan is
/// useful for both branch-picker routing and new-session isolation: no branch
/// switch, stash, commit, cleanup, or worktree creation happens here.
#[tauri::command]
pub async fn project_worktree_transition_plan(
    project_path: String,
    target_branch: Option<String>,
) -> Result<ProjectWorktreeTransitionPlan, String> {
    let source = source_checkout(&project_path).await?;
    let managed_location = managed_root(&source.root)?;
    let source_is_managed = is_registered_managed_worktree(&source.root).await?;
    let target_branch = target_branch
        .as_deref()
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(str::to_string);

    let target_checkout_path = if let Some(target_branch) = target_branch.as_deref() {
        let branches = local_branch_list(&source.root.to_string_lossy()).await?;
        let Some(target) = branches
            .iter()
            .find(|candidate| candidate.name == target_branch)
        else {
            return Err(format!("Local branch {target_branch} no longer exists."));
        };
        match target.checkout_path.as_deref() {
            Some(owner) => {
                let owner_path = Path::new(owner).canonicalize().map_err(|_| {
                    format!(
                        "Branch {target_branch} is registered to unavailable checkout {owner}. Resolve that Git worktree record before switching."
                    )
                })?;
                (owner_path != source.root).then_some(owner.to_string())
            }
            None => None,
        }
    } else {
        None
    };
    // A named target was looked up above (and missing targets returned an
    // error); the pure decision function still accepts this fact explicitly
    // so simulations can exercise stale-branch races without Git.
    let target_exists = true;
    let decision = transition_decision(
        &source.root.to_string_lossy(),
        source.branch.as_deref(),
        source_is_managed,
        source.changes.is_dirty(),
        target_branch.as_deref(),
        target_exists,
        target_checkout_path.as_deref(),
    )?;

    let base_options = if decision.action == WorktreeTransitionAction::OpenOwner {
        Vec::new()
    } else {
        base_options(&source.root).await?
    };

    Ok(ProjectWorktreeTransitionPlan {
        source_root: source.root.to_string_lossy().into_owned(),
        source_branch: source.branch,
        source_revision: source.revision,
        source_changes: source.changes,
        source_is_managed,
        target_branch,
        target_checkout_path: decision.target_checkout_path,
        action: decision.action,
        preservation: decision.preservation,
        requires_confirmation: decision.requires_confirmation,
        base_options,
        managed_location: managed_location.to_string_lossy().into_owned(),
    })
}

/// Create a branch-backed, app-managed worktree from an explicit base choice.
/// The source checkout is never switched, stashed, committed, reset, or
/// cleaned. Each checkout receives a unique `agent/<id>` branch so commits are
/// durable even if the checkout is later archived.
#[tauri::command]
pub async fn project_managed_worktree_create(
    project_path: String,
    request: ManagedWorktreeRequest,
) -> Result<ManagedWorktree, String> {
    let source = source_checkout(&project_path).await?;
    if is_registered_managed_worktree(&source.root).await? {
        return Err(
            "This checkout is already a app-managed isolated worktree. Reuse it instead of nesting another checkout."
                .into(),
        );
    }
    let base = resolve_base(&source.root, request.base, request.target_branch.as_deref()).await?;
    let (id, label) = managed_identity(&source.root, &base.reference, request.label.as_deref())?;
    let branch = managed_branch_name(&id);
    let destination = managed_root(&source.root)?.join(&id);
    if destination.exists() {
        return Err(format!(
            "A managed worktree already exists at {}. Try again.",
            destination.display()
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "The managed-worktree location is invalid.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Create managed-worktree folder {}: {error}",
            parent.display()
        )
    })?;

    git_output(
        &source.root,
        vec![
            "worktree".into(),
            "add".into(),
            "-b".into(),
            branch.clone().into(),
            destination.as_os_str().to_os_string(),
            base.revision.clone().into(),
        ],
        "Create isolated worktree",
    )
    .await?;
    let destination = destination.canonicalize().map_err(|error| {
        format!("Created isolated worktree, but its path is unavailable: {error}")
    })?;
    let head_revision = base.revision.clone();
    let record = ManagedWorktreeRecord {
        id,
        label,
        path: destination.to_string_lossy().into_owned(),
        source_root: source.root.to_string_lossy().into_owned(),
        base: base.base,
        base_reference: base.reference,
        base_revision: base.revision,
        preserved_branch: Some(branch),
        created_at_ms: unix_time_ms(),
    };
    let registry_path = registry_path(&common_git_dir(&source.root).await?);
    let registration = async {
        let _registry_lock = acquire_registry_lock(&registry_path).await?;
        let mut registry = read_registry(&registry_path)?;
        registry.entries.push(record.clone());
        write_registry(&registry_path, &registry)
    }
    .await;
    if let Err(error) = registration {
        return Err(format!(
            "Created isolated worktree at {}, but could not record its managed lifecycle: {error}. It was left untouched.",
            destination.display()
        ));
    }

    Ok(public_record(
        record,
        ManagedWorktreeState::Ready,
        WorktreeChangeSummary::default(),
        Some(head_revision),
    ))
}

/// List only worktrees created by Agent Desktop's managed lifecycle. User-created
/// linked worktrees remain visible through normal Git branch ownership but are
/// never candidates for lifecycle cleanup here.
#[tauri::command]
pub async fn project_managed_worktree_list(
    project_path: String,
) -> Result<Vec<ManagedWorktree>, String> {
    let root = repository_root(&project_path).await?;
    let registry_path = registry_path(&common_git_dir(&root).await?);
    let entries = {
        let _registry_lock = acquire_registry_lock(&registry_path).await?;
        read_registry(&registry_path)?.entries
    };
    let attached = attached_worktree_paths(&root).await?;
    let mut worktrees = Vec::with_capacity(entries.len());

    for record in entries {
        let path = PathBuf::from(&record.path);
        let Ok(canonical) = path.canonicalize() else {
            worktrees.push(public_record(
                record,
                ManagedWorktreeState::Missing,
                WorktreeChangeSummary::default(),
                None,
            ));
            continue;
        };
        if !attached.iter().any(|candidate| candidate == &canonical) {
            worktrees.push(public_record(
                record,
                ManagedWorktreeState::Missing,
                WorktreeChangeSummary::default(),
                None,
            ));
            continue;
        }
        let status = managed_status(&root, &canonical, &record).await?;
        worktrees.push(public_record(
            record,
            status.state,
            status.changes,
            Some(status.head_revision),
        ));
    }
    worktrees.sort_by_key(|worktree| std::cmp::Reverse(worktree.created_at_ms));
    Ok(worktrees)
}

/// Explicitly remove one clean, registered managed worktree. This intentionally
/// omits --force, never prunes arbitrary Git worktrees, and refuses any path
/// not present in both Agent Desktop's registry and Git's current worktree list.
#[tauri::command]
pub async fn project_managed_worktree_cleanup(
    project_path: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<ManagedWorktreeCleanupReceipt, String> {
    cleanup_managed_worktree(&project_path, &id, state.inner()).await
}

/// The native command above owns the live-session safety boundary. This core
/// operation stays separately testable so lifecycle tests exercise the exact
/// cleanup gate without constructing a Tauri command context.
pub(super) async fn cleanup_managed_worktree(
    project_path: &str,
    id: &str,
    state: &AppState,
) -> Result<ManagedWorktreeCleanupReceipt, String> {
    let root = repository_root(project_path).await?;
    let registry_path = registry_path(&common_git_dir(&root).await?);
    let _registry_lock = acquire_registry_lock(&registry_path).await?;
    let mut registry = read_registry(&registry_path)?;
    let id = id.trim();
    let Some(index) = registry.entries.iter().position(|entry| entry.id == id) else {
        return Err("That managed worktree is not registered for this repository.".into());
    };
    let record = registry.entries[index].clone();
    let path = PathBuf::from(&record.path).canonicalize().map_err(|_| {
        format!(
            "Managed worktree {} is already missing. It was not removed or forgotten automatically.",
            record.path
        )
    })?;
    let attached = attached_worktree_paths(&root).await?;
    if !attached.iter().any(|candidate| candidate == &path) {
        return Err(format!(
            "Managed worktree {} is no longer attached to this repository. It was left untouched.",
            path.display()
        ));
    }
    let live_sessions = live_sessions_using_checkout(state, &path).await;
    if !live_sessions.is_empty() {
        return Err(format!(
            "Managed worktree {} is still used by {} live desktop session{}. Archive or close that chat before archiving this checkout.",
            path.display(),
            live_sessions.len(),
            if live_sessions.len() == 1 { "" } else { "s" }
        ));
    }
    let status = managed_status(&root, &path, &record).await?;
    if status.changes.is_dirty() {
        return Err(format!(
            "Managed worktree {} has local changes ({} changed, {} untracked, {} conflicted). Commit, move, or remove them before cleanup.",
            path.display(),
            status.changes.changed_files,
            status.changes.untracked_files,
            status.changes.conflicted_files
        ));
    }
    if status.state == ManagedWorktreeState::Committed {
        let branch = managed_branch_name(&record.id);
        return Err(format!(
            "Managed worktree {} has new commits that are not protected by a branch. Save them as {branch} before archiving this checkout.",
            path.display()
        ));
    }

    git_output(
        &root,
        vec![
            "worktree".into(),
            "remove".into(),
            path.as_os_str().to_os_string(),
        ],
        "Remove managed worktree",
    )
    .await?;
    registry.entries.remove(index);
    write_registry(&registry_path, &registry)?;

    Ok(ManagedWorktreeCleanupReceipt {
        id: record.id,
        path: path.to_string_lossy().into_owned(),
        removed: true,
    })
}

/// Protect a managed worktree's unprotected commits with a stable local branch.
/// Normal managed worktrees are branch-backed from creation; this command is a
/// recovery path for legacy detached checkouts or an externally detached
/// branch, and it never moves an existing branch.
#[tauri::command]
pub async fn project_managed_worktree_save_branch(
    project_path: String,
    id: String,
) -> Result<ManagedWorktreeBranchReceipt, String> {
    let root = repository_root(&project_path).await?;
    let registry_path = registry_path(&common_git_dir(&root).await?);
    let _registry_lock = acquire_registry_lock(&registry_path).await?;
    let mut registry = read_registry(&registry_path)?;
    let id = id.trim();
    let Some(index) = registry.entries.iter().position(|entry| entry.id == id) else {
        return Err("That managed worktree is not registered for this repository.".into());
    };
    let mut record = registry.entries[index].clone();
    let path = PathBuf::from(&record.path).canonicalize().map_err(|_| {
        format!(
            "Managed worktree {} is already missing. Its commits were not modified.",
            record.path
        )
    })?;
    let attached = attached_worktree_paths(&root).await?;
    if !attached.iter().any(|candidate| candidate == &path) {
        return Err(format!(
            "Managed worktree {} is no longer attached to this repository. Its commits were left untouched.",
            path.display()
        ));
    }
    let changes = change_summary(&path).await?;
    if changes.is_dirty() {
        return Err(format!(
            "Managed worktree {} still has local changes. Commit, move, or remove them before saving its commits as a branch.",
            path.display()
        ));
    }
    let head_revision = checkout_revision(&path).await?;
    if head_revision == record.base_revision {
        return Err("This managed worktree has no new commits to save as a branch.".into());
    }

    let branch = match record.preserved_branch.as_deref() {
        Some(candidate) => match branch_revision(&root, candidate).await {
            Some(existing) if existing == head_revision => candidate.to_string(),
            Some(_) => recovery_branch_name(&record.id),
            None => candidate.to_string(),
        },
        None => managed_branch_name(&record.id),
    };
    if let Some(existing) = branch_revision(&root, &branch).await {
        if existing != head_revision {
            return Err(format!(
                "Branch {branch} already points somewhere else, so Agent Desktop will not overwrite it. Create or choose another branch manually."
            ));
        }
    } else {
        git_output(
            &root,
            vec![
                "branch".into(),
                branch.clone().into(),
                head_revision.clone().into(),
            ],
            "Save managed worktree commits as a branch",
        )
        .await?;
    }

    record.preserved_branch = Some(branch.clone());
    registry.entries[index] = record;
    write_registry(&registry_path, &registry)?;
    Ok(ManagedWorktreeBranchReceipt {
        id: id.to_string(),
        path: path.to_string_lossy().into_owned(),
        branch,
        head_revision,
    })
}

struct ManagedWorktreeStatus {
    state: ManagedWorktreeState,
    changes: WorktreeChangeSummary,
    head_revision: String,
}

async fn managed_status(
    repo_root: &Path,
    checkout: &Path,
    record: &ManagedWorktreeRecord,
) -> Result<ManagedWorktreeStatus, String> {
    let changes = change_summary(checkout).await?;
    let head_revision = checkout_revision(checkout).await?;
    let state = if changes.is_dirty() {
        ManagedWorktreeState::Dirty
    } else if head_revision == record.base_revision {
        ManagedWorktreeState::Ready
    } else if let Some(branch) = record.preserved_branch.as_deref() {
        if branch_revision(repo_root, branch)
            .await
            .is_some_and(|revision| revision == head_revision)
        {
            ManagedWorktreeState::Saved
        } else {
            ManagedWorktreeState::Committed
        }
    } else {
        ManagedWorktreeState::Committed
    };
    Ok(ManagedWorktreeStatus {
        state,
        changes,
        head_revision,
    })
}

fn managed_branch_name(id: &str) -> String {
    // `id` is normalized to an ASCII-only path/ref segment before it reaches
    // this boundary. Never derive a Git ref from raw UI input here.
    format!("agent/{id}")
}

fn recovery_branch_name(id: &str) -> String {
    format!("{}-saved", managed_branch_name(id))
}

async fn branch_revision(repo_root: &Path, branch: &str) -> Option<String> {
    let revision = git_output(
        repo_root,
        vec![
            "rev-parse".into(),
            "--verify".into(),
            format!("refs/heads/{branch}^{{commit}}").into(),
        ],
        "Read saved managed-worktree branch",
    )
    .await
    .ok()?;
    (!revision.is_empty()).then_some(revision)
}

async fn live_sessions_using_checkout(state: &AppState, checkout: &Path) -> Vec<String> {
    let sessions = state.runtime_registry.session_entries().await;
    let mut matches = Vec::new();
    for (id, entry) in sessions {
        let checkout_root = entry
            .lock()
            .await
            .session
            .environment
            .as_ref()
            .and_then(|environment| environment.checkout_root.as_deref())
            .map(str::to_owned);
        if checkout_root
            .as_deref()
            .and_then(|root| PathBuf::from(root).canonicalize().ok())
            .is_some_and(|root| root == checkout)
        {
            matches.push(id.as_str().to_string());
        }
    }
    matches
}

fn managed_root(repo_root: &Path) -> Result<PathBuf, String> {
    let parent = repo_root
        .parent()
        .ok_or_else(|| "The repository has no parent folder for managed worktrees.".to_string())?;
    let repo_name = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "The repository folder has no usable name.".to_string())?;
    Ok(parent.join(format!("{repo_name}.agent-worktrees")))
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
