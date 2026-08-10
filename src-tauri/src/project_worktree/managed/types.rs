use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which immutable revision a new isolated session starts from.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedWorktreeBase {
    /// The selected checkout's current commit. This never fetches or modifies
    /// the selected checkout, including when it has uncommitted changes.
    #[default]
    Current,
    /// The repository's remote default branch, refreshed immediately before
    /// creation when it responds within a short deadline. Falls back to the
    /// locally advertised default branch, then current HEAD.
    Default,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorktreeRequest {
    #[serde(default)]
    pub base: ManagedWorktreeBase,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub target_branch: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeTransitionAction {
    /// Start a branch-backed managed checkout without touching the source checkout.
    CreateIsolated,
    /// The requested branch already has a checkout and must be opened there.
    OpenOwner,
    /// The source is clean and may be switched before a new isolated session.
    SwitchClean,
    /// Switching a dirty source would move changes across branches; preserve
    /// those changes in place and require the caller to choose deliberately.
    PreserveChanges,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreePreservation {
    Clean,
    ChangesRemainInSource,
    OwnerCheckout,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeChangeSummary {
    pub changed_files: u32,
    pub untracked_files: u32,
    pub conflicted_files: u32,
}

impl WorktreeChangeSummary {
    pub(super) fn is_dirty(&self) -> bool {
        self.changed_files > 0 || self.untracked_files > 0 || self.conflicted_files > 0
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorktreeBaseOption {
    pub id: ManagedWorktreeBase,
    pub label: String,
    pub reference: String,
    pub revision: String,
    pub fallback: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWorktreeTransitionPlan {
    pub source_root: String,
    pub source_branch: Option<String>,
    pub source_revision: String,
    pub source_changes: WorktreeChangeSummary,
    pub source_is_managed: bool,
    pub target_branch: Option<String>,
    pub target_checkout_path: Option<String>,
    pub action: WorktreeTransitionAction,
    pub preservation: WorktreePreservation,
    pub requires_confirmation: bool,
    pub base_options: Vec<ManagedWorktreeBaseOption>,
    pub managed_location: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedWorktreeState {
    /// No local changes and no commits beyond the immutable session base.
    Ready,
    /// Local changes still need to be committed, moved, or removed by the user.
    Dirty,
    /// The checkout contains commits that are not protected by its registered
    /// branch. Removing it would make those commits hard to recover.
    Committed,
    /// A named local branch protects the checkout's committed work. The
    /// checkout may now be archived after every live session has closed.
    Saved,
    Missing,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorktree {
    pub id: String,
    pub label: String,
    pub path: String,
    pub source_root: String,
    pub base: ManagedWorktreeBase,
    pub base_reference: String,
    pub base_revision: String,
    /// The checkout's current commit. Missing worktrees have no readable head
    /// revision.
    pub head_revision: Option<String>,
    /// A Clark Code-created branch that protects committed work before archival.
    pub preserved_branch: Option<String>,
    pub created_at_ms: u64,
    pub state: ManagedWorktreeState,
    pub changes: WorktreeChangeSummary,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorktreeCleanupReceipt {
    pub id: String,
    pub path: String,
    pub removed: bool,
}

/// Receipt for protecting a managed checkout's commits with a named local
/// branch. This is primarily a recovery path for legacy detached checkouts or
/// an externally detached managed branch.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorktreeBranchReceipt {
    pub id: String,
    pub path: String,
    pub branch: String,
    pub head_revision: String,
}

#[derive(Clone, Debug)]
pub(super) struct BaseResolution {
    pub(super) base: ManagedWorktreeBase,
    pub(super) label: String,
    pub(super) reference: String,
    pub(super) revision: String,
    pub(super) fallback: bool,
}

#[derive(Clone, Debug)]
pub(super) struct SourceCheckout {
    pub(super) root: PathBuf,
    pub(super) branch: Option<String>,
    pub(super) revision: String,
    pub(super) changes: WorktreeChangeSummary,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ManagedWorktreeRecord {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) path: String,
    pub(super) source_root: String,
    pub(super) base: ManagedWorktreeBase,
    pub(super) base_reference: String,
    pub(super) base_revision: String,
    /// Added without a registry version bump so existing lifecycle records are
    /// readable. An absent value means a legacy checkout has no branch
    /// protection yet.
    #[serde(default)]
    pub(super) preserved_branch: Option<String>,
    pub(super) created_at_ms: u64,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ManagedWorktreeRegistry {
    #[serde(default = "registry_version")]
    pub(super) version: u32,
    #[serde(default)]
    pub(super) entries: Vec<ManagedWorktreeRecord>,
}

pub(super) fn registry_version() -> u32 {
    1
}

pub(super) fn public_record(
    record: ManagedWorktreeRecord,
    state: ManagedWorktreeState,
    changes: WorktreeChangeSummary,
    head_revision: Option<String>,
) -> ManagedWorktree {
    ManagedWorktree {
        id: record.id,
        label: record.label,
        path: record.path,
        source_root: record.source_root,
        base: record.base,
        base_reference: record.base_reference,
        base_revision: record.base_revision,
        head_revision,
        preserved_branch: record.preserved_branch,
        created_at_ms: record.created_at_ms,
        state,
        changes,
    }
}
