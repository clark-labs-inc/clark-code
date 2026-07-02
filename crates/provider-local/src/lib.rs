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
mod background;
mod browser_binary;
mod browser_cdp;
mod changes;
mod checkpoint;
mod claude_import;
mod commands;
mod compaction;
mod config;
mod engine;
mod exec;
mod files;
mod hooks;
mod llm;
mod loop_state;
mod markdown_frontmatter;
mod mcp;
mod memory;
mod permissions;
mod platform;
mod project_settings;
mod prompt;
mod provider;
mod safety;
mod sandbox;
mod tools;
mod workspace;

pub use changes::{changes_diff, changes_revert, changes_summary, ChangedFile};
pub use checkpoint::{create_checkpoint, is_git_repo, restore_checkpoint};
// Migrate an existing Claude Code setup: discover its MCP servers + skills.
pub use claude_import::{discover_mcp_servers, discover_skills, ClaudeSkill};
pub use commands::{discover_commands, CustomCommand};
pub use config::{LocalConfig, DEFAULT_BASE_URL, DEFAULT_RESEARCH_MODEL};
// The execution backends. `LocalExecutor` is wired today; `RemoteExecutor`
// (over clark-exec-server) is selected per session once remote projects land.
pub use exec::{Executor, LocalExecutor, RemoteExecutor};
pub use files::list_project_files;
pub use mcp::{probe_mcp_servers, McpServerConfig, McpStatus};
pub use memory::{
    global_memory_dir, load_facts, load_index, memory_dir, MemoryFact, MemoryHeader, MemoryType,
};
pub use platform::{personal_memory_section, recall_personal_memories, PersonalMemory};
pub use provider::LocalAgentProvider;
// The app-managed document workspace root, so the host can confine `read_doc_text`.
pub use workspace::workspace_root;
