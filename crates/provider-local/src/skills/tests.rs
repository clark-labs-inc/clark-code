use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::exec::LocalExecutor;

use super::loader::discover_catalog_with_home;
use super::{explicit_skill_injections, render_catalog, SkillOrigin, SkillScope};

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
async fn project_skill_overrides_user_skill_with_the_same_name() {
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
    let review = catalog.resolve_name("review").unwrap();
    assert_eq!(review.description, "Project review");
    assert_eq!(review.scope, SkillScope::Project);
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
