use super::*;
use crate::exec::LocalExecutor;

#[test]
fn paths_are_under_dot_clark() {
    let root = Path::new("/proj");
    assert_eq!(memory_dir(root), Path::new("/proj/.clark/memory"));
}

#[test]
fn parses_frontmatter_fields() {
    let text = "---\nname: build-cmd\ndescription: how to build\nsaved: 2026-07-01\nsource: user-stated\ntype: project\n---\n\nUse cargo.";
    let fm = parse_frontmatter(text);
    assert_eq!(fm.name.as_deref(), Some("build-cmd"));
    assert_eq!(fm.description.as_deref(), Some("how to build"));
    assert_eq!(fm.kind, Some(MemoryType::Project));
    assert_eq!(fm.saved.as_deref(), Some("2026-07-01"));
    assert_eq!(fm.source.as_deref(), Some("user-stated"));
}

#[test]
fn civil_date_math_round_trips() {
    // days_from_civil and iso_date_today are inverses around known dates.
    assert_eq!(days_from_civil(1970, 1, 1), Some(0));
    assert_eq!(days_from_civil(2026, 7, 15), Some(20_649));
    let today = iso_date_today();
    assert_eq!(days_since_iso_date(&today), Some(0));
}

#[test]
fn slugify_kebabs_and_trims() {
    assert_eq!(slugify("Build & Test commands!"), "build-test-commands");
    assert_eq!(slugify("  Hello   World  "), "hello-world");
    assert_eq!(slugify("***"), "");
}

#[tokio::test]
async fn long_single_line_content_keeps_frontmatter_on_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let mem = memory_dir(dir.path());
    let exec = LocalExecutor;
    // A single 400-char line used to blow past the description cap and get
    // a "\n… [truncated]" suffix injected INSIDE the frontmatter block.
    let long = "PawPal waitlist project details ".repeat(13);
    let file = save_memory(
        &exec,
        &mem,
        "Long note",
        &long,
        Some(MemoryType::Project),
        None,
    )
    .await
    .unwrap();
    let text = std::fs::read_to_string(mem.join(&file)).unwrap();
    let desc_line = text
        .lines()
        .find(|l| l.starts_with("description: "))
        .expect("description line");
    assert!(desc_line.chars().count() <= 160 + "description: ".len());
    assert!(!text.contains("[truncated]"), "{text}");
    // The type line must survive as parseable frontmatter.
    let fm = parse_frontmatter(&text);
    assert!(fm.description.is_some());
    assert_eq!(fm.kind, Some(MemoryType::Project));
}

#[tokio::test]
async fn save_then_recall_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let mem = memory_dir(dir.path());
    let exec = LocalExecutor;
    let file = save_memory(
        &exec,
        &mem,
        "Build command",
        "Run `cargo build` from the repo root.",
        Some(MemoryType::Project),
        Some("user-stated"),
    )
    .await
    .unwrap();
    assert_eq!(file, "build-command.md");

    // Index created + points at the fact.
    let index = load_index(&exec, &mem).await.unwrap();
    assert!(index.contains("build-command.md"));

    // Fact readable, frontmatter stripped from the body; saved/source stamped.
    let facts = load_facts(&exec, &mem).await;
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].header.name.as_deref(), Some("Build command"));
    assert_eq!(facts[0].header.kind, Some(MemoryType::Project));
    assert_eq!(facts[0].header.saved.as_deref(), Some(&*iso_date_today()));
    assert_eq!(facts[0].header.source.as_deref(), Some("user-stated"));
    assert!(facts[0].body.contains("cargo build"));
    assert!(!facts[0].body.contains("description:"));

    // Recall bundles index + bodies with an age stamp; listing likewise.
    let recall = recall_scope(&exec, &mem, "Project", None).await.unwrap();
    assert!(recall.contains("cargo build"));
    assert!(recall.contains("saved today"));
    let listing = scope_listing(&exec, &mem, "Project", None).await.unwrap();
    assert!(listing.contains("Build command"));
    assert!(listing.contains("saved today"));
}

#[tokio::test]
async fn empty_scope_yields_none() {
    let dir = tempfile::tempdir().unwrap();
    let exec = LocalExecutor;
    assert!(
        scope_listing(&exec, &memory_dir(dir.path()), "Project", None)
            .await
            .is_none()
    );
    assert!(load_index(&exec, &memory_dir(dir.path())).await.is_none());
}

#[tokio::test]
async fn listing_restates_decisions_imperatively() {
    let dir = tempfile::tempdir().unwrap();
    let mem = memory_dir(dir.path());
    let exec = LocalExecutor;
    save_memory(
        &exec,
        &mem,
        "Brand vocabulary",
        "Customers are called 'members'. Never write 'customers'.",
        Some(MemoryType::Project),
        None,
    )
    .await
    .unwrap();
    save_memory(
        &exec,
        &mem,
        "Build command",
        "Run `cargo build`.",
        None,
        None,
    )
    .await
    .unwrap();

    let listing = scope_listing(&exec, &mem, "Project", None).await.unwrap();
    assert!(listing.contains("Standing decisions"), "{listing}");
    // The rule-shaped note is restated; the plain code fact isn't.
    let decisions = listing.split("Standing decisions").nth(1).unwrap();
    assert!(decisions.contains("members"), "{listing}");
    assert!(!decisions.contains("cargo build"), "{listing}");
}

#[tokio::test]
async fn forget_removes_note_and_index_line() {
    let dir = tempfile::tempdir().unwrap();
    let mem = memory_dir(dir.path());
    let exec = LocalExecutor;
    save_memory(
        &exec,
        &mem,
        "Brand vocabulary",
        "Customers are 'owners'.",
        None,
        None,
    )
    .await
    .unwrap();
    save_memory(&exec, &mem, "Deploy command", "Use ship.sh.", None, None)
        .await
        .unwrap();

    let removed = delete_memory(&exec, &mem, "brand vocabulary")
        .await
        .unwrap();
    assert_eq!(removed.as_deref(), Some("brand-vocabulary.md"));
    assert!(!mem.join("brand-vocabulary.md").exists());
    // Index keeps the surviving note, drops the removed one.
    let index = load_index(&exec, &mem).await.unwrap();
    assert!(!index.contains("brand-vocabulary.md"));
    assert!(index.contains("deploy-command.md"));
    // No match → Ok(None), not an error.
    assert!(delete_memory(&exec, &mem, "nonexistent thing")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn recall_flags_missing_paths_and_scripts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::write(root.join("lib/app.js"), "export {};\n").unwrap();
    std::fs::write(
        root.join("package.json"),
        "{\"scripts\":{\"test\":\"node --test\"}}",
    )
    .unwrap();
    let mem = memory_dir(root);
    let exec = LocalExecutor;
    save_memory(
        &exec,
        &mem,
        "Entrypoint",
        "The entrypoint is `src/main.js`; run `npm run unit` to test. See lib/app.js too.",
        None,
        None,
    )
    .await
    .unwrap();

    let recall = recall_scope(&exec, &mem, "Project", Some(root))
        .await
        .unwrap();
    // Missing path and unknown script get flagged; existing path doesn't.
    assert!(recall.contains("⚠"), "{recall}");
    assert!(recall.contains("src/main.js"), "{recall}");
    assert!(recall.contains("npm run unit"), "{recall}");
    assert!(
        !recall.contains("`lib/app.js`, which does not exist"),
        "{recall}"
    );
}
