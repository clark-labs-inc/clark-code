use super::cloud_authority::{clark_auth_base, clear_cloud_authority, jwt_subject};
use super::*;
use crate::runtime_registry::{AccountKey, CloudAccountState};
use tauri_plugin_google_auth::GoogleAuthExt;

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthUserDescriptor {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub avatar: Option<String>,
    pub method: String,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthDescriptor {
    pub user: AuthUserDescriptor,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetainedGoogleTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<i64>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetainedAuth {
    version: u32,
    descriptor: AuthDescriptor,
    auth_origin: String,
    clark_token: String,
    google: RetainedGoogleTokens,
}

fn retained_auth(raw: &str) -> Result<RetainedAuth, String> {
    let retained: RetainedAuth = serde_json::from_str(raw).map_err(|_| {
        "Clark's retained sign-in is obsolete or invalid; sign in again".to_string()
    })?;
    if retained.version != 2 {
        return Err("Clark's retained sign-in version is unsupported; sign in again".into());
    }
    Ok(retained)
}

async fn retain_native_auth(state: &AppState, retained: &RetainedAuth) -> Result<(), String> {
    let owner = jwt_subject(&retained.clark_token)?;
    if owner != retained.descriptor.user.id {
        return Err("Clark session identity did not match the signed-in account".into());
    }
    let rest_base = clark_auth_base(&retained.auth_origin)?;
    let account = AccountKey::new(owner)?;
    let _switch = state.account_lifecycle.write().await;
    if let Some(active) = state.runtime_registry.cloud_account().await {
        if active.account != account || active.rest_base != rest_base {
            return Err(
                "clark_account_mismatch: Sign out before connecting a different Clark account."
                    .into(),
            );
        }
    }
    let serialized = serde_json::to_string(retained)
        .map_err(|error| format!("could not retain Clark sign-in: {error}"))?;
    // Persist the complete credential generation before publishing it to live
    // callers. A disk failure leaves the prior account generation untouched.
    state.credentials.set_retained_auth(serialized).await?;
    state
        .runtime_registry
        .set_cloud_account(Some(CloudAccountState {
            rest_base,
            account,
            token: zeroize::Zeroizing::new(retained.clark_token.clone()),
        }))
        .await;
    Ok(())
}

/// Restore the one locally retained Clark sign-in. The encrypted file is the
/// sole persistent copy; the WebView receives only a non-secret descriptor.
#[tauri::command]
pub async fn clark_account_load(
    state: State<'_, AppState>,
) -> Result<Option<AuthDescriptor>, String> {
    let Some(raw) = state.credentials.retained_auth().await? else {
        return Ok(None);
    };
    let retained = retained_auth(raw.as_str())?;
    retain_native_auth(state.inner(), &retained).await?;
    Ok(Some(retained.descriptor))
}

#[tauri::command]
pub async fn clark_sign_out(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let access_token = state
        .credentials
        .retained_auth()
        .await?
        .and_then(|raw| retained_auth(raw.as_str()).ok())
        .map(|retained| retained.google.access_token);
    clear_cloud_authority(state.inner()).await?;
    let app = app.clone();
    let _ = tokio::task::spawn_blocking(move || {
        app.google_auth()
            .sign_out(tauri_plugin_google_auth::SignOutRequest {
                access_token,
                flow_type: None,
            })
    })
    .await;
    Ok(())
}

async fn exchange_google_id_token(
    auth_origin: &str,
    id_token: &str,
) -> Result<(AuthDescriptor, String), String> {
    let base = clark_auth_base(auth_origin)?;
    let client = clark_http::build_client(clark_http::ClientOptions {
        cookie_store: true,
        ..Default::default()
    })
    .map_err(|e| e.to_string())?;

    // 1. Trade the Google ID token for a Clark session (sets the session cookie
    //    on this client's jar).
    let signin = client
        .post(format!("{base}/api/auth/sign-in/social"))
        .json(&serde_json::json!({
            "provider": "google",
            "idToken": { "token": id_token },
        }))
        .send()
        .await
        .map_err(|e| format!("sign-in request failed: {e}"))?;
    if !signin.status().is_success() {
        let status = signin.status();
        let body = signin.text().await.unwrap_or_default();
        return Err(format!(
            "Clark rejected the Google sign-in ({status}): {body}"
        ));
    }
    let signin_body: Value = signin.json().await.unwrap_or(Value::Null);

    // Prefer the user echoed by sign-in; fall back to get-session if absent.
    let mut user = signin_body.get("user").cloned().unwrap_or(Value::Null);
    if user
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .is_empty()
    {
        if let Ok(resp) = client
            .get(format!("{base}/api/auth/get-session"))
            .send()
            .await
        {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(u) = body.get("user") {
                    user = u.clone();
                }
            }
        }
    }

    // 2. Fetch the bearer JWT the gateway validates on the WebSocket handshake.
    let token_resp = client
        .get(format!("{base}/api/auth/token"))
        .send()
        .await
        .map_err(|e| format!("token request failed: {e}"))?;
    if !token_resp.status().is_success() {
        return Err(format!(
            "Clark token bootstrap failed ({})",
            token_resp.status()
        ));
    }
    let token_body: Value = token_resp.json().await.map_err(|e| e.to_string())?;
    let token = token_body
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if token.is_empty() {
        return Err("Clark returned an empty session token".into());
    }

    let str_field = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    let id = str_field(&user, "id")
        .ok_or_else(|| "Clark sign-in returned no account identity".to_string())?;
    let subject = jwt_subject(&token)?;
    if subject != id {
        return Err("Clark session identity did not match the signed-in account".into());
    }
    Ok((
        AuthDescriptor {
            user: AuthUserDescriptor {
                id,
                name: str_field(&user, "name")
                    .or_else(|| str_field(&user, "email"))
                    .unwrap_or_else(|| "Google user".into()),
                email: str_field(&user, "email"),
                avatar: str_field(&user, "image"),
                method: "google".into(),
            },
        },
        token,
    ))
}

const AUTH_SUCCESS_HTML: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Signed in to Clark Code</title></head><body><main><h1>You're signed in</h1><p>You can close this tab and return to Clark Code.</p><a href="clark://auth-complete">Return to Clark Code</a></main><script>setTimeout(function(){location.replace('clark://auth-complete')},500)</script></body></html>"#;

fn google_client_config() -> Result<(String, Option<String>), String> {
    let client_id = option_env!("CLARK_GOOGLE_DESKTOP_CLIENT_ID")
        .filter(|value| !value.trim().is_empty())
        .ok_or("Google sign-in is not configured in this Clark Desktop build")?;
    let client_secret = option_env!("CLARK_GOOGLE_DESKTOP_CLIENT_SECRET")
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    Ok((client_id.to_string(), client_secret))
}

fn configured_auth_origin() -> Result<String, String> {
    clark_auth_base(option_env!("CLARK_AUTH_ORIGIN").unwrap_or("https://www.clarkchat.com"))
}

#[tauri::command]
pub async fn clark_google_sign_in(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AuthDescriptor, String> {
    let auth_origin = configured_auth_origin()?;
    let (client_id, client_secret) = google_client_config()?;
    let google = tokio::task::spawn_blocking(move || {
        app.google_auth()
            .sign_in(tauri_plugin_google_auth::SignInRequest {
                client_id,
                client_secret,
                scopes: Some(vec!["openid".into(), "email".into(), "profile".into()]),
                hosted_domain: None,
                login_hint: None,
                redirect_uri: Some("http://127.0.0.1".into()),
                success_html_response: Some(AUTH_SUCCESS_HTML.into()),
                flow_type: None,
            })
    })
    .await
    .map_err(|error| format!("Google sign-in task failed: {error}"))?
    .map_err(|error| format!("Google sign-in failed: {error}"))?;
    let id_token = google
        .id_token
        .as_deref()
        .ok_or("Google did not return an ID token")?;
    let (descriptor, clark_token) = exchange_google_id_token(&auth_origin, id_token).await?;
    let retained = RetainedAuth {
        version: 2,
        descriptor: descriptor.clone(),
        auth_origin,
        clark_token,
        google: RetainedGoogleTokens {
            access_token: google.access_token,
            refresh_token: google.refresh_token,
            expires_at: google.expires_at,
        },
    };
    retain_native_auth(state.inner(), &retained).await?;
    Ok(descriptor)
}

#[tauri::command]
pub async fn clark_refresh_cloud_session(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AuthDescriptor, String> {
    let raw = state
        .credentials
        .retained_auth()
        .await?
        .ok_or("Clark has no retained sign-in")?;
    let mut retained = retained_auth(raw.as_str())?;
    let refresh_token = retained
        .google
        .refresh_token
        .clone()
        .ok_or("Google did not provide a refresh token; sign in again")?;
    let (client_id, client_secret) = google_client_config()?;
    let google = tokio::task::spawn_blocking(move || {
        app.google_auth()
            .refresh_token(tauri_plugin_google_auth::RefreshTokenRequest {
                refresh_token: Some(refresh_token),
                client_id,
                client_secret,
                scopes: Some(vec!["openid".into(), "email".into(), "profile".into()]),
                flow_type: None,
            })
    })
    .await
    .map_err(|error| format!("Google refresh task failed: {error}"))?
    .map_err(|error| format!("Google refresh failed: {error}"))?;
    let id_token = google
        .id_token
        .as_deref()
        .ok_or("Google did not refresh the ID token")?;
    let (descriptor, clark_token) =
        exchange_google_id_token(&retained.auth_origin, id_token).await?;
    retained.descriptor = descriptor.clone();
    retained.clark_token = clark_token;
    retained.google.access_token = google.access_token;
    retained.google.refresh_token = google.refresh_token.or(retained.google.refresh_token);
    retained.google.expires_at = google.expires_at;
    retain_native_auth(state.inner(), &retained).await?;
    Ok(descriptor)
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;

    use super::{
        retain_native_auth, AuthDescriptor, AuthUserDescriptor, RetainedAuth, RetainedGoogleTokens,
    };
    use crate::AppState;

    fn token(subject: &str) -> String {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::json!({ "sub": subject }).to_string());
        format!("header.{payload}.signature")
    }

    fn retained(subject: &str) -> RetainedAuth {
        RetainedAuth {
            version: 2,
            descriptor: AuthDescriptor {
                user: AuthUserDescriptor {
                    id: subject.into(),
                    name: "Clark user".into(),
                    email: Some("user@example.test".into()),
                    avatar: None,
                    method: "google".into(),
                },
            },
            auth_origin: "https://www.clarkchat.com".into(),
            clark_token: token(subject),
            google: RetainedGoogleTokens {
                access_token: "google-access-secret".into(),
                refresh_token: Some("google-refresh-secret".into()),
                expires_at: Some(42),
            },
        }
    }

    #[test]
    fn renderer_descriptor_serializes_no_credential_fields() {
        let value = serde_json::to_value(retained("account-a").descriptor).unwrap();
        let encoded = value.to_string();
        assert!(!encoded.contains("token"));
        assert!(!encoded.contains("secret"));
        assert_eq!(value["user"]["id"], "account-a");
        assert!(value.get("clark").is_none());
    }

    #[tokio::test]
    async fn retained_auth_publishes_one_account_generation_and_rejects_switches() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        state
            .credentials
            .configure(root.path().join("credentials"))
            .unwrap();
        let first = retained("account-a");
        retain_native_auth(&state, &first).await.unwrap();
        let active = state.runtime_registry.cloud_account().await.unwrap();
        assert_eq!(active.account.as_str(), "account-a");
        assert_eq!(active.token.as_str(), first.clark_token);

        let error = retain_native_auth(&state, &retained("account-b"))
            .await
            .unwrap_err();
        assert!(error.starts_with("clark_account_mismatch:"));
        let active = state.runtime_registry.cloud_account().await.unwrap();
        assert_eq!(active.account.as_str(), "account-a");
        let disk = state.credentials.retained_auth().await.unwrap().unwrap();
        assert!(disk.contains("account-a"));
        assert!(!disk.contains("account-b"));
    }
}
