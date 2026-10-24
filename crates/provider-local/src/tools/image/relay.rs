//! Authenticated image relay transport and raster-response validation.

use std::time::Duration;

use base64::Engine as _;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::json;

use super::{detect_image_mime, normalize_image_mime, GenerateImage};

const MAX_OUTPUT_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

impl GenerateImage {
    pub(super) async fn generate(
        &self,
        prompt: &str,
        input_images: Vec<String>,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<GeneratedImage, String> {
        let client = desktop_http::build_client(desktop_http::ClientOptions {
            request_timeout: Some(REQUEST_TIMEOUT),
            user_agent: Some("agent-desktop"),
            ..Default::default()
        })
        .map_err(|error| format!("creating image client: {error}"))?;
        let url = generation_url(&self.config.base_url);
        let request = client
            .post(url)
            .bearer_auth(&self.config.api_key)
            .header("idempotency-key", uuid::Uuid::new_v4().to_string())
            .json(&json!({
                "source_model": self.config.model,
                "prompt": prompt,
                "input_images": input_images,
            }));
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err("image generation cancelled".to_string()),
            response = request.send() => response.map_err(|error| format!("calling Clark Code image generation: {error}"))?,
        };
        parse_generation_response(response).await
    }
}

#[derive(Deserialize)]
struct ImageGenerationResponse {
    #[serde(default)]
    data: Vec<ImageGenerationData>,
}

#[derive(Deserialize)]
pub(super) struct ImageGenerationData {
    pub(super) b64_json: String,
    #[serde(default, alias = "mime_type")]
    pub(super) media_type: Option<String>,
}

pub(super) struct GeneratedImage {
    pub(super) bytes: Vec<u8>,
    pub(super) mime_type: &'static str,
}

async fn parse_generation_response(response: reqwest::Response) -> Result<GeneratedImage, String> {
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("reading Clark Code image response: {error}"))?;
    if status != StatusCode::OK {
        return Err(format!(
            "Clark Code image generation returned HTTP {status}: {}",
            compact_response(&body)
        ));
    }
    let parsed: ImageGenerationResponse = serde_json::from_slice(&body).map_err(|error| {
        format!(
            "Clark Code image generation returned an invalid response ({error}): {}",
            compact_response(&body)
        )
    })?;
    let image = parsed
        .data
        .into_iter()
        .next()
        .ok_or_else(|| "Clark Code image generation completed without an image".to_string())?;
    decode_generated_image(image)
}

pub(super) fn decode_generated_image(image: ImageGenerationData) -> Result<GeneratedImage, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image.b64_json.as_bytes())
        .map_err(|error| format!("decoding generated image: {error}"))?;
    if bytes.len() > MAX_OUTPUT_IMAGE_BYTES {
        return Err(format!(
            "generated image is too large (max {MAX_OUTPUT_IMAGE_BYTES} bytes)"
        ));
    }
    let detected = detect_image_mime(&bytes).ok_or_else(|| {
        "Clark Code returned bytes that are not a supported raster image".to_string()
    })?;
    if let Some(reported) = image.media_type.as_deref() {
        let reported = normalize_image_mime(reported).ok_or_else(|| {
            "Clark Code returned an unsupported generated image MIME type".to_string()
        })?;
        if reported != detected {
            return Err(format!(
                "Clark Code returned inconsistent generated image MIME types ({reported} vs {detected})"
            ));
        }
    }
    Ok(GeneratedImage {
        bytes,
        mime_type: detected,
    })
}

pub(super) fn generation_url(base_url: &str) -> String {
    format!("{}/images/generations", base_url.trim_end_matches('/'))
}

fn compact_response(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .take(300)
        .collect::<String>()
        .replace('\n', " ")
}
