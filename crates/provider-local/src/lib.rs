//! Local coding-agent provider — clark-desktop's coding mode.
//!
//! The agent loop runs **locally**: the model reasons over the production Clark
//! Platform API (`https://api.clarkslabs.com/v1`, OpenAI-compatible, `ck_live_`
//! key) and the tool calls execute on the user's own machine and codebase.
//! Research (`clark_research`) and per-repo memory extraction route through the
//! same agentic Platform API + key — Clark runs web search / browsing
//! server-side and returns findings. The model sees one flat tool list;
//! execution routes to local or remote backends behind a uniform
//! [`tools::ToolExecutor`] trait.
//!
//! It implements the same [`agent_core::Provider`] trait as every other backend,
//! so the UI and projection layer are unchanged.

mod checkpoint;
mod config;
mod engine;
mod files;
mod llm;
mod mcp;
mod memory;
mod prompt;
mod provider;
mod safety;
mod sandbox;
mod tools;

pub use checkpoint::{is_git_repo, restore_checkpoint};
pub use config::{LocalConfig, DEFAULT_BASE_URL, DEFAULT_RESEARCH_MODEL};
pub use files::list_project_files;
pub use mcp::{probe_mcp_servers, McpServerConfig, McpStatus};
pub use memory::{
    extract_repo_memory, has_memory, load_facts, load_index, memory_dir, MemoryFact, MemoryHeader,
    MemoryType,
};
pub use provider::LocalAgentProvider;
