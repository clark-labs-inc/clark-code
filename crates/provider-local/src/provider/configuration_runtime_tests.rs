use super::*;

#[test]
fn replacing_memory_is_idempotent() {
    let mut prompt = "base\n# Memory\nold".to_string();
    replace_memory_section(&mut prompt, Some("\n# Memory\nnew"));
    assert_eq!(prompt, "base\n# Memory\nnew");
    replace_memory_section(&mut prompt, None);
    assert_eq!(prompt, "base");
}

#[test]
fn image_relay_requires_a_key_and_preserves_host_model_exclusions() {
    let mut config = LocalConfig::from_provider_config(&ProviderConfig::default());
    config.api_key = None;
    assert!(image_generation_config(&config, "host-standard").is_none());
    config.api_key = Some("test-platform-key".into());
    config.image_generation_excluded_models = vec!["host-disabled".into()];
    assert!(image_generation_config(&config, "host-disabled").is_none());
    let image = image_generation_config(&config, "host-standard").unwrap();
    assert_eq!(image.model, "host-standard");
    assert_eq!(image.base_url, config.base_url);
}

#[tokio::test]
async fn image_relay_source_model_tracks_initial_selection_and_model_changes() {
    let (base_url, requests) = image_relay().await;
    let directory = tempfile::tempdir().unwrap();
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            auth_token: Some("test-platform-key".into()),
            extra: serde_json::json!({
                "base_url": base_url,
                "model": "host-standard",
                "models": [
                    {"id": "host-standard", "label": "Standard", "description": "First model"},
                    {"id": "host-large", "label": "Large", "description": "Second model"},
                    {"id": "host-disabled", "label": "Disabled", "description": "Images excluded"}
                ],
                "image_generation_excluded_models": ["host-disabled"],
                "memories": false,
                "sandbox_mode": "disabled"
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(directory.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await
        .unwrap();

    for (index, model) in ["host-standard", "host-large"].into_iter().enumerate() {
        if index != 0 {
            provider
                .configure(
                    &session.id,
                    ProviderConfigurationChange::Model { model: model.into() },
                )
                .await
                .unwrap();
        }
        let outcome = provider
            .tool("generate_image")
            .unwrap()
            .invoke(
                serde_json::json!({
                    "prompt": "A blue circle",
                    "output_path": format!("images/selection-{index}.png"),
                    // Model-authored arguments cannot override host billing identity.
                    "source_model": "untrusted-argument",
                    "model": "untrusted-image-model"
                }),
                &provider.tool_ctx().unwrap(),
            )
            .await;
        assert!(!outcome.is_error, "{}", outcome.content);
        assert!(directory
            .path()
            .join(format!("images/selection-{index}.png"))
            .is_file());
    }

    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 2);
    for (request, model) in requests.iter().zip(["host-standard", "host-large"]) {
        assert_eq!(request["source_model"], model);
        assert_eq!(request["prompt"], "A blue circle");
        assert_eq!(request["input_images"], serde_json::json!([]));
        assert!(request.get("model").is_none());
    }

    for (model, available) in [("host-disabled", false), ("host-standard", true)] {
        provider
            .configure(
                &session.id,
                ProviderConfigurationChange::Model { model: model.into() },
            )
            .await
            .unwrap();
        assert_eq!(provider.tool("generate_image").is_some(), available);
    }
}

async fn image_relay() -> (String, tokio::task::JoinHandle<Vec<serde_json::Value>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let body_start = loop {
                let mut chunk = [0; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                assert!(read > 0, "request ended before its headers");
                request.extend_from_slice(&chunk[..read]);
                if let Some(position) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break position + 4;
                }
            };
            let headers = std::str::from_utf8(&request[..body_start]).unwrap();
            assert!(headers.starts_with("POST /v1/images/generations HTTP/1.1\r\n"));
            assert!(headers
                .to_ascii_lowercase()
                .contains("authorization: bearer test-platform-key"));
            assert!(headers.to_ascii_lowercase().contains("idempotency-key:"));
            let content_length: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while request.len() < body_start + content_length {
                let mut chunk = [0; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                assert!(read > 0, "request ended before its body");
                request.extend_from_slice(&chunk[..read]);
            }
            requests.push(serde_json::from_slice(&request[body_start..]).unwrap());
            let body = serde_json::json!({"data": [{
                "b64_json": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                "media_type": "image/png"
            }]}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    (format!("http://{address}/v1"), requests)
}
