use base64::Engine as _;
use serde_json::Value;

use crate::state::{AppState, CloudAccountAuthority};

const CLARK_HOSTS: &[&str] = &["www.clarkchat.com", "dev.clarkslabs.com"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CloudAccess {
    pub rest_base: String,
    pub owner_scope: String,
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

/// Shared authenticated client. Redirects are disabled so a Clark bearer or
/// Google ID token can never follow a server response onto another origin.
static CLOUD_HTTP: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build cloud http client")
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

pub(crate) async fn install_cloud_authority(
    state: &AppState,
    rest_base: String,
    token: String,
    owner_scope: String,
) {
    let _bootstrap = state.cloud_bootstrap.lock().await;
    *state.cloud_authority.write().await = Some(CloudAccountAuthority {
        rest_base,
        owner_scope,
    });
    *state.cloud_token.write().await = Some(token);
}

pub(crate) async fn clear_cloud_authority(state: &AppState, token: &str) {
    let _bootstrap = state.cloud_bootstrap.lock().await;
    if state.cloud_token.read().await.as_deref() != Some(token) {
        return;
    }
    *state.cloud_authority.write().await = None;
    *state.cloud_token.write().await = None;
}

/// Rotate a native bearer after a WebView refresh without changing the
/// account-bound local outbox partition.
pub(crate) async fn refresh_cloud_authority(state: &AppState, token: &str) -> Result<(), String> {
    let _bootstrap = state.cloud_bootstrap.lock().await;
    let owner_scope = jwt_subject(token)?;
    let authority = state
        .cloud_authority
        .read()
        .await
        .clone()
        .ok_or("Clark has no active signed-in account")?;
    if owner_scope != authority.owner_scope {
        return Err("Clark session refresh belongs to a different account".into());
    }
    *state.cloud_token.write().await = Some(token.to_string());
    Ok(())
}

/// Return the native account binding for these credentials. On a cold process,
/// the first request is authenticated against the exact Clark origin before the
/// JWT subject is accepted as the cache partition. Once bound, only the native
/// sign-in exchange may switch accounts; caller-provided owner labels are never
/// trusted.
pub(crate) async fn require_cloud_access(
    state: &AppState,
    endpoint: &str,
    token: &str,
) -> Result<CloudAccess, String> {
    let rest_base = clark_rest_base(endpoint)?;
    let _bootstrap = state.cloud_bootstrap.lock().await;

    if let Some(authority) = state.cloud_authority.read().await.clone() {
        let current_token = state.cloud_token.read().await;
        if authority.rest_base != rest_base || current_token.as_deref() != Some(token) {
            return Err("Clark credentials do not match the active signed-in account".into());
        }
        return Ok(CloudAccess {
            rest_base: authority.rest_base,
            owner_scope: authority.owner_scope,
        });
    }

    let response = clark_http_client()?
        .get(format!("{rest_base}/api/desktop/conversations?limit=1"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| format!("could not validate the signed-in Clark account: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Clark account validation failed ({})",
            response.status()
        ));
    }

    // The payload is decoded only after the exact Clark origin accepted the
    // bearer. The server's signature validation is the source of trust.
    let owner_scope = jwt_subject(token)?;
    *state.cloud_authority.write().await = Some(CloudAccountAuthority {
        rest_base: rest_base.clone(),
        owner_scope: owner_scope.clone(),
    });
    *state.cloud_token.write().await = Some(token.to_string());
    Ok(CloudAccess {
        rest_base,
        owner_scope,
    })
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{
        clark_auth_base, clark_rest_base, install_cloud_authority, jwt_subject,
        refresh_cloud_authority, require_cloud_access,
    };
    use crate::AppState;

    fn unsigned_token(subject: &str) -> String {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::json!({ "sub": subject }).to_string());
        format!("header.{payload}.signature")
    }

    #[test]
    fn bearer_destinations_are_exact_clark_origins() {
        assert_eq!(
            clark_rest_base("wss://www.clarkchat.com/ws").unwrap(),
            "https://www.clarkchat.com"
        );
        assert_eq!(
            clark_auth_base("https://dev.clarkslabs.com").unwrap(),
            "https://dev.clarkslabs.com"
        );
        for endpoint in [
            "https://attacker.example/ws",
            "https://www.clarkchat.com.attacker.example/ws",
            "https://attacker.example@www.clarkchat.com/ws",
            "https://www.clarkchat.com/ws?next=https://attacker.example",
            "http://www.clarkchat.com/ws",
        ] {
            assert!(clark_rest_base(endpoint).is_err(), "{endpoint}");
        }
    }

    #[test]
    fn cache_subject_is_taken_from_the_validated_jwt() {
        assert_eq!(
            jwt_subject(&unsigned_token("account-123")).unwrap(),
            "account-123"
        );
        assert!(jwt_subject("not-a-jwt").is_err());
    }

    #[tokio::test]
    async fn first_validated_account_owns_the_native_cache_partition() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}/ws", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let count = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            assert!(request.contains("GET /api/desktop/conversations?limit=1"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]")
                .await
                .unwrap();
        });

        let state = AppState::new();
        let token = unsigned_token("account-a");
        let access = require_cloud_access(&state, &endpoint, &token)
            .await
            .unwrap();
        assert_eq!(access.owner_scope, "account-a");
        assert!(
            require_cloud_access(&state, &endpoint, &unsigned_token("account-b"))
                .await
                .is_err()
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn same_account_refresh_rotates_the_native_token_without_switching_owner() {
        let state = AppState::new();
        let first = unsigned_token("account-a");
        let refreshed = format!("{}.refreshed", unsigned_token("account-a"));
        install_cloud_authority(
            &state,
            "https://www.clarkchat.com".into(),
            first,
            "account-a".into(),
        )
        .await;
        refresh_cloud_authority(&state, &refreshed).await.unwrap();
        assert_eq!(state.cloud_token.read().await.as_deref(), Some(refreshed.as_str()));
        assert!(refresh_cloud_authority(&state, &unsigned_token("account-b"))
            .await
            .is_err());
        assert_eq!(state.cloud_token.read().await.as_deref(), Some(refreshed.as_str()));
    }

    #[tokio::test]
    async fn authenticated_client_does_not_follow_cross_origin_redirects() {
        let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}/ws", source.local_addr().unwrap());
        let location = format!("http://{}/stolen", destination.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = source.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let state = AppState::new();
        assert!(
            require_cloud_access(&state, &endpoint, &unsigned_token("account-a"))
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), destination.accept())
                .await
                .is_err()
        );
        server.await.unwrap();
    }
}
