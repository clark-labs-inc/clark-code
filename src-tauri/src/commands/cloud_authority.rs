use base64::Engine as _;
use serde_json::Value;
use tokio::sync::OwnedRwLockReadGuard;

use crate::state::AppState;

const CLARK_HOSTS: &[&str] = &["www.clarkchat.com", "dev.clarkslabs.com"];

pub(crate) struct CloudAccess {
    pub rest_base: String,
    pub owner_scope: String,
    pub token: String,
    _account_lifecycle: Option<OwnedRwLockReadGuard<()>>,
}

#[cfg(test)]
impl CloudAccess {
    pub(crate) fn for_test(rest_base: String, owner_scope: String, token: String) -> Self {
        Self {
            rest_base,
            owner_scope,
            token,
            _account_lifecycle: None,
        }
    }
}

fn trusted_clark_origin(raw: &str, gateway: bool) -> Result<String, String> {
    let mut url =
        reqwest::Url::parse(raw.trim()).map_err(|_| "Clark endpoint is invalid".to_string())?;
    if url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Clark endpoint must be an exact trusted origin".into());
    }

    let path = url.path();
    if gateway {
        if path != "/" && path != "/ws" {
            return Err("Clark gateway endpoint must end at /ws".into());
        }
    } else if path != "/" {
        return Err("Clark authentication endpoint must be an origin".into());
    }

    let host = url
        .host_str()
        .ok_or_else(|| "Clark endpoint has no host".to_string())?;
    let production = CLARK_HOSTS.contains(&host);
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
    let scheme = url.scheme();
    let secure = matches!(scheme, "https" | "wss");
    let local_debug = cfg!(any(debug_assertions, test))
        && loopback
        && matches!(scheme, "http" | "ws" | "https" | "wss");
    if !(production && secure && url.port().is_none()) && !local_debug {
        return Err("Clark endpoint is not an approved Clark origin".into());
    }

    if matches!(scheme, "ws" | "wss") {
        let rest_scheme = if scheme == "wss" { "https" } else { "http" };
        url.set_scheme(rest_scheme)
            .map_err(|_| "Clark endpoint scheme is invalid".to_string())?;
    }
    url.set_path("/");
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub(crate) fn clark_rest_base(endpoint: &str) -> Result<String, String> {
    trusted_clark_origin(endpoint, true)
}

pub(crate) fn clark_auth_base(origin: &str) -> Result<String, String> {
    trusted_clark_origin(origin, false)
}

pub(crate) fn clark_gateway_endpoint(rest_base: &str) -> Result<String, String> {
    let rest_base = trusted_clark_origin(rest_base, false)?;
    let mut url =
        reqwest::Url::parse(&rest_base).map_err(|_| "Clark endpoint is invalid".to_string())?;
    let websocket_scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        _ => return Err("Clark endpoint scheme is invalid".into()),
    };
    url.set_scheme(websocket_scheme)
        .map_err(|_| "Clark endpoint scheme is invalid".to_string())?;
    url.set_path("/ws");
    Ok(url.to_string())
}

/// Shared authenticated client. Redirects are disabled so a bearer or Google
/// ID token cannot follow a server response onto another origin.
static CLOUD_HTTP: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    clark_http::build_client(clark_http::ClientOptions::default()).expect("build cloud http client")
});

pub(crate) fn clark_http_client() -> Result<reqwest::Client, String> {
    Ok(CLOUD_HTTP.clone())
}

pub(crate) fn jwt_subject(token: &str) -> Result<String, String> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| "Clark returned an invalid session token".to_string())?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .map_err(|_| "Clark returned an invalid session token".to_string())?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Clark returned an invalid session token".to_string())?;
    value
        .get("sub")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Clark session token has no account subject".to_string())
}

pub(crate) async fn clear_cloud_authority(state: &AppState) -> Result<(), String> {
    let _switch = state.account_lifecycle.write().await;
    let mut generation = state
        .runtime_registry
        .cloud_account_generation_write()
        .await;
    let active = generation.clone();
    let active_owner = active
        .as_ref()
        .map(|account| account.account.as_str().to_string());
    // Delete the complete durable credential generation before unpublishing
    // native resources. A disk failure leaves the old account intact.
    state.credentials.sign_out(active_owner.as_deref()).await?;
    let active_account = active.map(|account| account.account);
    let removed_sessions = if let Some(account) = active_account.as_ref() {
        state.runtime_registry.take_account_sessions(account).await
    } else {
        Vec::new()
    };
    *generation = None;
    drop(generation);
    if let Some(account) = active_account {
        state.runtime_registry.disconnect_account(&account).await;
    }
    for entry in removed_sessions {
        let mut live = entry.lock().await;
        live.closing = true;
        let session_id = live.session.id.clone();
        if let Err(error) = live.provider.close_session(&session_id).await {
            tracing::warn!(%error, session = %session_id, "signed-out provider close failed");
        }
    }
    Ok(())
}

/// Return the one native account generation used by ordinary cloud commands.
/// No renderer-controlled endpoint, token, or account label participates.
pub(crate) async fn current_cloud_access(state: &AppState) -> Result<CloudAccess, String> {
    let account_lifecycle = state.account_lifecycle.clone().read_owned().await;
    let account = state
        .runtime_registry
        .cloud_account()
        .await
        .ok_or("Clark has no active signed-in account")?;
    Ok(CloudAccess {
        rest_base: account.rest_base,
        owner_scope: account.account.as_str().to_string(),
        token: account.token.as_str().to_string(),
        _account_lifecycle: Some(account_lifecycle),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use agent_core::provider::EventStream;
    use agent_core::{
        ClientResponse, Error, PromptInput, Provider, ProviderCapabilities, ProviderConfig,
        ProviderId, RunId, Session, SessionId, SessionOptions, Snapshot,
    };
    use base64::Engine as _;
    use futures::stream::{self, StreamExt};
    use tokio::sync::Mutex;

    use super::{
        clark_auth_base, clark_gateway_endpoint, clark_rest_base, clear_cloud_authority,
        current_cloud_access, jwt_subject,
    };
    use crate::runtime_registry::{AccountKey, CloudAccountState, SessionKey};
    use crate::state::HostSession;
    use crate::AppState;

    struct CloseRecordingProvider(Arc<AtomicBool>);

    #[async_trait::async_trait]
    impl Provider for CloseRecordingProvider {
        fn id(&self) -> ProviderId {
            ProviderId::new("account-switch-test")
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::default()
        }

        async fn connect(&mut self, _config: ProviderConfig) -> agent_core::Result<()> {
            Ok(())
        }

        async fn new_session(&mut self, _options: SessionOptions) -> agent_core::Result<Session> {
            Err(Error::Unsupported("not used by this test".into()))
        }

        async fn load_session(&mut self, _id: SessionId) -> agent_core::Result<Session> {
            Err(Error::Unsupported("not used by this test".into()))
        }

        async fn prompt(
            &mut self,
            _session: &SessionId,
            _input: PromptInput,
        ) -> agent_core::Result<EventStream> {
            Ok(stream::empty().boxed())
        }

        async fn cancel(&mut self, _session: &SessionId, _run: &RunId) -> agent_core::Result<()> {
            Ok(())
        }

        async fn close_session(&mut self, _session: &SessionId) -> agent_core::Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn respond(
            &mut self,
            _session: &SessionId,
            _response: ClientResponse,
        ) -> agent_core::Result<()> {
            Ok(())
        }
    }

    fn unsigned_token(subject: &str) -> String {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::json!({ "sub": subject }).to_string());
        format!("header.{payload}.signature")
    }

    #[test]
    fn trusted_origins_are_exact_and_normalized() {
        assert_eq!(
            clark_rest_base("wss://www.clarkchat.com/ws").unwrap(),
            "https://www.clarkchat.com"
        );
        assert_eq!(
            clark_auth_base("https://www.clarkchat.com").unwrap(),
            "https://www.clarkchat.com"
        );
        assert_eq!(
            clark_gateway_endpoint("https://www.clarkchat.com").unwrap(),
            "wss://www.clarkchat.com/ws"
        );
        assert!(clark_rest_base("https://evil.example/ws").is_err());
        assert!(clark_rest_base("https://www.clarkchat.com/other").is_err());
        assert!(clark_auth_base("https://www.clarkchat.com?next=evil").is_err());
    }

    #[test]
    fn jwt_subject_requires_a_nonempty_subject() {
        assert_eq!(
            jwt_subject(&unsigned_token("account-a")).unwrap(),
            "account-a"
        );
        assert!(jwt_subject("not-a-jwt").is_err());
        assert!(jwt_subject(&unsigned_token("")).is_err());
    }

    #[tokio::test]
    async fn sign_out_waits_for_active_account_admission() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        state
            .credentials
            .configure(root.path().join("credentials"))
            .unwrap();
        state
            .runtime_registry
            .set_cloud_account(Some(CloudAccountState {
                rest_base: "https://www.clarkchat.com".into(),
                account: AccountKey::new("account-a").unwrap(),
                token: zeroize::Zeroizing::new("token-a".into()),
            }))
            .await;
        let admission = current_cloud_access(&state).await.unwrap();
        let switching = {
            let state = state.clone();
            tokio::spawn(async move { clear_cloud_authority(&state).await })
        };
        tokio::task::yield_now().await;
        assert!(!switching.is_finished());

        drop(admission);
        switching.await.unwrap().unwrap();
        assert!(state.runtime_registry.cloud_account().await.is_none());
    }

    #[tokio::test]
    async fn same_account_requests_do_not_serialize_behind_each_other() {
        let state = AppState::new();
        state
            .runtime_registry
            .set_cloud_account(Some(CloudAccountState {
                rest_base: "https://www.clarkchat.com".into(),
                account: AccountKey::new("account-a").unwrap(),
                token: zeroize::Zeroizing::new("token-a".into()),
            }))
            .await;

        let first = current_cloud_access(&state).await.unwrap();
        let second = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            current_cloud_access(&state),
        )
        .await
        .expect("same-account admission must remain concurrent")
        .unwrap();

        assert_eq!(first.owner_scope, second.owner_scope);
    }

    #[tokio::test]
    async fn cloud_access_is_one_atomic_registry_generation() {
        let state = AppState::new();
        assert!(current_cloud_access(&state).await.is_err());
        state
            .runtime_registry
            .set_cloud_account(Some(CloudAccountState {
                rest_base: "https://www.clarkchat.com".into(),
                account: AccountKey::new("account-a").unwrap(),
                token: zeroize::Zeroizing::new(unsigned_token("account-a")),
            }))
            .await;
        let access = current_cloud_access(&state).await.unwrap();
        assert_eq!(access.rest_base, "https://www.clarkchat.com");
        assert_eq!(access.owner_scope, "account-a");
        assert_eq!(jwt_subject(&access.token).unwrap(), access.owner_scope);
    }

    #[tokio::test]
    async fn sign_out_retires_only_the_active_account_generation() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        state
            .credentials
            .configure(root.path().join("credentials"))
            .unwrap();
        state
            .credentials
            .set_retained_auth(r#"{"token":"active-secret"}"#.into())
            .await
            .unwrap();
        state
            .credentials
            .set_code_key("account-a", "ck_live_account_a_secret".into())
            .await
            .unwrap();
        state
            .credentials
            .set_code_key("account-b", "ck_live_account_b_secret".into())
            .await
            .unwrap();
        let account = AccountKey::new("account-a").unwrap();
        state
            .runtime_registry
            .set_cloud_account(Some(CloudAccountState {
                rest_base: "https://www.clarkchat.com".into(),
                account: account.clone(),
                token: zeroize::Zeroizing::new(unsigned_token("account-a")),
            }))
            .await;
        state
            .runtime_registry
            .store_command_claim(
                account.clone(),
                "command-a".into(),
                "host-a".into(),
                "instance-a".into(),
                "claim-secret".into(),
            )
            .await
            .unwrap();
        let closed = Arc::new(AtomicBool::new(false));
        let session_id = SessionId::new("account-a-session");
        let session_key = SessionKey::from_session(&session_id).unwrap();
        state
            .runtime_registry
            .bind_session(
                Some(account.clone()),
                session_key.clone(),
                Arc::new(Mutex::new(HostSession {
                    account: Some(account.clone()),
                    provider: Box::new(CloseRecordingProvider(closed.clone())),
                    session: Session {
                        id: session_id.clone(),
                        provider: ProviderId::new("account-switch-test"),
                        capabilities: ProviderCapabilities::default(),
                        mode: None,
                        collaboration_mode: Default::default(),
                        environment: None,
                    },
                    snapshot: Snapshot::default(),
                    trajectory: None,
                    projection_gate: Arc::new(Mutex::new(())),
                    closing: false,
                })),
            )
            .await
            .unwrap();

        clear_cloud_authority(&state).await.unwrap();

        assert!(state.runtime_registry.cloud_account().await.is_none());
        assert!(state
            .runtime_registry
            .current_session_entry(&session_key)
            .await
            .is_none());
        assert!(closed.load(Ordering::SeqCst));
        assert!(state.credentials.retained_auth().await.unwrap().is_none());
        assert!(state
            .credentials
            .code_key("account-a")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            state
                .credentials
                .code_key("account-b")
                .await
                .unwrap()
                .unwrap()
                .as_str(),
            "ck_live_account_b_secret"
        );
        assert!(state
            .runtime_registry
            .command_claim(&account, "command-a", "host-a", "instance-a")
            .await
            .is_err());
    }
}
