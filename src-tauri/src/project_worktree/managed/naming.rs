use std::path::Path;

use uuid::Uuid;

use super::super::validate_name;

const MAX_STEM_LEN: usize = 36;

pub(super) fn managed_identity(
    repo_root: &Path,
    base_reference: &str,
    requested_label: Option<&str>,
) -> Result<(String, String), String> {
    let label = match requested_label {
        Some(value) => validate_name(value)?.to_string(),
        None => automatic_label(repo_root, base_reference),
    };
    let stem = &label[..label.len().min(MAX_STEM_LEN)];
    let suffix = Uuid::new_v4().simple().to_string();
    Ok((format!("{stem}-{}", &suffix[..8]), label))
}

fn automatic_label(repo_root: &Path, base_reference: &str) -> String {
    let repo = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(slug)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "repo".to_string());
    let branch = branch_label(base_reference);
    compact_pair(&repo, &branch, MAX_STEM_LEN)
}

fn branch_label(reference: &str) -> String {
    let reference = reference
        .strip_prefix("refs/heads/")
        .or_else(|| reference.strip_prefix("refs/remotes/"))
        .unwrap_or(reference);
    let reference = reference.strip_prefix("origin/").unwrap_or(reference);
    let branch = slug(reference);
    if branch.is_empty() {
        "head".to_string()
    } else {
        branch
    }
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    let mut pending_separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !result.is_empty() {
                result.push('-');
            }
            result.push(character.to_ascii_lowercase());
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }
    result
}

fn compact_pair(repo: &str, branch: &str, limit: usize) -> String {
    if repo.len() + branch.len() + 1 <= limit {
        return format!("{repo}-{branch}");
    }
    let repo_budget = ((limit - 1) / 2).min(repo.len());
    let branch_budget = (limit - 1 - repo_budget).min(branch.len());
    let repo_budget = (limit - 1 - branch_budget).min(repo.len());
    format!("{}-{}", &repo[..repo_budget], &branch[..branch_budget])
}

#[cfg(test)]
mod tests {
    use super::{automatic_label, branch_label, compact_pair, MAX_STEM_LEN};
    use std::path::Path;

    #[test]
    fn automatic_names_identify_repository_and_branch() {
        assert_eq!(
            automatic_label(Path::new("/src/Clark Desktop"), "feature/worktree-names"),
            "clark-desktop-feature-worktree-names"
        );
        assert_eq!(branch_label("origin/main"), "main");
        assert_eq!(branch_label("refs/heads/release/v2"), "release-v2");
    }

    #[test]
    fn long_names_keep_recognizable_parts_of_both_inputs() {
        let label = compact_pair(
            "extremely-long-repository-name",
            "extremely-long-feature-branch-name",
            MAX_STEM_LEN,
        );
        assert_eq!(label.len(), MAX_STEM_LEN);
        assert!(label.starts_with("extremely-long-re"));
        assert!(label.ends_with("extremely-long-fea"));
    }
}
