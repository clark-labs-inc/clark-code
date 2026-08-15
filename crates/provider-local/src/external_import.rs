//! Discover importable setup from compatible coding agents.
//!
//! Discovery is read-only and executor-backed, so the same contract works for
//! local repositories and repositories reached through a native-owned worker.
//! Clark Code imports compatible stdio MCP servers into its app settings while
//! reading skills and project instructions in place, avoiding copied config
//! that can drift from the source agent's setup.

mod claude;
mod memories;
#[path = "external_import/codex.rs"]
mod openai;

pub(crate) use memories::migrate as migrate_memories;

use std::path::Path;

use serde::Serialize;

use crate::exec::Executor;
use crate::markdown_frontmatter::resolve_home;
use crate::mcp::McpServerConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationSource {
    Claude,
    Openai,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MigratedSkill {
    pub name: String,
    pub description: String,
    pub path: String,
    pub scope: &'static str,
    pub source: MigrationSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MigratedInstruction {
    pub path: String,
    pub scope: &'static str,
    pub source: MigrationSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentMigrationDiscovery {
    pub source: MigrationSource,
    pub mcp: Vec<McpServerConfig>,
    pub skills: Vec<MigratedSkill>,
    pub instructions: Vec<MigratedInstruction>,
}

impl AgentMigrationDiscovery {
    fn is_empty(&self) -> bool {
        self.mcp.is_empty() && self.skills.is_empty() && self.instructions.is_empty()
    }
}

/// Detect supported setup for each source independently. Empty sources are
/// omitted so the UI can offer migration only when there is something real to
/// review.
pub async fn discover_agent_setups(
    exec: &dyn Executor,
    project_root: &Path,
) -> Vec<AgentMigrationDiscovery> {
    let home = resolve_home(exec, project_root).await;
    discover_agent_setups_with_home(exec, project_root, home.as_deref()).await
}

/// Deterministic discovery with an explicit source home. Besides making evals
/// independent of the developer machine, this lets callers represent an
/// executor target whose home has already been resolved.
pub async fn discover_agent_setups_with_home(
    exec: &dyn Executor,
    project_root: &Path,
    home: Option<&Path>,
) -> Vec<AgentMigrationDiscovery> {
    let mut discoveries = Vec::new();
    let claude = AgentMigrationDiscovery {
        source: MigrationSource::Claude,
        mcp: claude::discover_mcp_servers(exec, project_root, home).await,
        skills: claude::discover_skills(exec, project_root, home).await,
        instructions: claude::discover_instructions(exec, project_root).await,
    };
    if !claude.is_empty() {
        discoveries.push(claude);
    }

    let openai = AgentMigrationDiscovery {
        source: MigrationSource::Openai,
        mcp: openai::discover_mcp_servers(exec, project_root, home).await,
        skills: openai::discover_skills(exec, project_root, home).await,
        instructions: openai::discover_instructions(exec, project_root).await,
    };
    if !openai.is_empty() {
        discoveries.push(openai);
    }
    discoveries
}

#[cfg(test)]
#[path = "external_import_tests.rs"]
mod tests;
