//! Native integration contracts. The registry accepts only compiled adapters
//! and exact, task-scoped selections. It has no network, polling, arbitrary
//! query, write, or send capability.
pub mod imessage;
mod tool_pack;
mod types;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use tool_pack::ReadToolPack;
pub use types::*;

pub type SharedRegistry = Arc<Mutex<Option<Registry>>>;

struct Grant {
    scope: Scope,
    epoch: u64,
    created: Instant,
    selected: Option<Conversation>,
    messages: Vec<Message>,
    enabled_message_ids: Vec<String>,
}

pub struct Registry {
    adapters: BTreeMap<String, Box<dyn Integration>>,
    grants: BTreeMap<String, Grant>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            adapters: BTreeMap::new(),
            grants: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, adapter: Box<dyn Integration>) -> Result<(), String> {
        let manifest = adapter.manifest();
        if manifest.id.is_empty()
            || !manifest
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
            || manifest.name.trim().is_empty()
            || manifest.capabilities.is_empty()
        {
            return Err("Invalid integration manifest".into());
        }
        if self.adapters.contains_key(&manifest.id) {
            return Err("Duplicate integration id".into());
        }
        self.adapters.insert(manifest.id, adapter);
        Ok(())
    }

    pub fn catalog(&self) -> Vec<Manifest> {
        self.adapters
            .values()
            .map(|adapter| adapter.manifest())
            .collect()
    }

    fn adapter(&self, id: &str) -> Result<&dyn Integration, String> {
        self.adapters
            .get(id)
            .map(|adapter| adapter.as_ref())
            .ok_or_else(|| "Unknown integration".into())
    }

    pub fn availability(&self, id: &str) -> Result<Availability, String> {
        Ok(self.adapter(id)?.availability())
    }

    /// At most one task owns an integration grant. Reconnecting revokes every
    /// prior conversation and tool selection before the native prompt opens.
    pub fn connect(&mut self, scope: &Scope, id: &str) -> Result<Vec<Conversation>, String> {
        self.revoke(id);
        let adapter = self.adapter(id)?;
        if !adapter.interactive() || !adapter.approve_read(&scope.task) {
            return Err("Access was not granted".into());
        }
        let epoch = adapter.epoch();
        let conversations = adapter.conversations()?;
        self.grants.insert(
            id.into(),
            Grant {
                scope: scope.clone(),
                epoch,
                created: Instant::now(),
                selected: None,
                messages: Vec::new(),
                enabled_message_ids: Vec::new(),
            },
        );
        Ok(conversations)
    }

    fn authorize(&self, scope: &Scope, id: &str) -> Result<&Grant, String> {
        let grant = self
            .grants
            .get(id)
            .ok_or("Connect this integration for this task first")?;
        let adapter = self.adapter(id)?;
        if &grant.scope != scope
            || grant.epoch != adapter.epoch()
            || grant.created.elapsed() >= Duration::from_secs(900)
            || !adapter.interactive()
        {
            return Err("Integration access expired, was revoked, or belongs to another task. Reconnect explicitly.".into());
        }
        Ok(grant)
    }

    pub fn select(
        &mut self,
        scope: &Scope,
        id: &str,
        conversation_id: &str,
    ) -> Result<Vec<Message>, String> {
        self.authorize(scope, id)?;
        let adapter = self.adapter(id)?;
        let selected = adapter
            .conversations()?
            .into_iter()
            .find(|conversation| conversation.id == conversation_id)
            .ok_or("This is not an eligible self-conversation")?;
        let messages = adapter.read(&selected)?;
        let grant = self.grants.get_mut(id).ok_or("Grant was revoked")?;
        grant.selected = Some(selected);
        grant.messages = messages.clone();
        grant.enabled_message_ids.clear();
        Ok(messages)
    }

    fn selected_text(
        &self,
        scope: &Scope,
        id: &str,
        message_ids: &[String],
    ) -> Result<String, String> {
        let grant = self.authorize(scope, id)?;
        let selected = grant
            .selected
            .as_ref()
            .ok_or("Select a conversation first")?;
        if message_ids.is_empty() || message_ids.len() > 20 {
            return Err("Select 1–20 messages".into());
        }
        if message_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != message_ids.len()
        {
            return Err("Selected message IDs must be unique".into());
        }
        let current = self.adapter(id)?.read(selected)?;
        for requested in message_ids {
            let previous = grant
                .messages
                .iter()
                .find(|message| &message.id == requested)
                .ok_or("Message was not in the selected snapshot")?;
            if !current
                .iter()
                .any(|message| message.id == previous.id && message.text == previous.text)
            {
                return Err("Selected text changed. Refresh and select it again.".into());
            }
        }
        let text = grant
            .messages
            .iter()
            .filter(|message| message_ids.contains(&message.id))
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        if text.len() > 32_000 {
            return Err("Selected text exceeds the 32 KB tool limit".into());
        }
        Ok(text)
    }

    /// Pin the exact message IDs the read tool may return. The renderer cannot
    /// enable a conversation or future messages as a wildcard.
    pub fn enable_read_tool(
        &mut self,
        scope: &Scope,
        id: &str,
        message_ids: Vec<String>,
    ) -> Result<usize, String> {
        self.selected_text(scope, id, &message_ids)?;
        let count = message_ids.len();
        self.grants
            .get_mut(id)
            .ok_or("Grant was revoked")?
            .enabled_message_ids = message_ids;
        Ok(count)
    }

    pub fn read_tool(&self, scope: &Scope, id: &str) -> Result<String, String> {
        let message_ids = self.authorize(scope, id)?.enabled_message_ids.clone();
        if message_ids.is_empty() {
            return Err("Select messages in Settings and enable them for this task first".into());
        }
        self.selected_text(scope, id, &message_ids)
    }

    pub fn disable_read_tool(&mut self, scope: &Scope, id: &str) -> Result<(), String> {
        self.authorize(scope, id)?;
        self.grants
            .get_mut(id)
            .ok_or("Grant was revoked")?
            .enabled_message_ids
            .clear();
        Ok(())
    }

    pub fn revoke(&mut self, id: &str) {
        self.grants.remove(id);
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
