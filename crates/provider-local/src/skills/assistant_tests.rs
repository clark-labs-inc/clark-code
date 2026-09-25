use super::invokes_skill;
use super::loader::discover_catalog_with_home;
use crate::exec::LocalExecutor;
use agent_core::domain::ContentBlock;
use std::collections::HashSet;

#[tokio::test]
async fn security_assistant_is_available_without_scan_contracts() {
    let temp = tempfile::tempdir().unwrap();
    let mut catalog = discover_catalog_with_home(&LocalExecutor, temp.path(), None).await;
    catalog.resolve_capabilities(&HashSet::new(), &[]);
    let assistant = catalog.resolve_name("security:assistant").unwrap();
    assert!(!assistant.allow_implicit_invocation);
    let body = catalog.read(&LocalExecutor, assistant).await.unwrap();
    assert!(body.contains("ordinary agent tools"));
    assert!(body.contains("research_rank"));
    assert!(catalog.resolve_name("security:security-scan").is_err());
    let blocks = vec![ContentBlock::text(
        "$security:assistant explain this design",
    )];
    assert!(invokes_skill(
        &catalog,
        &blocks,
        "$security:assistant explain this design",
        "security:assistant"
    ));
    assert!(!invokes_skill(
        &catalog,
        &blocks,
        "$security:assistant explain this design",
        "security:security-scan"
    ));
}
