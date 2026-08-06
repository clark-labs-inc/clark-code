use std::time::Duration;

use rand::RngCore;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use super::{auth_origin, Credential};

#[derive(Debug, Deserialize)]
struct CreateKeyResponse {
    key: String,
    api_key: ApiKeySummary,
}

#[derive(Debug, Deserialize)]
struct ApiKeySummary {
    id: String,
}

pub async fn login() -> Result<Credential, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("could not open the Clark sign-in callback: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("could not inspect the Clark sign-in callback: {error}"))?
        .port();
    let state = random_state();
    let url = format!(
        "{}/desktop-auth?port={port}&client=cli&state={state}",
        auth_origin()
    );
    println!("Finish signing in via your browser:\n\n{url}\n");
    println!("On a remote or headless machine, press Ctrl+C and run `clark login --device-code`.");
    if let Err(error) = webbrowser::open(&url) {
        eprintln!("Clark could not open the browser automatically: {error}");
    }
    let (token, email) = tokio::time::timeout(Duration::from_secs(5 * 60), async {
        loop {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|error| format!("Clark sign-in callback failed: {error}"))?;
            let mut buffer = vec![0_u8; 16 * 1024];
            let count = stream
                .read(&mut buffer)
                .await
                .map_err(|error| format!("could not read Clark sign-in callback: {error}"))?;
            let first_line = String::from_utf8_lossy(&buffer[..count])
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            let Some(target) = first_line
                .strip_prefix("GET ")
                .and_then(|line| line.split_whitespace().next())
            else {
                write_callback(&mut stream, false).await;
                continue;
            };
            let parsed = Url::parse(&format!("http://127.0.0.1:{port}{target}"))
                .map_err(|_| "Clark sign-in returned an invalid callback URL".to_string())?;
            let value = |name: &str| {
                parsed
                    .query_pairs()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.into_owned())
            };
            if value("state").as_deref() != Some(state.as_str()) {
                write_callback(&mut stream, false).await;
                continue;
            }
            let token = value("token")
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "Clark sign-in returned no session token".to_string())?;
            let email = value("email").filter(|value| !value.trim().is_empty());
            write_callback(&mut stream, true).await;
            break Ok::<(String, Option<String>), String>((token, email));
        }
    })
    .await
    .map_err(|_| "Clark browser sign-in expired after five minutes".to_string())??;
    provision_key(&token, email).await
}

async fn provision_key(token: &str, email: Option<String>) -> Result<Credential, String> {
    let response = clark_http::build_client(clark_http::ClientOptions {
        request_timeout: Some(Duration::from_secs(30)),
        user_agent: Some(concat!("clark-cli/", env!("CARGO_PKG_VERSION"))),
        ..Default::default()
    })
    .map_err(|error| format!("could not initialize Clark network client: {error}"))?
    .post(format!("{}/api/platform/api-keys", auth_origin()))
    .bearer_auth(token)
    .json(&serde_json::json!({
        "name": hostname_key_name(),
        "purpose": "clark_code_desktop",
    }))
    .send()
    .await
    .map_err(|error| format!("could not provision this machine's Clark key: {error}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Clark could not provision this machine's credential ({status}): {}",
            body.chars().take(500).collect::<String>()
        ));
    }
    let created: CreateKeyResponse = serde_json::from_str(&body)
        .map_err(|error| format!("Clark returned an invalid credential response: {error}"))?;
    Ok(Credential {
        api_key: created.key,
        account_email: email,
        api_key_id: Some(created.api_key.id),
        created_by: "browser".into(),
    })
}

async fn write_callback(stream: &mut tokio::net::TcpStream, success: bool) {
    let (status, title, message) = if success {
        (
            "200 OK",
            "Signed in to Clark",
            "You can close this tab and return to your terminal.",
        )
    } else {
        (
            "400 Bad Request",
            "Clark sign-in was not accepted",
            "Return to your terminal and try again.",
        )
    };
    let body = format!("<!doctype html><meta charset=utf-8><title>{title}</title><style>body{{font:16px system-ui;background:#111;color:#eee;display:grid;place-items:center;min-height:90vh}}main{{max-width:32rem;text-align:center}}</style><main><h1>{title}</h1><p>{message}</p></main>");
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

fn hostname_key_name() -> String {
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "terminal".into());
    let hostname = hostname
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(40)
        .collect::<String>();
    format!(
        "Clark CLI ({})",
        if hostname.is_empty() {
            "terminal"
        } else {
            &hostname
        }
    )
}

fn random_state() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_state_is_high_entropy_and_url_safe() {
        let state = random_state();
        assert_eq!(state.len(), 64);
        assert!(state.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
