use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::exec::LocalExecutor;

use super::loader::discover_catalog_with_home;
use super::{
    bound_skill_injections, explicit_skill_injections, install_skill_pack, invokes_skill,
    render_catalog, uninstall_skill_pack, InstallSkillPackRequest, SkillOrigin, SkillPackAction,
    SkillPackScope, SkillScope,
};
use agent_core::domain::ContentBlock;

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("parent")).unwrap();
    fs::write(path, contents).unwrap();
}

fn write_skill(root: &Path, directory: &str, name: &str, description: &str, body: &str) {
    write(
        &root.join(directory).join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: \"{description}\"\n---\n\n{body}\n"),
    );
}

#[tokio::test]
async fn recursively_discovers_project_skills_and_ignores_hidden_system_skills() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let home = temp.path().join("home");
    write_skill(
        &project.join(".clark/skills"),
        "release/nested",
        "release",
        "Ship: verify the real boundary",
        "Project release body.",
    );
    write_skill(
        &home.join(".codex/skills"),
        ".system/skill-creator",
        "skill-creator",
        "Internal system skill",
        "Do not import.",
    );

    let catalog = discover_catalog_with_home(&LocalExecutor, &project, Some(&home)).await;
    let release = catalog.resolve_name("release").unwrap();
    assert_eq!(release.scope, SkillScope::Project);
    assert_eq!(release.origin, SkillOrigin::Clark);
    assert_eq!(release.description, "Ship: verify the real boundary");
    assert!(catalog.resolve_name("skill-creator").is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn follows_directory_symlinks_with_canonical_identity_and_cycle_protection() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let shared = temp.path().join("superpowers/skills");
    write_skill(
        &shared,
        "brainstorming",
        "brainstorming",
        "Explore requirements before implementation",
        "Ask one question at a time.",
    );
    fs::create_dir_all(project.join(".agents/skills")).unwrap();
    std::os::unix::fs::symlink(&shared, project.join(".agents/skills/superpowers")).unwrap();
    std::os::unix::fs::symlink(&shared, shared.join("loop")).unwrap();

    let catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    let matches = catalog
        .skills
        .iter()
        .filter(|skill| skill.base_name == "brainstorming")
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].locator(),
        shared
            .join("brainstorming/SKILL.md")
            .canonicalize()
            .unwrap()
            .to_string_lossy()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn ignores_symlinked_skill_files_with_an_actionable_diagnostic() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let shared = temp.path().join("shared/SKILL.md");
    write(
        &shared,
        "---\nname: linked\ndescription: Linked file\n---\n\nBody.\n",
    );
    fs::create_dir_all(project.join(".agents/skills/linked")).unwrap();
    std::os::unix::fs::symlink(&shared, project.join(".agents/skills/linked/SKILL.md")).unwrap();

    let catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    assert!(catalog.resolve_name("linked").is_err());
    assert!(catalog.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "symlinked_skill_file"
            && diagnostic.message.contains("link the containing directory")
    }));
}

#[tokio::test]
async fn repairs_common_unquoted_colon_descriptions() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    write(
        &project.join(".agents/skills/aws/SKILL.md"),
        "---\nname: aws_review\ndescription: Review AWS: ECS changes\n---\n\nBody.\n",
    );

    let catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    let skill = catalog.resolve_name("aws_review").unwrap();
    assert_eq!(skill.description, "Review AWS: ECS changes");
}

#[tokio::test]
async fn preserves_name_collisions_with_exact_source_qualified_invocations() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let home = temp.path().join("home");
    write_skill(
        &project.join(".agents/skills"),
        "review",
        "review",
        "Project review",
        "Use project rules.",
    );
    write_skill(
        &home.join(".claude/skills"),
        "review",
        "review",
        "User review",
        "Use user rules.",
    );

    let catalog = discover_catalog_with_home(&LocalExecutor, &project, Some(&home)).await;
    assert!(catalog
        .resolve_name("review")
        .unwrap_err()
        .contains("ambiguous"));
    let project_review = catalog.resolve_name("project:compatible:review").unwrap();
    let user_review = catalog.resolve_name("user:claude:review").unwrap();
    assert_eq!(project_review.description, "Project review");
    assert_eq!(project_review.scope, SkillScope::Project);
    assert_eq!(user_review.description, "User review");
    assert_ne!(project_review.id, user_review.id);
    assert!(project_review.has_name_collision);
    assert!(user_review.has_name_collision);
}

#[tokio::test]
async fn plugin_manifest_namespaces_skills_and_rejects_escape_paths() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let plugin = project.join(".clark/plugins/acme");
    write(
        &plugin.join(".clark-plugin/plugin.json"),
        r#"{"name":"acme","skills":["./skills","../outside"]}"#,
    );
    write_skill(
        &plugin.join("skills"),
        "review",
        "review",
        "Review Acme changes",
        "Acme review body.",
    );

    let catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    let skill = catalog.resolve_name("acme:review").unwrap();
    assert_eq!(skill.base_name, "review");
    assert_eq!(skill.origin, SkillOrigin::Plugin);
    assert!(catalog
        .warnings
        .iter()
        .any(|warning| warning.contains("outside the plugin root")));
}

#[tokio::test]
async fn metadata_dependencies_and_explicit_only_policy_are_enforced() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let skill_dir = project.join(".clark/skills/deploy");
    write_skill(
        &project.join(".clark/skills"),
        "deploy",
        "deploy",
        "Deploy a service",
        "Deployment body.",
    );
    write(
        &skill_dir.join("agents/openai.yaml"),
        "dependencies:\n  tools:\n    - type: tool\n      value: deploy_service\npolicy:\n  allow_implicit_invocation: false\n",
    );

    let mut catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    catalog.resolve_capabilities(&HashSet::from(["bash".to_string()]), &[]);
    assert!(catalog.resolve_name("deploy").is_err());

    catalog.resolve_capabilities(
        &HashSet::from(["bash".to_string(), "deploy_service".to_string()]),
        &[],
    );
    assert!(catalog.resolve_name("deploy").is_ok());
    assert!(!render_catalog(&catalog).unwrap().contains("`deploy`"));
}

#[tokio::test]
async fn invalid_skill_metadata_is_a_structured_catalog_error() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let skill_dir = project.join(".clark/skills/review");
    write_skill(
        &project.join(".clark/skills"),
        "review",
        "review",
        "Review changes",
        "Body.",
    );
    write(&skill_dir.join("agents/openai.yaml"), "dependencies: [\n");

    let catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    assert!(catalog.resolve_name("review").is_err());
    assert!(catalog.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "invalid_skill"
            && diagnostic.message.contains("invalid agents/openai.yaml")
    }));
}

#[tokio::test]
async fn explicit_mentions_inject_complete_bundled_skill_once() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let mut catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    catalog.resolve_capabilities(&HashSet::from(["bash".to_string()]), &[]);

    let sections = explicit_skill_injections(
        &LocalExecutor,
        &catalog,
        "Use $github:gh-fix-ci and then $github:gh-fix-ci.",
    )
    .await;
    assert_eq!(sections.len(), 1);
    assert!(sections[0].contains("Diagnose and fix GitHub Actions failures"));
    assert!(sections[0].contains("inspection or diagnosis request remains read-only"));
}

#[tokio::test]
async fn qualified_bundled_skill_can_be_disabled_without_hiding_its_siblings() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let mut catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    catalog.resolve_capabilities(
        &HashSet::from(["bash".to_string()]),
        &["github:yeet".to_string()],
    );

    assert!(catalog.resolve_name("github:yeet").is_err());
    assert!(catalog.resolve_name("github:gh-fix-ci").is_ok());
}

#[tokio::test]
async fn bundled_sentry_skill_is_read_only_and_resolves_through_its_alias() {
    let temp = tempfile::tempdir().unwrap();
    let mut catalog = discover_catalog_with_home(&LocalExecutor, temp.path(), None).await;
    catalog.resolve_capabilities(&HashSet::from(["bash".to_string()]), &[]);

    let sentry = catalog.resolve_name("sentry").unwrap();
    assert_eq!(sentry.name, "sentry:sentry");
    let body = catalog.read(&LocalExecutor, sentry).await.unwrap();
    assert!(body.contains("Use only Sentry GET endpoints"));
    assert!(body.contains("Never ask them to paste the token into chat"));
    assert!(!body.contains("TODO"));
}

#[tokio::test]
async fn bundled_scout_skill_requires_the_typed_evidence_toolchain() {
    let temp = tempfile::tempdir().unwrap();
    let mut catalog = discover_catalog_with_home(&LocalExecutor, temp.path(), None).await;
    catalog.resolve_capabilities(&HashSet::from(["bash".to_string()]), &[]);
    assert!(catalog.resolve_name("scout").is_err());

    catalog.resolve_capabilities(
        &HashSet::from([
            "scout_capabilities".to_string(),
            "scout_adapter".to_string(),
            "scout_ledger".to_string(),
            "scout_enterprise".to_string(),
            "scout_enterprise_query".to_string(),
            "scout_probe".to_string(),
            "scout_measure".to_string(),
            "delegate_read_only".to_string(),
            "resolve_delegation".to_string(),
        ]),
        &[],
    );
    let scout = catalog.resolve_name("scout").unwrap();
    assert_eq!(scout.name, "scout:scout");
    assert!(
        !scout.allow_implicit_invocation,
        "Scout must be explicitly selected so its host-pinned model applies before the first turn"
    );
    assert!(invokes_skill(
        &catalog,
        &[ContentBlock::text("$scout:scout map AWS")],
        "$scout:scout map AWS",
        "scout:scout",
    ));
    assert!(invokes_skill(
        &catalog,
        &[ContentBlock::skill_reference(
            &scout.id,
            &scout.revision,
            "Scout",
        )],
        "map AWS",
        "scout:scout",
    ));
    assert!(!invokes_skill(
        &catalog,
        &[ContentBlock::text("$github:github inspect AWS")],
        "$github:github inspect AWS",
        "scout:scout",
    ));
    let body = catalog.read(&LocalExecutor, scout).await.unwrap();
    assert!(body.contains("Scout's model is not user-configurable"));
    assert!(body.contains("Never print, return, hash, or persist secret values"));
    assert!(body.contains("Call `scout_capabilities`"));
    assert!(body.contains("Call `scout_enterprise\n   enroll`"));
    assert!(body.contains("`scout_enterprise submit_adapter_receipt`"));
    assert!(body.contains("with only that retained `task_id`\n   and `receipt_id`"));
    assert!(body.contains("Concurrent collectors share nothing directly"));
    assert!(body.contains("Private key bytes never enter tool"));
    assert!(body.contains("Warm reads report an index receipt"));
    assert!(body.contains("Use `scout_enterprise_query snapshot`"));
    assert!(body.contains("Workers propose"));
    assert!(body.contains("independently checked reproduction"));
    assert!(body.contains("Exhaust the declared business-system graph, not the host filesystem"));
    assert!(body.contains("Stop only when every frontier row is terminal"));
    assert!(body.contains("The simulation model must name business actors"));
}

#[tokio::test]
async fn bundled_security_skill_requires_its_contract_tool_and_explicit_selection() {
    let temp = tempfile::tempdir().unwrap();
    let mut catalog = discover_catalog_with_home(&LocalExecutor, temp.path(), None).await;
    let common = HashSet::from([
        "read_file".to_string(),
        "grep".to_string(),
        "glob".to_string(),
        "bash".to_string(),
    ]);
    catalog.resolve_capabilities(&common, &[]);
    assert!(catalog.resolve_name("security-scan").is_err());

    let mut tools = common;
    tools.insert("security_scan_contract".to_string());
    tools.insert("security_poc_execute".to_string());
    catalog.resolve_capabilities(&tools, &[]);
    let security = catalog.resolve_name("security-scan").unwrap();
    let security_diff = catalog.resolve_name("security-diff").unwrap();
    assert_eq!(security.name, "security:security-scan");
    assert_eq!(security_diff.name, "security:security-diff");
    assert!(!security.allow_implicit_invocation);
    assert!(!security_diff.allow_implicit_invocation);
    assert!(invokes_skill(
        &catalog,
        &[ContentBlock::text("$security:security-scan scan this repo")],
        "$security:security-scan scan this repo",
        "security:security-scan",
    ));
    let body = catalog.read(&LocalExecutor, security).await.unwrap();
    assert!(body.contains("Never claim a clean or completed scan"));
    assert!(body.contains("source → nearest control → sink or broken control → impact"));
    assert!(body.contains("exact `z-ai/glm-5.2` production model"));
    assert!(body.contains("positive control"));
    assert!(body.contains("host-issued receipt ids"));
    let diff_body = catalog.read(&LocalExecutor, security_diff).await.unwrap();
    assert!(diff_body.contains("staged, unstaged, untracked, renamed, and deleted"));
    assert!(diff_body.contains("Every candidate must touch at least one changed"));
    assert!(diff_body.contains("exact `z-ai/glm-5.2` production model"));

    tools.insert("delegate_read_only".to_string());
    tools.insert("resolve_delegation".to_string());
    catalog.resolve_capabilities(&tools, &[]);
    let security_deep = catalog.resolve_name("security-deep").unwrap();
    assert_eq!(security_deep.name, "security:security-deep");
    assert!(!security_deep.allow_implicit_invocation);
    let deep_body = catalog.read(&LocalExecutor, security_deep).await.unwrap();
    assert!(deep_body.contains("explicitly authorizes bounded read-only delegation"));
    assert!(deep_body.contains("two consecutive passes that add no new candidate ids"));
    assert!(deep_body.contains("exact `z-ai/glm-5.2` production model"));
}

#[test]
fn bundled_security_diff_openai_metadata_matches_runtime_dependency() {
    let metadata: serde_yaml::Value = serde_yaml::from_str(include_str!(
        "../../skills/security/security-diff/agents/openai.yaml"
    ))
    .unwrap();
    let tools = metadata["dependencies"]["tools"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|tool| tool["value"].as_str().unwrap())
        .collect::<HashSet<_>>();
    assert!(tools.contains("security_scan_contract"));
    assert!(tools.contains("security_poc_execute"));
    assert_eq!(
        metadata["policy"]["allow_implicit_invocation"].as_bool(),
        Some(false)
    );
}

#[test]
fn bundled_scout_openai_metadata_matches_runtime_dependencies() {
    let metadata: serde_yaml::Value =
        serde_yaml::from_str(include_str!("../../skills/scout/agents/openai.yaml")).unwrap();
    let tools = metadata["dependencies"]["tools"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|tool| tool["value"].as_str().unwrap())
        .collect::<HashSet<_>>();
    assert_eq!(
        tools,
        HashSet::from([
            "scout_capabilities",
            "scout_adapter",
            "scout_ledger",
            "scout_enterprise",
            "scout_enterprise_query",
            "scout_probe",
            "scout_measure",
            "delegate_read_only",
            "resolve_delegation",
        ])
    );
}

#[tokio::test]
async fn skill_resources_are_relative_text_and_cannot_escape_or_follow_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let skill_dir = project.join(".clark/skills/review");
    write_skill(
        &project.join(".clark/skills"),
        "review",
        "review",
        "Review with a reference",
        "Read references/checklist.md.",
    );
    write(
        &skill_dir.join("references/checklist.md"),
        "Verify the real boundary.",
    );
    write(&project.join("secret.md"), "outside");

    let catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    let skill = catalog.resolve_name("review").unwrap();
    assert_eq!(
        catalog
            .read_resource(&LocalExecutor, skill, Some("references/checklist.md"))
            .await
            .unwrap(),
        "Verify the real boundary."
    );
    assert!(catalog
        .read_resource(&LocalExecutor, skill, Some("../../../secret.md"))
        .await
        .unwrap_err()
        .contains("relative path"));

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            project.join("secret.md"),
            skill_dir.join("references/leak.md"),
        )
        .unwrap();
        assert!(catalog
            .read_resource(&LocalExecutor, skill, Some("references/leak.md"))
            .await
            .unwrap_err()
            .contains("refuses symlink"));
    }
}

#[tokio::test]
async fn catalog_is_bounded_and_describes_clark_authority() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    for index in 0..200 {
        write_skill(
            &project.join(".clark/skills"),
            &format!("skill-{index}"),
            &format!("skill-{index}"),
            &"long description ".repeat(30),
            "Body.",
        );
    }
    let mut catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    catalog.resolve_capabilities(&HashSet::from(["bash".to_string()]), &[]);
    let rendered = render_catalog(&catalog).unwrap();
    assert!(rendered.len() < 8 * 1024 + 200);
    assert!(rendered.contains("not extra authority"));
    assert!(rendered.contains("additional skill(s) omitted"));
}

#[tokio::test]
async fn managed_pack_install_update_restart_binding_and_uninstall_are_exact() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let source = project.join("fixtures/read-millar-superpowers");
    write_skill(
        &source.join("skills"),
        "brainstorming",
        "brainstorming",
        "Explore requirements before implementation",
        "Version one body.",
    );

    let installed = install_skill_pack(
        &LocalExecutor,
        &project,
        InstallSkillPackRequest {
            pack_id: "superpowers".into(),
            source_path: source.to_string_lossy().into_owned(),
            scope: SkillPackScope::Project,
        },
    )
    .await
    .unwrap();
    assert_eq!(installed.action, SkillPackAction::Installed);
    assert_eq!(installed.skill_count, 1);

    // A fresh discovery stands in for an app restart: the atomic registry, not
    // process memory, determines the active revision.
    let first = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    let first_skill = first.resolve_name("brainstorming").unwrap();
    let first_id = first_skill.id.clone();
    let first_revision = first_skill.revision.clone();
    let bound = bound_skill_injections(
        &LocalExecutor,
        &first,
        &[ContentBlock::skill_reference(
            &first_id,
            &first_revision,
            "brainstorming",
        )],
    )
    .await
    .unwrap();
    assert_eq!(bound.len(), 1);
    assert!(bound[0].contains("Version one body."));

    write_skill(
        &source.join("skills"),
        "brainstorming",
        "brainstorming",
        "Explore requirements before implementation",
        "Version two body.",
    );
    let updated = install_skill_pack(
        &LocalExecutor,
        &project,
        InstallSkillPackRequest {
            pack_id: "superpowers".into(),
            source_path: source.to_string_lossy().into_owned(),
            scope: SkillPackScope::Project,
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.action, SkillPackAction::Updated);
    assert_eq!(
        updated.previous_revision.as_deref(),
        installed.revision.as_deref()
    );

    let second = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    let second_skill = second.resolve_name("brainstorming").unwrap();
    assert_eq!(second_skill.id, first_id);
    assert_ne!(second_skill.revision, first_revision);
    let stale = bound_skill_injections(
        &LocalExecutor,
        &second,
        &[ContentBlock::skill_reference(
            &first_id,
            &first_revision,
            "brainstorming",
        )],
    )
    .await
    .unwrap_err();
    assert!(stale.contains("changed from revision"));

    let removed = uninstall_skill_pack(
        &LocalExecutor,
        &project,
        "superpowers",
        SkillPackScope::Project,
    )
    .await
    .unwrap();
    assert_eq!(removed.action, SkillPackAction::Uninstalled);
    let after = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    assert!(after.resolve_name("brainstorming").is_err());
}

#[tokio::test]
#[ignore = "set CLARK_SUPERPOWERS_FIXTURE to exercise a real obra/superpowers checkout"]
async fn imports_the_real_superpowers_repository_layout() {
    let source = std::env::var("CLARK_SUPERPOWERS_FIXTURE")
        .expect("CLARK_SUPERPOWERS_FIXTURE must point to the repository root");
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    std::fs::create_dir_all(&project).unwrap();
    let receipt = install_skill_pack(
        &LocalExecutor,
        &project,
        InstallSkillPackRequest {
            pack_id: "superpowers".into(),
            source_path: source,
            scope: SkillPackScope::Project,
        },
    )
    .await
    .unwrap();
    assert_eq!(receipt.action, SkillPackAction::Installed);
    assert!(receipt.skill_count >= 10);

    let catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    let brainstorming = catalog.resolve_name("brainstorming").unwrap();
    let contents = catalog.read(&LocalExecutor, brainstorming).await.unwrap();
    assert!(contents.contains("brainstorm"));
    let reference = catalog
        .read_resource(&LocalExecutor, brainstorming, Some("visual-companion.md"))
        .await
        .unwrap();
    assert!(!reference.trim().is_empty());

    uninstall_skill_pack(
        &LocalExecutor,
        &project,
        "superpowers",
        SkillPackScope::Project,
    )
    .await
    .unwrap();
    let after = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    assert!(after.resolve_name("brainstorming").is_err());
}

#[tokio::test]
async fn model_visible_skill_text_hides_compatibility_storage_names() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let home = temp.path().join("home");
    write_skill(
        &home.join(".codex/skills"),
        "review",
        "review",
        "Review project changes",
        "Inspect the relevant diff.",
    );
    let mut catalog = discover_catalog_with_home(&LocalExecutor, &project, Some(&home)).await;
    catalog.resolve_capabilities(&HashSet::from(["bash".to_string()]), &[]);

    let catalog_text = render_catalog(&catalog).unwrap();
    assert!(!catalog_text.to_ascii_lowercase().contains("codex"));
    let skill = catalog.resolve_name("review").unwrap();
    let body = catalog.read(&LocalExecutor, skill).await.unwrap();
    let injection = super::render_injection(skill, &body);
    assert!(!injection.to_ascii_lowercase().contains("codex"));
}
