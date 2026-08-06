mod browser;
mod device;
mod storage;

use std::io::IsTerminal;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

pub use storage::{CredentialSource, CredentialStore};

const PLATFORM_API_BASE: &str = "https://api.clarkslabs.com/v1";

#[derive(Debug, Serialize, Deserialize)]
pub struct Credential {
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    pub created_by: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogoutResult {
    NotSignedIn,
    RemovedExistingApiKey,
    RevokedMachineCredential,
}

impl Drop for Credential {
    fn drop(&mut self) {
        self.api_key.zeroize();
    }
}

#[derive(Clone, Debug, Deserialize)]
struct PlatformErrorEnvelope {
    error: Option<PlatformError>,
}

#[derive(Clone, Debug, Deserialize)]
struct PlatformError {
    message: Option<String>,
}

pub async fn login(method: crate::args::LoginMethod) -> Result<CredentialSource, String> {
    let credential = match method {
        crate::args::LoginMethod::Browser => browser::login().await?,
        crate::args::LoginMethod::DeviceCode => device::login().await?,
        crate::args::LoginMethod::ApiKey => api_key_login()?,
    };
    validate_api_key(&credential.api_key).await?;
    let context = crate::cloud::load_context(&credential.api_key, None).await?;
    context.authorize(crate::runtime::Workspace::Code)?;
    let source = CredentialStore::new()?.save(&credential)?;
    Ok(source)
}

pub fn require_credential() -> Result<Credential, String> {
    if let Ok(api_key) = std::env::var("CLARK_API_KEY") {
        if !api_key.trim().is_empty() {
            return validate_key_shape(Credential {
                api_key,
                account_email: None,
                api_key_id: None,
                created_by: "environment".into(),
            });
        }
    }
    CredentialStore::new()?
        .load()?
        .ok_or_else(missing_credential_message)
        .and_then(validate_key_shape)
}

pub async fn status() -> Result<(CredentialSource, Option<String>), String> {
    if let Ok(api_key) = std::env::var("CLARK_API_KEY") {
        if !api_key.trim().is_empty() {
            validate_api_key(&api_key).await?;
            return Ok((CredentialSource::Environment, None));
        }
    }
    let store = CredentialStore::new()?;
    let credential = store.load()?.ok_or_else(missing_credential_message)?;
    validate_api_key(&credential.api_key).await?;
    Ok((store.active_source(), credential.account_email.clone()))
}

pub async fn logout() -> Result<LogoutResult, String> {
    if std::env::var("CLARK_API_KEY")
        .ok()
        .is_some_and(|key| !key.trim().is_empty())
    {
        return Err("Clark is authenticated by CLARK_API_KEY. Unset that environment variable to sign out; no stored credential was changed.".into());
    }
    let store = CredentialStore::new()?;
    let Some(credential) = store.load()? else {
        return Ok(LogoutResult::NotSignedIn);
    };
    if credential.created_by == "api_key" {
        store.delete()?;
        return Ok(LogoutResult::RemovedExistingApiKey);
    }
    let response = clark_http::build_client(clark_http::ClientOptions {
        request_timeout: Some(std::time::Duration::from_secs(20)),
        user_agent: Some(concat!("clark-cli/", env!("CARGO_PKG_VERSION"))),
        ..Default::default()
    })
    .map_err(|error| format!("could not initialize Clark logout: {error}"))?
    .delete(format!("{}/cli/credential", platform_api_base()))
    .bearer_auth(&credential.api_key)
    .send()
    .await;
    let removed = store.delete()?;
    match response {
        Ok(response) if response.status().is_success() => {
            if removed {
                Ok(LogoutResult::RevokedMachineCredential)
            } else {
                Ok(LogoutResult::NotSignedIn)
            }
        }
        Ok(response) => Err(format!(
            "Removed the local Clark credential, but cloud revocation failed ({}). Revoke it at https://www.clarkchat.com/platform.",
            response.status()
        )),
        Err(error) => Err(format!(
            "Removed the local Clark credential, but cloud revocation could not be confirmed: {error}. Revoke it at https://www.clarkchat.com/platform."
        )),
    }
}

fn api_key_login() -> Result<Credential, String> {
    let api_key = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Clark API key: ")
            .map_err(|error| format!("could not read API key: {error}"))?
    } else {
        let mut value = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut value)
            .map_err(|error| format!("could not read API key from stdin: {error}"))?;
        value
    };
    validate_key_shape(Credential {
        api_key: api_key.trim().to_string(),
        account_email: None,
        api_key_id: None,
        created_by: "api_key".into(),
    })
}

fn validate_key_shape(credential: Credential) -> Result<Credential, String> {
    if !credential.api_key.starts_with("ck_live_") || credential.api_key.len() < 32 {
        return Err("Clark API keys start with ck_live_. Run `clark login` to create a machine credential, or generate an API key at https://www.clarkchat.com/platform.".into());
    }
    Ok(credential)
}

pub async fn validate_api_key(api_key: &str) -> Result<(), String> {
    let response = clark_http::build_client(clark_http::ClientOptions {
        request_timeout: Some(std::time::Duration::from_secs(20)),
        user_agent: Some(concat!("clark-cli/", env!("CARGO_PKG_VERSION"))),
        ..Default::default()
    })
    .map_err(|error| format!("could not initialize Clark network client: {error}"))?
    .get(format!("{}/models", platform_api_base()))
    .bearer_auth(api_key)
    .send()
    .await
    .map_err(|error| format!("could not reach Clark: {error}"))?;
    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<PlatformErrorEnvelope>(&body)
        .ok()
        .and_then(|body| body.error)
        .and_then(|error| error.message)
        .unwrap_or_else(|| body.chars().take(300).collect());
    Err(format!(
        "Clark rejected the stored credential ({status}): {detail}"
    ))
}

pub fn missing_credential_message() -> String {
    "Clark cloud credential is missing. Run `clark login` (or `clark login --device-code` over SSH). For CI only, set CLARK_API_KEY. Clark refuses to run specialists without cloud synchronization.".into()
}

pub(crate) fn auth_origin() -> String {
    if cfg!(debug_assertions) {
        if let Ok(origin) = std::env::var("CLARK_AUTH_ORIGIN") {
            if origin.starts_with("http://127.0.0.1:") || origin.starts_with("http://localhost:") {
                return origin.trim_end_matches('/').to_string();
            }
        }
    }
    "https://www.clarkchat.com".into()
}

pub(crate) fn platform_api_base() -> String {
    if cfg!(debug_assertions) {
        if let Ok(base) = std::env::var("CLARK_API_BASE_URL") {
            if base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:") {
                return base.trim_end_matches('/').to_string();
            }
        }
    }
    PLATFORM_API_BASE.into()
}

pub(crate) fn platform_api_origin() -> Result<String, String> {
    let mut url = url::Url::parse(&platform_api_base())
        .map_err(|error| format!("Clark Platform API URL is invalid: {error}"))?;
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_key_failure_names_every_supported_recovery() {
        let message = missing_credential_message();
        assert!(message.contains("clark login"));
        assert!(message.contains("--device-code"));
        assert!(message.contains("CLARK_API_KEY"));
        assert!(message.contains("cloud synchronization"));
    }

    #[test]
    fn invalid_key_is_rejected_before_network_use() {
        let result = validate_key_shape(Credential {
            api_key: "secret".into(),
            account_email: None,
            api_key_id: None,
            created_by: "test".into(),
        });
        assert!(result.unwrap_err().contains("ck_live_"));
    }
}
