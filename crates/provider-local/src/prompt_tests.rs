use super::*;

#[test]
fn includes_root_and_research_note_when_available() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(
        &sb,
        true,
        false,
        Some(crate::project_settings::DEFAULT_COMMIT_ATTRIBUTION),
        Some(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION),
    );
    assert!(p.contains("Project root:"));
    assert!(p.contains("product-brokered research capability"));
    assert!(p.find("# Instruction boundaries").unwrap() < p.find("# Git").unwrap());
    assert!(
        p.find("# External knowledge and research").unwrap() < p.find("# Communication").unwrap()
    );
    assert!(p.find("# Communication").unwrap() < p.find("# Git").unwrap());
    assert!(p.contains("`sandbox_permissions` set to `require_escalated`"));
    assert!(p.contains("do not browse sibling projects for examples"));
    assert!(p.contains("Clark Code will ask for a scoped approval"));
    assert!(p.contains("Plan Mode is read-only and cannot request escalation"));
    assert!(p.contains("final `# User request`"));
    assert!(
        p.contains("call `deliver_files` with every deliverable path before the final response")
    );
    assert!(p.contains("file-manager reveal"));
    assert!(p.contains("Finish each completed turn with an ordinary assistant response"));
    assert!(!p.contains("`final_answer`"));
}

#[test]
fn configured_research_is_brokered_first_with_web_fetch_only_as_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(
        &sb,
        true,
        false,
        Some(crate::project_settings::DEFAULT_COMMIT_ATTRIBUTION),
        Some(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION),
    );

    assert!(p.contains("product-brokered research capability"));
    assert!(p.contains("Discover it with `tool_search`"));
    assert!(p.contains("Make `tool_search` the only tool call in that turn"));
    assert!(p.contains("wait for the next model call before using any capability it activates"));
    assert!(p.contains("Do not call `web_fetch` while brokered research is running"));
    assert!(p.contains("Use `web_fetch` only after brokered research explicitly fails"));
    assert!(p.contains("current upstream state"));
    assert!(p.contains("Never switch to `bash`, `curl`, or `wget`"));
    assert!(p.find("# External knowledge and research").unwrap() < p.find("# Behavior").unwrap());
}

#[test]
fn pins_milestone_narration_and_tool_backed_claims() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(
        &sb,
        false,
        false,
        Some(crate::project_settings::DEFAULT_COMMIT_ATTRIBUTION),
        Some(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION),
    );

    assert!(p.contains("Before the first non-trivial tool batch"));
    assert!(p.contains("what changed or was found"));
    assert!(p.contains("not categorical sections"));
    assert!(p.contains("Do not narrate routine reads"));
    assert!(p.contains("same assistant response"));
    assert!(p.contains("matching tool-call evidence"));
    assert!(p.contains("without narration markup tags"));
}

#[test]
fn omits_research_note_when_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(
        &sb,
        false,
        false,
        Some(crate::project_settings::DEFAULT_COMMIT_ATTRIBUTION),
        Some(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION),
    );
    assert!(!p.contains("product-brokered research capability"));
    assert!(p.contains("Cloud research is not configured"));
    assert!(p.contains("activate `web_fetch`"));
    assert!(p.contains("cannot perform broad search or reliable multi-source synthesis"));
    assert!(p.contains("Never fetch URLs through `bash`, `curl`, or `wget`"));
}

#[test]
fn remote_prompt_keeps_tools_and_setup_on_the_ssh_host() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(
        &sb,
        false,
        true,
        Some(crate::project_settings::DEFAULT_COMMIT_ATTRIBUTION),
        Some(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION),
    );
    assert!(p.contains("SSH-connected remote computer"));
    assert!(p.contains("Android emulator"));
    assert!(p.contains("intentionally unavailable"));
    assert!(p.contains("Never fall back to the desktop machine"));
    assert!(p.contains("# Interactive authentication"));
    assert!(p.contains("aws sso login --profile <profile> --use-device-code --no-browser"));
    assert!(p.contains("Poll it with `bash_output`"));
    assert!(p.contains("open the URL in a browser on their desktop and enter the code"));
    assert!(p.contains("run the login command on the SSH-connected computer"));
    assert!(p.contains("A missing remote browser is not a reason to abandon device authentication"));
    assert!(!p.contains("operating directly on the user's local machine"));
}

#[test]
fn includes_planning_guidance() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(
        &sb,
        false,
        false,
        Some(crate::project_settings::DEFAULT_COMMIT_ATTRIBUTION),
        Some(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION),
    );
    assert!(p.contains("update_plan"));
    // Plan Mode is discoverable from the stable prompt (both entry points).
    assert!(p.contains("enter_plan_mode"));
    assert!(p.contains("proposed_plan"));
    assert!(p.contains("tool_search"));
}

#[test]
fn implementation_guidance_requires_an_early_working_diff() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(&sb, false, false, None, None);

    assert!(p.contains("make a working diff"));
    assert!(p.contains("Before generating an edit, identify the target"));
    assert!(!p.contains("After at most eight"));
    assert!(p.contains("Do not spend most of the run planning with no edits"));
    assert!(p.contains("run the smallest decisive check"));
}

#[test]
fn explicit_goal_request_precedes_implementation() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(
        &sb,
        false,
        false,
        Some(crate::project_settings::DEFAULT_COMMIT_ATTRIBUTION),
        Some(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION),
    );
    assert!(p.contains("call `create_goal` before any implementation tool"));
    assert!(p.contains("without the requested goal lifecycle"));
}

#[test]
fn includes_shared_tree_and_audience_guidance() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(
        &sb,
        false,
        false,
        Some(crate::project_settings::DEFAULT_COMMIT_ATTRIBUTION),
        Some(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION),
    );
    // Non-engineer audience + material, repository-first clarification.
    assert!(p.contains("# Working with the user"));
    assert!(p.contains("ONE short clarifying question"));
    assert!(p.contains("Resolve ambiguity with read-only inspection first"));
    assert!(p.contains("do not make assumption-dependent changes"));
    assert!(p.contains("state the assumption briefly and proceed without asking"));
    // Shared-tree git rules: no stash/reset, foreign changes are off-limits.
    assert!(p.contains("Every branch you create must start with `agent/`"));
    assert!(p.contains("`git stash`"));
    assert!(p.contains("checkout latest main"));
    assert!(p.contains("git worktree list --porcelain"));
    assert!(p.contains("never use `--ignore-other-worktrees`"));
    assert!(p.contains("changes you did not create"));
    assert!(p.contains("Keep the repository's configured human author"));
    assert!(p.contains("Co-Authored-By: Local Agent <noreply@localhost>"));
    assert!(p.contains("## Creating pull requests"));
    assert!(p.contains("Code written by Local Agent"));
    assert!(!p.to_ascii_lowercase().contains("codex"));
    // Test-quality bar: at least one would-fail case.
    assert!(p.contains("# Testing"));
    assert!(p.contains("would fail if your change were broken"));
    // Judgment: serve intent, stop on dead premises, cause vs. symptom.
    assert!(p.contains("# Judgment"));
    assert!(p.contains("serve the intent"));
    assert!(p.contains("fixes the cause or only hides the symptom"));
    assert!(p.contains("perform one audit"));
    assert!(p.contains("absence of a known failure does not prove completion"));
    assert!(p.contains("stop using tools and answer immediately"));
    assert!(p.contains("An available tool or deferred capability is not unfinished work"));
    assert!(p.contains("Repeating a final answer, apology, or stop acknowledgment is not progress"));
    assert!(p.contains("Preserve existing tests as independent contracts"));
    assert!(p.contains("public names, shapes, status values, identifiers"));
    // Hard rules keep the primacy slot: # Git before every other section.
    let git = p.find("# Git").unwrap();
    assert!(git < p.find("# Working with the user").unwrap());
    assert!(git < p.find("# Judgment").unwrap());
    assert!(git < p.find("# Behavior").unwrap());
}

#[test]
fn opening_build_request_clarifies_step_by_step_before_building() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(&sb, false, false, None, None);

    // Opening build requests stay in ordinary conversation: interview first, then build.
    assert!(p.contains("Opening a conversation with a request to build something"));
    assert!(p.contains("Clarify step by step"));
    assert!(p.contains("one short question at a time"));
    assert!(p.contains("recommended default so the user can reply in a word"));
    assert!(
        p.contains("skip the interview entirely for small, precise, or self-contained requests")
    );
    assert!(p.contains("build immediately without re-interviewing"));
    // The single-question rule is explicitly scoped to later turns so it
    // cannot be read as overriding the opening interview.
    let opening = p
        .find("Opening a conversation with a request to build something")
        .unwrap();
    let later = p
        .find("On later turns: ask at most ONE short clarifying question")
        .unwrap();
    assert!(opening < later);
}

#[test]
fn commit_workflow_matches_claude_customization_and_opt_out() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let custom = "Co-Authored-By: Custom Agent <agent@example.com>";
    let p = system_prompt(
        &sb,
        false,
        false,
        Some(custom),
        Some(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION),
    );
    assert!(p.contains("## Creating commits"));
    assert!(p.contains("git status"));
    assert!(p.contains("write_file` to a temporary repository-relative file"));
    assert!(p.contains("git commit -F <path>"));
    assert!(p.contains("Do not embed commit text in shell quoting, heredocs, here-strings, or command substitution."));
    assert!(!p.contains("git commit -m \"$(cat <<'EOF'"));
    assert!(!p.contains("git commit -m @'"));
    assert_eq!(p.matches(custom).count(), 1);
    assert!(p.contains("Never skip hooks or signing checks"));
    // PR-body attribution section is present with the default note.
    assert!(p.contains("## Creating pull requests"));
    assert!(p.contains(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION));

    let disabled = system_prompt(
        &sb,
        false,
        false,
        Some(""),
        Some(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION),
    );
    assert!(disabled.contains("## Creating commits"));
    assert!(!disabled.contains("Co-Authored-By:"));
    // Commit attribution disabled, but PR-body note still present.
    assert!(disabled.contains("## Creating pull requests"));
    assert!(disabled.contains(crate::project_settings::DEFAULT_PR_BODY_ATTRIBUTION));

    let hidden = system_prompt(&sb, false, false, None, None);
    assert!(!hidden.contains("## Creating commits"));
    assert!(!hidden.contains("git commit -F <path>"));
    assert!(!hidden.contains("## Creating pull requests"));
}

#[test]
fn output_style_instructions_are_empty_for_default_and_unknown() {
    assert_eq!(output_style_instructions("default"), "");
    assert_eq!(output_style_instructions("nonexistent"), "");
    let terse = output_style_instructions("terse");
    assert!(terse.contains("Terse"));
    assert!(terse.contains("never shortens durable"));
    assert!(terse.contains("validation evidence"));
}

#[test]
fn evidence_and_dependency_rules_precede_completion_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(dir.path()).unwrap();
    let p = system_prompt(&sb, false, false, None, None);
    assert!(p.contains("wait for that result in the next model turn"));
    assert!(p.find("# Durable and external effects").unwrap() < p.find("# Completion").unwrap());
    assert!(
        p.find("Batch only independent tool calls").unwrap()
            < p.find("Finish each completed turn with an ordinary assistant response")
                .unwrap()
    );
}
