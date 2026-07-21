//! Deferred tool discovery and per-conversation schema exposure.
//!
//! Every executor remains registered so dispatch and permissions have one
//! canonical lookup path. A required `ToolGate` advertises only the compact
//! eager set plus schemas activated by `tool_search` for this conversation.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use crate::loop_state::SessionState;

use super::{arg_i64_opt, arg_str, ToolCtx, ToolExecutor, ToolOutcome};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ToolExposure {
    Eager,
    Deferred,
}

#[derive(Clone, Debug)]
struct CatalogEntry {
    name: String,
    description: String,
}

#[derive(Default)]
struct Catalog {
    eager: HashSet<String>,
    deferred: Vec<CatalogEntry>,
}

#[derive(Clone, Default)]
pub(super) struct DeferredToolCatalog {
    inner: Arc<Mutex<Catalog>>,
}

impl DeferredToolCatalog {
    pub(super) fn register(&self, name: &str, description: &str, exposure: ToolExposure) {
        let mut catalog = self.inner.lock().unwrap();
        match exposure {
            ToolExposure::Eager => {
                catalog.eager.insert(name.to_string());
            }
            ToolExposure::Deferred => catalog.deferred.push(CatalogEntry {
                name: name.to_string(),
                description: description.to_string(),
            }),
        }
    }

    pub(super) fn remove_prefix(&self, prefixes: &[&str]) {
        let mut catalog = self.inner.lock().unwrap();
        catalog
            .eager
            .retain(|name| !prefixes.iter().any(|prefix| name.starts_with(prefix)));
        catalog
            .deferred
            .retain(|entry| !prefixes.iter().any(|prefix| entry.name.starts_with(prefix)));
    }

    fn eager_names(&self) -> HashSet<String> {
        self.inner.lock().unwrap().eager.clone()
    }

    fn is_deferred(&self, name: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .deferred
            .iter()
            .any(|entry| entry.name == name)
    }

    fn search(&self, query: &str, limit: usize) -> Vec<CatalogEntry> {
        let phrase = query.trim().to_ascii_lowercase();
        let terms = phrase
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|term| term.len() >= 2)
            .collect::<Vec<_>>();
        if phrase.is_empty() || terms.is_empty() {
            return Vec::new();
        }

        let catalog = self.inner.lock().unwrap();
        let mut ranked = catalog
            .deferred
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let name = entry.name.to_ascii_lowercase();
                let description = entry.description.to_ascii_lowercase();
                let mut score = 0usize;
                if name == phrase {
                    score += 100;
                } else if name.contains(&phrase) {
                    score += 40;
                }
                for term in &terms {
                    if name.contains(term) {
                        score += 20;
                    }
                    if description.contains(term) {
                        score += 5;
                    }
                }
                (score > 0).then(|| (score, index, entry.clone()))
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        ranked
            .into_iter()
            .take(limit)
            .map(|(_, _, entry)| entry)
            .collect()
    }
}

pub(super) struct ToolSearch {
    catalog: DeferredToolCatalog,
}

impl ToolSearch {
    pub(super) fn new(catalog: DeferredToolCatalog) -> Self {
        Self { catalog }
    }
}

#[async_trait]
impl ToolExecutor for ToolSearch {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn description(&self) -> &str {
        "Find and activate deferred tools when the visible core tools cannot perform the task. Search by capability, such as devices, goals, web, memory, images, integrations, or MCP."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Capability or tool to find."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 12,
                    "description": "Maximum matches to activate (default 8)."
                }
            },
            "required": ["query"]
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let query = match arg_str(&args, "query") {
            Ok(query) if !query.trim().is_empty() => query,
            _ => return ToolOutcome::error("`query` must be a non-empty capability description"),
        };
        let limit = arg_i64_opt(&args, "limit").unwrap_or(8).clamp(1, 12) as usize;
        let matches = self.catalog.search(&query, limit);
        if matches.is_empty() {
            return ToolOutcome::ok(
                "No deferred tools matched. Try a concrete capability such as Android, iOS, goals, web research, memory, image generation, browser, organization knowledge, delegation, or an MCP server/tool name.",
            );
        }

        {
            let mut session = ctx.session.lock().await;
            session
                .deferred_tools
                .extend(matches.iter().map(|entry| entry.name.clone()));
        }
        let lines = matches
            .iter()
            .map(|entry| {
                let description = entry
                    .description
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(220)
                    .collect::<String>();
                format!("- `{}`: {}", entry.name, description)
            })
            .collect::<Vec<_>>()
            .join("\n");
        ToolOutcome::ok(format!(
            "Activated deferred tools for the next model call:\n{lines}"
        ))
    }
}

pub(super) struct DeferredToolGate {
    catalog: DeferredToolCatalog,
    session: Arc<AsyncMutex<SessionState>>,
}

impl DeferredToolGate {
    pub(super) fn new(
        catalog: DeferredToolCatalog,
        session: Arc<AsyncMutex<SessionState>>,
    ) -> Self {
        Self { catalog, session }
    }

    async fn activated_names(&self) -> HashSet<String> {
        let session = self.session.lock().await;
        let mut names = session.deferred_tools.clone();
        if session.goal.is_some() {
            names.extend(
                ["create_goal", "update_goal", "get_goal"]
                    .into_iter()
                    .map(str::to_string),
            );
        }
        names
    }
}

impl clark_agent::Plugin for DeferredToolGate {
    fn name(&self) -> &'static str {
        "deferred_tool_gate"
    }

    fn capabilities(&self) -> clark_agent::PluginCapabilities {
        clark_agent::PluginCapabilities::tool_gate()
    }
}

#[async_trait]
impl clark_agent::plugin::ToolGate for DeferredToolGate {
    async fn next_turn_tool_allowlist(
        &self,
        ctx: clark_agent::plugin::ToolGateContext<'_>,
    ) -> Option<HashSet<String>> {
        let mut allowed = self.catalog.eager_names();
        allowed.extend(self.activated_names().await);
        allowed.retain(|name| {
            ctx.available_tool_names
                .iter()
                .any(|available| *available == name)
        });
        Some(allowed)
    }

    async fn denial_reason(
        &self,
        tool_name: &str,
        _ctx: clark_agent::plugin::ToolGateContext<'_>,
    ) -> Option<String> {
        if self.catalog.is_deferred(tool_name) && !self.activated_names().await.contains(tool_name)
        {
            return Some(format!(
                "Tool `{tool_name}` is deferred. Call `tool_search` for the capability first."
            ));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clark_agent::plugin::ToolGate;

    #[test]
    fn search_prefers_names_and_preserves_registration_order_for_ties() {
        let catalog = DeferredToolCatalog::default();
        catalog.register(
            "android_tap",
            "Tap an Android device",
            ToolExposure::Deferred,
        );
        catalog.register(
            "android_screenshot",
            "Capture an Android screenshot",
            ToolExposure::Deferred,
        );
        catalog.register("memory", "Recall durable facts", ToolExposure::Deferred);

        let names = catalog
            .search("android", 8)
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["android_tap", "android_screenshot"]);
    }

    #[tokio::test]
    async fn gate_exposes_only_eager_and_conversation_activated_tools() {
        let catalog = DeferredToolCatalog::default();
        catalog.register("read_file", "Read", ToolExposure::Eager);
        catalog.register("tool_search", "Discover", ToolExposure::Eager);
        catalog.register("android_tap", "Tap", ToolExposure::Deferred);
        let session = Arc::new(AsyncMutex::new(SessionState::default()));
        let gate = DeferredToolGate::new(catalog, session.clone());
        let available = ["read_file", "tool_search", "android_tap"];

        let initial = gate
            .next_turn_tool_allowlist(clark_agent::plugin::ToolGateContext {
                iteration: 0,
                messages: &[],
                conversation_id: Some("session"),
                available_tool_names: &available,
            })
            .await
            .unwrap();
        assert_eq!(
            initial,
            HashSet::from(["read_file".into(), "tool_search".into()])
        );

        session
            .lock()
            .await
            .deferred_tools
            .insert("android_tap".into());
        let activated = gate
            .next_turn_tool_allowlist(clark_agent::plugin::ToolGateContext {
                iteration: 1,
                messages: &[],
                conversation_id: Some("session"),
                available_tool_names: &available,
            })
            .await
            .unwrap();
        assert!(activated.contains("android_tap"));
    }
}
