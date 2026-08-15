use agent_core::{Provider, ProviderConfig, SessionId, SessionOptions};
use code_host::{CodingSessionRecipe, ScoutCartographyRecipe};
use serde_json::Value;
use tauri::{AppHandle, State};

use super::{make_provider, prepare_provider_config, register_session, ProviderLaunchRequest};
use crate::runtime_registry::{AccountKey, WorkerHandle};
use crate::state::AppState;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteWorkerBinding {
    worker_handle: String,
    cwd: String,
}

fn remote_session_recipe(
    config: &ProviderConfig,
    project_root: &std::path::Path,
) -> Result<Option<CodingSessionRecipe>, String> {
    let specialist_kind = config
        .extra
        .get("specialist_kind")
        .and_then(Value::as_str)
        .map(str::to_string);
    let scout_cartography = config
        .extra
        .get("scout_cartography")
        .map(|value| {
            let mut object = value
                .as_object()
                .cloned()
                .ok_or("prepared Scout cartography recipe is invalid")?;
            let identity_name = object
                .get("identity_root")
                .and_then(Value::as_str)
                .and_then(|path| std::path::Path::new(path).file_name())
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .ok_or("prepared Scout identity scope is invalid")?;
            let remote_identity_root = project_root
                .join(".clark")
                .join("scout")
                .join("identity")
                .join(identity_name);
            object.insert(
                "identity_root".into(),
                Value::String(remote_identity_root.to_string_lossy().into_owned()),
            );
            // target_id is consumed by the product when deriving the private
            // identity scope. The worker receives no SSH destination or local
            // app-data path in its session recipe.
            object.remove("target_id");
            serde_json::from_value::<ScoutCartographyRecipe>(Value::Object(object))
                .map_err(|error| format!("prepared Scout cartography recipe is invalid: {error}"))
        })
        .transpose()?;
    if specialist_kind.is_none() && scout_cartography.is_none() {
        return Ok(None);
    }
    let recipe = CodingSessionRecipe {
        specialist_kind: specialist_kind
            .or_else(|| scout_cartography.as_ref().map(|_| "scout".into())),
        scout_cartography,
    };
    recipe.validate(project_root)?;
    Ok(Some(recipe))
}

async fn open_provider(
    provider_id: &str,
    app: &AppHandle,
    state: &AppState,
    mut config: ProviderConfig,
) -> Result<(Box<dyn Provider>, Option<AccountKey>), String> {
    let remote_binding = config
        .extra
        .as_object_mut()
        .and_then(|extra| extra.remove("remote_worker"));
    let Some(remote_binding) = remote_binding else {
        let (config, account) = prepare_provider_config(provider_id, app, config, state).await?;
        let mut provider = make_provider(provider_id, &config, app, state).await?;
        provider
            .connect(config)
            .await
            .map_err(|error| error.to_string())?;
        return Ok((provider, account));
    };

    if provider_id != "local" {
        return Err("remote workers can only back the local coding provider".into());
    }
    let binding: RemoteWorkerBinding = serde_json::from_value(remote_binding)
        .map_err(|error| format!("remote worker binding is invalid: {error}"))?;
    let renderer_extra_is_bounded = config.extra.as_object().is_some_and(|extra| {
        extra
            .keys()
            .all(|key| matches!(key.as_str(), "specialist_kind" | "scout_cartography"))
    });
    if config.endpoint.is_some()
        || config.command.is_some()
        || config.cwd.is_some()
        || config.auth_token.is_some()
        || !config.headers.is_empty()
        || !renderer_extra_is_bounded
    {
        return Err(
            "remote worker configuration contains an unsupported renderer-owned field".into(),
        );
    }
    let (prepared, prepared_account) =
        prepare_provider_config(provider_id, app, config, state).await?;
    let registry_account = state
        .runtime_registry
        .cloud_account()
        .await
        .map(|account| account.account.as_str().to_string())
        .ok_or("Clark Code must be signed in before opening a remote worker session")?;
    let account = AccountKey::new(registry_account)?;
    if prepared_account
        .as_ref()
        .is_some_and(|owner| owner != &account)
    {
        return Err("remote worker and specialist authority belong to different accounts".into());
    }
    let handle = WorkerHandle::parse(&binding.worker_handle)?;
    let runtime = state.runtime_registry.resolve(&account, &handle).await?;
    if std::path::Path::new(&binding.cwd) != runtime.project_root() {
        return Err("remote session root does not match its native worker registration".into());
    }
    let recipe = remote_session_recipe(&prepared, runtime.project_root())?;
    let mut provider = provider_remote_worker::RemoteWorkerProvider::new(
        runtime.worker(),
        runtime.project_id().as_str().to_string(),
        runtime.project_root().to_path_buf(),
    );
    if let Some(recipe) = recipe {
        provider = provider.with_session_recipe(recipe);
    }
    provider
        .connect(ProviderConfig::default())
        .await
        .map_err(|error| error.to_string())?;
    Ok((Box::new(provider), Some(account)))
}

/// The complete provider/session binding requested by one WebView invocation.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionOpenRequest {
    New {
        options: SessionOptions,
        bind_id: Option<String>,
    },
    Load {
        id: String,
    },
}

fn scout_requires_full_access(config: &ProviderConfig) -> bool {
    config.extra.get("scout_cartography").is_some()
        || config
            .extra
            .get("specialist")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == "scout")
        || config
            .extra
            .get("specialist")
            .and_then(Value::as_object)
            .and_then(|specialist| specialist.get("kind"))
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == "scout")
}

/// One native transaction that constructs, configures, connects, and binds a
/// provider to exactly one session. No unbound provider is ever published in
/// shared state, so concurrent opens cannot consume or overwrite each other.
#[tauri::command]
pub async fn session_open(
    provider_id: String,
    config: ProviderLaunchRequest,
    request: SessionOpenRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    tracing::info!(provider = %provider_id, "session_open");
    let _account_lifecycle = state.account_lifecycle.read().await;
    let config = config.into_provider_config(&provider_id)?;
    // Scout maps an explicitly selected organization/workspace rather than a
    // single checkout. Full access is therefore part of the native session
    // contract, not a mutable WebView preference.
    let scout_full_access = scout_requires_full_access(&config);
    let (mut provider, account) = open_provider(&provider_id, &app, state.inner(), config).await?;
    let account = match account {
        Some(account) => Some(account),
        None => state
            .runtime_registry
            .cloud_account()
            .await
            .map(|current| current.account),
    };

    let session = match request {
        SessionOpenRequest::New {
            mut options,
            bind_id,
        } => {
            if scout_full_access {
                options.mode = Some("full".into());
                options.collaboration_mode = Some(agent_core::CollaborationMode::Default);
            }
            let mut session = provider
                .new_session(options)
                .await
                .map_err(|e| e.to_string())?;
            if let Some(bind) = bind_id {
                session.id = SessionId::new(bind);
            }
            session
        }
        SessionOpenRequest::Load { id } => provider
            .load_session(SessionId::new(id))
            .await
            .map_err(|e| e.to_string())?,
    };

    if let Some(owner) = account.as_ref().map(AccountKey::as_str) {
        let still_current = state
            .runtime_registry
            .cloud_account()
            .await
            .is_some_and(|current| current.account.as_str() == owner);
        if !still_current {
            let _ = provider.close_session(&session.id).await;
            return Err("Clark Code account changed while opening the remote session".into());
        }
        return register_session(&app, &state, provider, session, account).await;
    }
    register_session(&app, &state, provider, session, None).await
}

#[cfg(test)]
mod tests {
    use agent_core::ProviderConfig;
    use serde_json::json;

    use super::{remote_session_recipe, scout_requires_full_access, SessionOpenRequest};

    #[test]
    fn new_request_requires_the_native_bind_id_wire_name() {
        let request: SessionOpenRequest = serde_json::from_value(serde_json::json!({
            "kind": "new",
            "options": { "cwd": "/srv/project" },
            "bind_id": "conversation-1"
        }))
        .expect("valid session-open request");

        match request {
            SessionOpenRequest::New { bind_id, .. } => {
                assert_eq!(bind_id.as_deref(), Some("conversation-1"));
            }
            SessionOpenRequest::Load { .. } => panic!("expected new request"),
        }
    }

    #[test]
    fn prepared_scout_recipe_moves_identity_into_the_remote_project() {
        let config = ProviderConfig {
            extra: json!({
                "specialist_kind": "scout",
                "scout_cartography": {
                    "organization_id": "59b8fe20-6072-4c16-9dae-9d7cbbf2533c",
                    "workspace_id": "2fac2db5-20d6-499c-b691-47ad19fc0ca8",
                    "identity_root": "/Users/test/Library/Application Support/Clark/scout/binding-1",
                    "platform": "linux",
                    "architecture": "x86_64",
                    "route_prefix": "/v1/system-cartography",
                    "human_run_request_id": format!("scout-run:{}", "a".repeat(64)),
                    "target_id": "client-neon"
                }
            }),
            ..ProviderConfig::default()
        };
        let recipe = remote_session_recipe(&config, std::path::Path::new("/srv/neon"))
            .unwrap()
            .unwrap();
        assert_eq!(recipe.specialist_kind.as_deref(), Some("scout"));
        assert_eq!(
            recipe.scout_cartography.unwrap().identity_root,
            std::path::PathBuf::from("/srv/neon/.clark/scout/identity/binding-1")
        );
    }

    #[test]
    fn load_request_carries_only_the_requested_session_identity() {
        let request: SessionOpenRequest = serde_json::from_value(serde_json::json!({
            "kind": "load",
            "id": "conversation-2"
        }))
        .expect("valid session-open request");

        match request {
            SessionOpenRequest::Load { id } => assert_eq!(id, "conversation-2"),
            SessionOpenRequest::New { .. } => panic!("expected load request"),
        }
    }

    #[test]
    fn scout_bindings_require_full_access() {
        for extra in [
            json!({ "scout_cartography": { "workspace_id": "workspace-1" } }),
            json!({ "specialist": "scout" }),
            json!({ "specialist": { "kind": "scout" } }),
        ] {
            assert!(scout_requires_full_access(&ProviderConfig {
                extra,
                ..ProviderConfig::default()
            }));
        }
        assert!(!scout_requires_full_access(&ProviderConfig {
            extra: json!({ "specialist": { "kind": "security" } }),
            ..ProviderConfig::default()
        }));
    }
}
