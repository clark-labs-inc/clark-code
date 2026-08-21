use agent_core::{Provider, ProviderConfig, SessionId, SessionOptions};
use code_host::{CodingSessionExtensionRecipe, CodingSessionRecipe, ScoutCartographyRecipe};
use serde_json::Value;
use tauri::{AppHandle, State};

use super::{make_provider, prepare_provider_config, register_session, ProviderLaunchRequest};
use crate::product::{ProductRemoteSessionRequest, ProductRequestContext};
use crate::runtime_registry::{AccountKey, SessionKey, WorkerHandle};
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
    extensions: Vec<CodingSessionExtensionRecipe>,
) -> Result<Option<CodingSessionRecipe>, String> {
    let specialist_kind = config
        .extra
        .get("specialist_kind")
        .and_then(Value::as_str)
        .map(str::to_string);
    let hard_constraints = config
        .extra
        .get("hard_constraints")
        .cloned()
        .map(serde_json::from_value::<Vec<String>>)
        .transpose()
        .map_err(|error| format!("prepared coding hard constraints are invalid: {error}"))?
        .unwrap_or_default();
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
    if specialist_kind.is_none()
        && hard_constraints.is_empty()
        && scout_cartography.is_none()
        && extensions.is_empty()
    {
        return Ok(None);
    }
    let recipe = CodingSessionRecipe {
        specialist_kind: specialist_kind
            .or_else(|| scout_cartography.as_ref().map(|_| "scout".into())),
        scout_cartography,
        hard_constraints,
        extensions,
    };
    recipe.validate(project_root)?;
    Ok(Some(recipe))
}

fn unsupported_remote_renderer_field(config: &ProviderConfig) -> Option<String> {
    for (present, field) in [
        (config.endpoint.is_some(), "endpoint"),
        (config.command.is_some(), "command"),
        (config.cwd.is_some(), "cwd"),
        (config.auth_token.is_some(), "auth_token"),
        (!config.headers.is_empty(), "headers"),
    ] {
        if present {
            return Some(field.into());
        }
    }
    if !config.extra.is_object() {
        return Some("extra".into());
    }
    None
}

fn split_remote_renderer_extra(extra: Value) -> Result<(Value, Value), String> {
    let object = extra
        .as_object()
        .ok_or("remote worker configuration extra must be an object")?;
    let mut foundation = serde_json::Map::new();
    let mut product = serde_json::Map::new();
    for (key, value) in object {
        if matches!(
            key.as_str(),
            "specialist_kind" | "scout_cartography" | "hard_constraints"
        ) {
            foundation.insert(key.clone(), value.clone());
        } else {
            product.insert(key.clone(), value.clone());
        }
    }
    Ok((Value::Object(foundation), Value::Object(product)))
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
    if let Some(field) = unsupported_remote_renderer_field(&config) {
        return Err(format!(
            "remote worker configuration contains an unsupported renderer-owned field: {field}"
        ));
    }
    let (foundation_extra, product_extra) = split_remote_renderer_extra(config.extra)?;
    config.extra = foundation_extra;
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
    let extensions = state
        .product
        .prepare_remote_session_extensions(
            ProductRemoteSessionRequest {
                extra: product_extra,
                prepared_config: prepared.clone(),
                project_root: runtime.project_root().to_path_buf(),
                account_id: account.as_str().to_string(),
            },
            ProductRequestContext { app, state },
        )
        .await?;
    let recipe = remote_session_recipe(&prepared, runtime.project_root(), extensions)?;
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

fn specialist_requires_full_access(config: &ProviderConfig) -> bool {
    config.extra.get("scout_cartography").is_some()
        || config
            .extra
            .get("specialist_kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "scout" | "spec"))
        || config
            .extra
            .get("specialist")
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "scout" | "spec"))
        || config
            .extra
            .get("specialist")
            .and_then(Value::as_object)
            .and_then(|specialist| specialist.get("kind"))
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "scout" | "spec"))
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
    // Uninterrupted specialists own a native Full-access contract rather than
    // a mutable WebView preference. Their hard constraints remain enforced by
    // the provider before any permission mode is consulted.
    let protected_full_access = specialist_requires_full_access(&config);
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
            // Allocate the public identity before the provider creates any
            // session-scoped state. Renaming a provider session afterward is
            // too late for document workspaces and other native handles.
            let session_id = bind_id
                .map(SessionId::new)
                .unwrap_or_else(|| SessionId::new(uuid::Uuid::new_v4().to_string()));
            SessionKey::from_session(&session_id)?;
            if !provider_local::is_safe_session_id(session_id.as_str()) {
                return Err("session identity cannot be used as a workspace name".into());
            }
            options.session_id = Some(session_id.clone());
            if protected_full_access {
                options.mode = Some("full".into());
                options.collaboration_mode = Some(agent_core::CollaborationMode::Default);
            }
            let mut session = provider
                .new_session(options)
                .await
                .map_err(|e| e.to_string())?;
            if session.id != session_id {
                session.id = session_id;
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

    use super::{
        remote_session_recipe, specialist_requires_full_access, split_remote_renderer_extra,
        unsupported_remote_renderer_field, SessionOpenRequest,
    };

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
        let project_root = if cfg!(windows) {
            std::path::PathBuf::from(r"C:\srv\neon")
        } else {
            std::path::PathBuf::from("/srv/neon")
        };
        let recipe = remote_session_recipe(&config, &project_root, Vec::new())
            .unwrap()
            .unwrap();
        assert_eq!(recipe.specialist_kind.as_deref(), Some("scout"));
        assert_eq!(
            recipe.scout_cartography.unwrap().identity_root,
            project_root.join(".clark/scout/identity/binding-1")
        );
    }

    #[test]
    fn remote_renderer_config_splits_product_metadata_from_the_typed_recipe() {
        let config = ProviderConfig {
            extra: json!({
                "specialist_kind": "scout",
                "hard_constraints": ["no_delete", "no_github_push"],
                "scout_cartography": { "workspace_id": "workspace-1" },
                "cloud_advisor": { "organization_id": "org-1" }
            }),
            ..ProviderConfig::default()
        };
        assert_eq!(unsupported_remote_renderer_field(&config), None);
        let (foundation, product) = split_remote_renderer_extra(config.extra).unwrap();
        assert_eq!(foundation["specialist_kind"], "scout");
        assert_eq!(foundation["hard_constraints"][0], "no_delete");
        assert!(foundation.get("cloud_advisor").is_none());
        assert_eq!(product["cloud_advisor"]["organization_id"], "org-1");

        let invalid_constraints = ProviderConfig {
            extra: json!({ "hard_constraints": ["no_delete", 7] }),
            ..ProviderConfig::default()
        };
        assert!(remote_session_recipe(
            &invalid_constraints,
            std::path::Path::new("/srv/neon"),
            Vec::new()
        )
        .is_err());

        let renderer_owned_route = ProviderConfig {
            cwd: Some("/renderer/chosen".into()),
            ..ProviderConfig::default()
        };
        assert_eq!(
            unsupported_remote_renderer_field(&renderer_owned_route).as_deref(),
            Some("cwd")
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
    fn protected_specialist_bindings_require_full_access() {
        for extra in [
            json!({ "scout_cartography": { "workspace_id": "workspace-1" } }),
            json!({ "specialist_kind": "spec" }),
            json!({ "specialist": "scout" }),
            json!({ "specialist": { "kind": "scout" } }),
        ] {
            assert!(specialist_requires_full_access(&ProviderConfig {
                extra,
                ..ProviderConfig::default()
            }));
        }
        assert!(!specialist_requires_full_access(&ProviderConfig {
            extra: json!({ "specialist": { "kind": "security" } }),
            ..ProviderConfig::default()
        }));
    }
}
