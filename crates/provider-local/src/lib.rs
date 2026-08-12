//! Product-extensible local coding-agent provider.
//!
//! The agent loop runs locally against an OpenAI-compatible model endpoint;
//! tool calls execute on the user's own machine and codebase. Products can add
//! brokered capabilities through the `ToolPack` extension point.
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
mod configuration;
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
mod runtime_plugins;
mod safety;
mod sandbox;
mod scout_policy;
pub mod security;
mod security_history;
pub mod scout_census {
    pub use scout_capability_census::*;
}
mod skills;
mod tools;
mod workspace;

pub use changes::{changes_diff, changes_revert, changes_summary, ChangedFile};
pub use checkpoint::{create_checkpoint, is_git_repo, release_checkpoints};
// Discover compatible setup from other coding agents without mutating it.
pub use commands::{discover_commands, CustomCommand};
pub use config::{LocalConfig, DEFAULT_BASE_URL, DEFAULT_MODEL, DEFAULT_RESEARCH_MODEL};
pub use configuration::defaults as configuration_capabilities;
pub use exec::{Executor, LocalExecutor};
pub use external_import::{
    discover_agent_setups, discover_agent_setups_with_home, AgentMigrationDiscovery,
    MigratedInstruction, MigratedSkill, MigrationSource,
};
pub use files::list_project_files;
pub use git_metadata::{git_working_tree_status, inspect_git_checkout, GitCheckoutContext};
pub use instructions::{
    load as discover_instructions, InstructionOrigin, InstructionProvenance, InstructionScope,
    ProjectInstructions,
};
pub use llm::LlmClient;
pub use mcp::{probe_mcp_servers, McpServerConfig, McpStatus};
pub use memory::{
    global_memory_dir, global_memory_dir_for_scope, load_facts, load_index, memory_dir, MemoryFact,
    MemoryHeader, MemoryType,
};
pub use multi_repo_provider::{
    BrokeredCloudWriterConfig, BrokeredCloudWriterHarness, IntegrationReadinessGate,
    LocalIntegrationHarness, LocalMultiRepoRuntime, LocalMultiRepoRuntimeConfig,
    LocalReaderHarness, LocalReviewHarness, LocalWriterHarness,
};
pub use multi_repo_workspace::{
    FreshIntegrationWorkspace, IsolatedReaderWorkspace, IsolatedWriterWorkspace,
    PrimaryApplicationReceipt, RepositorySelection, RepositorySelectionRequest, SelectedRepository,
};
pub use orchestration::{local_read_only_harness, WorkspaceDigestGuard};
#[doc(hidden)]
pub use planning::{complete_plan_markdown_for_eval, planning_prompt_contract_for_eval};
pub use platform::{
    feature_context_section, personal_memory_section, repository_context_section,
    scope_personal_memories, FeatureContextFeedbackReceipt, FeatureContextFeedbackRequest,
    FeatureContextGap, FeatureContextObject, FeatureContextObligation, FeatureContextPacket,
    FeatureContextQueryKind, FeatureContextRepository, FeatureContextRepositoryBinding,
    FeatureContextRequest, FeatureContextResponse, FeatureContextRevision,
    OrganizationKnowledgeHit, OrganizationKnowledgePacket, OrganizationKnowledgeResponse,
    PersonalMemory, PlatformContextProvider, RepositoryCommitContext, RepositoryContext,
};
pub use provider::{local_sandbox_setup_policy, LocalAgentProvider};
pub use repository::{
    discover_repositories, inspect_repository, load_git_history, GitCommitEvidence,
    GitHistoryBatch, RepositoryIdentity, RepositoryRemote,
};
pub use runtime_plugins::{
    RuntimeAgentEvent, RuntimeAgentMessage, RuntimeEventSink, RuntimeFollowUpSource, RuntimePlugin,
    RuntimePluginCapabilities, RuntimePluginPack, RuntimeSteeringSource, RuntimeUserContent,
};
pub use security_history::{list_security_scans, SecurityScanRecord};
pub use skills::{
    discover_skill_catalog_snapshot, install_skill_pack, list_skill_packs, skill_environment_id,
    uninstall_skill_pack, InstallSkillPackRequest, InstalledSkillPack, SkillCatalogEntry,
    SkillCatalogService, SkillCatalogSnapshot, SkillDiagnostic, SkillDiagnosticSeverity,
    SkillOrigin, SkillPackAction, SkillPackReceipt, SkillPackScope, SkillScope,
};
pub use tools::{
    PermissionScope, ToolCtx, ToolExecutor, ToolExposure, ToolOutcome, ToolPack,
    ToolPermissionClass, ToolPermissionDecision, ToolRegistry,
};
// The app-managed document workspace, so the host can confine artifact reads.
pub use workspace::{
    initialize_quick_chat_workspace, is_markdown, is_quick_chat_workspace, is_session_workspace,
    session_workspace, workspace_root,
};
