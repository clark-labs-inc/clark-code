//! Native integration broker. Renderer IPC may grant, select, enable, or
//! revoke exact text. The local-provider ToolPack can only consume the enabled
//! selection already bound to its native task scope.
use std::sync::{Arc, Mutex, Weak};

use desktop_integrations::{imessage::IMessage, ReadToolPack, Registry, Scope, SharedRegistry};
use serde::Deserialize;
use serde_json::Value;
use tauri::{Manager, State, WebviewWindow};

use crate::{runtime_registry::SessionKey, AppState};

pub(crate) struct IntegrationState {
    registry: SharedRegistry,
    instances: Mutex<SessionInstances>,
}

impl Default for IntegrationState {
    fn default() -> Self {
        Self {
            registry: Arc::new(Mutex::new(None)),
            instances: Mutex::new(SessionInstances::default()),
        }
    }
}

type LiveSession = tokio::sync::Mutex<crate::state::HostSession>;

#[derive(Default)]
struct SessionInstances {
    next: u64,
    sessions: Vec<(Weak<LiveSession>, u64)>,
}

impl IntegrationState {
    pub(crate) fn read_tool_pack(&self) -> Arc<ReadToolPack> {
        Arc::new(ReadToolPack::new(self.registry.clone()))
    }

    pub(crate) fn instance(&self, session: &Arc<LiveSession>) -> Result<u64, String> {
        let mut instances = self
            .instances
            .lock()
            .map_err(|_| "Session instance registry unavailable")?;
        instances
            .sessions
            .retain(|(session, _)| session.strong_count() > 0);
        let weak = Arc::downgrade(session);
        if let Some((_, id)) = instances
            .sessions
            .iter()
            .find(|(known, _)| known.ptr_eq(&weak))
        {
            return Ok(*id);
        }
        instances.next = instances
            .next
            .checked_add(1)
            .ok_or("Session identity space exhausted")?;
        let id = instances.next;
        instances.sessions.push((weak, id));
        Ok(id)
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request {
    Catalog,
    Status {
        id: String,
    },
    OpenSettings,
    Connect {
        id: String,
    },
    Select {
        id: String,
        conversation_id: String,
    },
    EnableReadTool {
        id: String,
        message_ids: Vec<String>,
    },
    DisableReadTool {
        id: String,
    },
    Revoke {
        id: String,
    },
}

fn value(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|_| "Cannot encode integration response".into())
}

impl Request {
    fn needs_scope(&self) -> bool {
        !matches!(
            self,
            Self::Catalog | Self::Status { .. } | Self::OpenSettings | Self::Revoke { .. }
        )
    }

    fn execute(self, registry: &mut Registry, scope: Option<&Scope>) -> Result<Value, String> {
        match self {
            Self::Catalog => value(registry.catalog()),
            Self::Status { id } => value(registry.availability(&id)?),
            Self::Revoke { id } => {
                registry.revoke(&id);
                Ok(Value::Null)
            }
            Self::OpenSettings => {
                desktop_integrations::imessage::open_privacy_settings();
                Ok(Value::Null)
            }
            request => {
                let scope = scope.ok_or("Open a task before granting integration access")?;
                match request {
                    Self::Connect { id } => value(registry.connect(scope, &id)?),
                    Self::Select {
                        id,
                        conversation_id,
                    } => value(registry.select(scope, &id, &conversation_id)?),
                    Self::EnableReadTool { id, message_ids } => {
                        value(registry.enable_read_tool(scope, &id, message_ids)?)
                    }
                    Self::DisableReadTool { id } => {
                        registry.disable_read_tool(scope, &id)?;
                        Ok(Value::Null)
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

#[tauri::command]
pub(crate) async fn integration_request(
    window: WebviewWindow,
    state: State<'_, AppState>,
    session_id: Option<String>,
    request: Request,
) -> Result<Value, String> {
    if window.label() != "main" {
        return Err("Integrations are available only in the main native window".into());
    }
    let _account_lifecycle = state.account_lifecycle.read().await;
    let scope = if request.needs_scope() {
        let key = SessionKey::parse(session_id.ok_or("Open a task first")?)?;
        let entry = state
            .runtime_registry
            .current_session_entry(&key)
            .await
            .ok_or("This task is no longer open")?;
        let account = state.runtime_registry.cloud_account().await;
        Some(Scope {
            owner: account
                .map(|account| account.account.as_str().to_owned())
                .unwrap_or_else(|| "local".into()),
            task: key.as_str().into(),
            generation: state.runtime_registry.cloud_account_generation(),
            instance: window.state::<IntegrationState>().instance(&entry)?,
        })
    } else {
        None
    };
    let app = window.app_handle().clone();
    let callback_app = app.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = (|| {
            let service = callback_app.state::<IntegrationState>();
            // NSAlert runs a nested event loop. Avoid a second integration
            // request while native approval is already open.
            let mut guard = service
                .registry
                .try_lock()
                .map_err(|_| "An integration operation is already active")?;
            if guard.is_none() {
                let mut registry = Registry::new();
                registry.register(Box::new(IMessage::local()?))?;
                *guard = Some(registry);
            }
            request.execute(
                guard.as_mut().ok_or("Integration registry unavailable")?,
                scope.as_ref(),
            )
        })();
        let _ = send.send(result);
    })
    .map_err(|_| "Native integration dispatch unavailable")?;
    receive
        .await
        .map_err(|_| "Native integration operation was interrupted")?
}

#[cfg(test)]
mod tests {
    use super::Request;

    #[test]
    fn renderer_cannot_add_write_or_widen_read_arguments() {
        for input in [
            serde_json::json!({"action":"send", "draft_id":"a"}),
            serde_json::json!({"action":"draft", "id":"imessage", "text":"hello"}),
            serde_json::json!({"action":"request_automation", "id":"imessage"}),
            serde_json::json!({"action":"enable_read_tool", "id":"imessage", "message_ids":["1"], "conversation_id":"2"}),
            serde_json::json!({"action":"connect", "id":"imessage", "owner":"another-account"}),
        ] {
            assert!(serde_json::from_value::<Request>(input).is_err());
        }
    }
}
