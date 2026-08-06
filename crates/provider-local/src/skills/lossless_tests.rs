use std::collections::HashSet;

use crate::exec::LocalExecutor;

use super::loader::discover_catalog_with_home;

#[tokio::test]
async fn selected_skill_body_is_loaded_byte_for_byte() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("repo");
    let path = project.join(".clark/skills/large/SKILL.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let expected = format!(
        "---\nname: large\ndescription: Complete large skill\n---\n\nBEGIN:{}:END\n",
        "0123456789abcdef".repeat(8_000)
    );
    std::fs::write(&path, &expected).unwrap();

    let mut catalog = discover_catalog_with_home(&LocalExecutor, &project, None).await;
    catalog.resolve_capabilities(&HashSet::new(), &[]);
    let skill = catalog.resolve_name("large").unwrap();
    let actual = catalog.read(&LocalExecutor, skill).await.unwrap();

    assert_eq!(actual, expected);
}
