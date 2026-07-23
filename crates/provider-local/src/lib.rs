//! Local coding-agent provider — clark-desktop's coding mode.
//!
//! The agent loop runs **locally**: the model reasons over the production Clark
//! Platform API (`https://api.clarkslabs.com/v1`, OpenAI-compatible, `ck_live_`
//! key) and the tool calls execute on the user's own machine and codebase.
//! Research (`clark_research`) routes through the same agentic Platform API +
//! key — Clark runs web search / browsing server-side and returns findings.
//! Durable memory (project + global) is curated by the agent via the `memory`
//! tool. The model sees one flat tool list;
//! execution routes to local or remote backends behind a uniform
//! [`tools::ToolExecutor`] trait.
//!
//! It implements the same [`agent_core::Provider`] trait as every other backend,
//! so the UI and projection layer are unchanged.

mod agent_adapter;
mod attachments;
mod background;
mod browser_binary;
mod browser_cdp;
mod changes;
mod checkpoint;
mod commands;
mod compaction;
mod config;
mod effects;
mod engine;
mod exec;
mod external_import;
mod files;
mod git_metadata;
mod hooks;
mod incidents;
mod instructions;
mod llm;
mod loop_breaker;
mod loop_state;
mod markdown_frontmatter;
mod mcp;
mod memory;
mod memory_extraction;
mod multi_repo_provider;
mod multi_repo_workspace;
mod orchestration;
mod permissions;
mod planning;
mod platform;
mod project_settings;
mod prompt;
mod provider;
mod repository;
mod resume;
mod root_execution;
mod safety;
mod sandbox;
mod skills;
mod tools;
mod truncation;
mod workspace;

pub use changes::{changes_diff, changes_revert, changes_summary, ChangedFile};
pub use checkpoint::{create_checkpoint, is_git_repo, release_checkpoints};
// Discover compatible setup from other coding agents without mutating it.
pub use commands::{discover_commands, CustomCommand};
pub use config::{LocalConfig, DEFAULT_BASE_URL, DEFAULT_RESEARCH_MODEL};
pub use external_import::{
    discover_agent_setups, discover_agent_setups_with_home, AgentMigrationDiscovery,
    MigratedInstruction, MigratedSkill, MigrationSource,
};
// The execution backends. `LocalExecutor` is wired today; `RemoteExecutor`
// (over clark-exec-server) is selected per session once remote projects land.
pub use exec::{Executor, LocalExecutor, RemoteExecutor};
pub use files::list_project_files;
pub use instructions::{
    load as discover_instructions, InstructionOrigin, InstructionProvenance, InstructionScope,
    ProjectInstructions,
};
pub use mcp::{probe_mcp_servers, McpServerConfig, McpStatus};
pub use memory::{
    global_memory_dir, load_facts, load_index, memory_dir, MemoryFact, MemoryHeader, MemoryType,
};
pub use multi_repo_provider::{
    ClarkCloudWriterConfig, ClarkCloudWriterHarness, IntegrationReadinessGate,
    LocalIntegrationHarness, LocalMultiRepoRuntime, LocalMultiRepoRuntimeConfig,
    LocalReaderHarness, LocalReviewHarness, LocalWriterHarness,
};
pub use multi_repo_workspace::{
    FreshIntegrationWorkspace, IsolatedReaderWorkspace, IsolatedWriterWorkspace,
    PrimaryApplicationReceipt, RepositorySelection, RepositorySelectionRequest, SelectedRepository,
};
pub use orchestration::{local_read_only_harness, WorkspaceDigestGuard};
pub use platform::{
    personal_memory_section, recall_personal_memories, recall_repository_context,
    repository_context_section, scope_personal_memories, PersonalMemory, RepositoryCommitContext,
    RepositoryContext,
};
pub use provider::{local_sandbox_setup_policy, LocalAgentProvider};
pub use repository::{
    discover_repositories, inspect_repository, load_git_history, GitCommitEvidence,
    GitHistoryBatch, RepositoryIdentity, RepositoryRemote,
};
pub use skills::{
    discover_skill_catalog_snapshot, install_skill_pack, list_skill_packs, skill_environment_id,
    uninstall_skill_pack, InstallSkillPackRequest, InstalledSkillPack, SkillCatalogEntry,
    SkillCatalogService, SkillCatalogSnapshot, SkillDiagnostic, SkillDiagnosticSeverity,
    SkillOrigin, SkillPackAction, SkillPackReceipt, SkillPackScope, SkillScope,
};
// The app-managed document workspace root, so the host can confine `read_doc_text`.
pub use workspace::workspace_root;
