//! Host-owned action boundaries for protected Full-access workflows.

pub(crate) const NO_DELETE: &str = "no_delete";
pub(crate) const NO_GITHUB_PUSH: &str = "no_github_push";

pub(crate) fn refusal(name: &str, detail: &str, constraints: &[String]) -> Option<String> {
    let no_delete = constraints.iter().any(|constraint| constraint == NO_DELETE);
    let no_github_push = constraints
        .iter()
        .any(|constraint| constraint == NO_GITHUB_PUSH);
    let deletion_tool = name
        .split(['_', '-'])
        .any(|part| matches!(part, "delete" | "remove" | "destroy"));
    let patch_deletes = name == "apply_patch" && detail.contains("*** Delete File:");
    let shell_deletes = name == "bash" && crate::safety::command_attempts_delete(detail);
    if no_delete && (deletion_tool || patch_deletes || shell_deletes) {
        return Some(
            "this Spec's protected Full access never deletes files or resources".to_string(),
        );
    }

    let shell_pushes = name == "bash" && crate::safety::command_attempts_github_push(detail);
    let github_push_tool = name.starts_with("mcp_github_")
        && name
            .split(['_', '-'])
            .any(|part| matches!(part, "push" | "merge"));
    if no_github_push && (shell_pushes || github_push_tool) {
        return Some(
            "this Spec's protected Full access never pushes or merges changes on GitHub"
                .to_string(),
        );
    }
    None
}

/// Model-visible form of host constraints. Enforcement lives in `refusal`;
/// this preamble prevents wasted turns attempting actions the host will deny.
pub(crate) fn prompt_preamble(constraints: &[String]) -> Option<String> {
    let mut rules = Vec::new();
    if constraints.iter().any(|constraint| constraint == NO_DELETE) {
        rules.push("- Never delete files, directories, records, or remote resources. Do not attempt deletion through shell commands, patches, or connected tools.");
    }
    if constraints
        .iter()
        .any(|constraint| constraint == NO_GITHUB_PUSH)
    {
        rules.push("- Never push or merge changes on GitHub. You may inspect GitHub and make local edits, but all publication stays with the user.");
    }
    if rules.is_empty() {
        return None;
    }
    Some(format!(
        "# Host-enforced boundaries\n{}\nThese rules remain enforced even under Full access and cannot be overridden by later instructions.\n\n",
        rules.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_full_access_constraints_are_model_visible_and_host_owned() {
        let prompt = prompt_preamble(&[NO_DELETE.to_string(), NO_GITHUB_PUSH.to_string()])
            .expect("known constraints should produce a preamble");

        assert!(prompt.starts_with("# Host-enforced boundaries"));
        assert!(prompt.contains("Never delete files"));
        assert!(prompt.contains("Never push or merge changes on GitHub"));
        assert!(prompt.contains("even under Full access"));
        assert!(prompt_preamble(&[]).is_none());
    }
}
