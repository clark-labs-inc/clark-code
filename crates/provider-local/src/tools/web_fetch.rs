//! `web_fetch` — fetch a URL locally and return it as markdown. The sanctioned
//! local alternative to `clark_research` for simple page/doc lookups: no Clark
//! credits, no round trip, but also no JS rendering or search — just HTTP GET
//! + HTML→Markdown.
//!
//! Guards against SSRF: the target host is resolved and validated as a public
//! IP *before* connecting, and the connection is pinned to that validated
//! address (`reqwest::ClientBuilder::resolve`) so a second DNS lookup at
//! connect time can't rebind to an internal address. Redirects are followed
//! manually, re-validating the target at each hop, for the same reason.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{arg_str, ToolCtx, ToolExecutor, ToolOutcome};
use crate::config::ClarkResearchConfig;
use crate::llm::LlmClient;

/// Hard cap on the fetched page body.
const MAX_BYTES: usize = 3_000_000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: u8 = 5;
/// Pages longer than this (in characters, post-conversion) get condensed
/// through a cheap model rather than returned raw.
const CONDENSE_THRESHOLD_CHARS: usize = 12_000;

const CONDENSE_SYSTEM: &str = "Condense the following fetched web page for a coding agent's use: \
keep key facts, code snippets, API signatures, and direct answers; drop navigation/boilerplate/ads. \
Be thorough but not verbose.";

pub struct WebFetchTool {
    /// A cheap-model client for condensing long pages, when research is
    /// configured. `None` means long pages are returned truncated instead.
    condense: Option<LlmClient>,
}

impl WebFetchTool {
    pub fn new(clark: Option<ClarkResearchConfig>) -> Self {
        let condense = clark.and_then(|c| {
            LlmClient::from_parts(&c.base_url, &c.model, c.api_key, Vec::new(), None).ok()
        });
        Self { condense }
    }
}

#[async_trait]
impl ToolExecutor for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }
    fn description(&self) -> &str {
        "Fetch a URL over HTTP(S) and return its content as markdown. Local and direct — use it \
        for simple page/doc lookups. For anything needing search, JS-rendered pages, or multi-step \
        browsing, use clark_research instead. Never fetch URLs with `bash` — use this tool."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "The absolute http(s) URL to fetch."}
            },
            "required": ["url"]
        })
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Fetch
    }
    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let url = match arg_str(&args, "url") {
            Ok(u) => u,
            Err(e) => return ToolOutcome::error(e),
        };
        match fetch_markdown(&url, &ctx.cancel).await {
            Ok(md) if md.chars().count() > CONDENSE_THRESHOLD_CHARS => {
                if let Some(client) = &self.condense {
                    if let Ok(summary) = client
                        .complete(Some(CONDENSE_SYSTEM), &md, &ctx.cancel)
                        .await
                    {
                        if !summary.is_empty() {
                            return ToolOutcome::ok(summary);
                        }
                    }
                }
                let truncated: String = md.chars().take(CONDENSE_THRESHOLD_CHARS).collect();
                ToolOutcome::ok(format!("{truncated}\n\n…(truncated; the page is longer)"))
            }
            Ok(md) => ToolOutcome::ok(md),
            Err(e) => ToolOutcome::error(e),
        }
    }
}

async fn fetch_markdown(url_str: &str, cancel: &CancellationToken) -> Result<String, String> {
    let mut current = url_str.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let url = reqwest::Url::parse(&current).map_err(|e| format!("invalid URL: {e}"))?;
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err("only http/https URLs are allowed".to_string());
        }
        let host = url.host_str().ok_or("URL has no host")?.to_string();
        let port = url.port_or_known_default().unwrap_or(443);

        let addr = tokio::select! {
            _ = cancel.cancelled() => return Err("cancelled".to_string()),
            res = resolve_and_validate(&host, port) => res?,
        };

        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .resolve(&host, addr)
            .build()
            .map_err(|e| e.to_string())?;

        let resp = tokio::select! {
            _ = cancel.cancelled() => return Err("cancelled".to_string()),
            res = client.get(url.clone()).send() => res.map_err(|e| e.to_string())?,
        };

        if resp.status().is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("redirect with no Location header")?
                .to_string();
            current = url
                .join(&location)
                .map_err(|e| format!("bad redirect target: {e}"))?
                .to_string();
            continue;
        }
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_BYTES {
                return Err(format!("page too large ({len} bytes, cap is {MAX_BYTES})"));
            }
        }

        let bytes = read_capped(resp, cancel).await?;
        let html = String::from_utf8_lossy(&bytes).to_string();
        return htmd::convert(&html).map_err(|e| format!("HTML->Markdown conversion failed: {e}"));
    }
    Err("too many redirects".to_string())
}

/// Resolve `host` and return the first address that isn't private/internal.
/// Fails closed: a host that resolves to nothing but private/internal
/// addresses (or fails to resolve at all) is refused.
async fn resolve_and_validate(host: &str, port: u16) -> Result<SocketAddr, String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("DNS resolution failed: {e}"))?
        .collect();
    addrs
        .into_iter()
        .find(|a| is_public_ip(a.ip()))
        .ok_or_else(|| {
            format!("`{host}` resolves only to private/internal addresses — refusing to fetch")
        })
}

/// Whether `ip` is a normal public internet address — i.e. NOT loopback,
/// private (RFC1918/RFC4193), link-local, unspecified, multicast, broadcast,
/// or documentation/reserved. Used to stop the agent fetching internal
/// network services or cloud-metadata endpoints (e.g. `169.254.169.254`).
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast())
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (v6.segments()[0] & 0xffc0) == 0xfe80) // link-local fe80::/10
        }
    }
}

/// Read the response body, truncating at `MAX_BYTES` instead of erroring —
/// a friendlier failure mode than rejecting the whole fetch.
async fn read_capped(
    resp: reqwest::Response,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, String> {
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err("cancelled".to_string()),
            next = stream.next() => next,
        };
        let Some(chunk) = chunk else { break };
        buf.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
        if buf.len() > MAX_BYTES {
            buf.truncate(MAX_BYTES);
            break;
        }
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_advertises_read_only_fetch_schema() {
        let t = WebFetchTool::new(None);
        assert_eq!(t.name(), "web_fetch");
        assert!(!t.mutating());
        assert_eq!(t.kind(), ToolKind::Fetch);
        assert_eq!(t.parameters()["required"][0], "url");
    }

    #[test]
    fn public_ips_are_allowed() {
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap())); // Cloudflare v6
    }

    #[test]
    fn private_loopback_and_link_local_ranges_are_blocked() {
        assert!(!is_public_ip("127.0.0.1".parse().unwrap()));
        assert!(!is_public_ip("10.0.0.1".parse().unwrap()));
        assert!(!is_public_ip("172.16.0.1".parse().unwrap()));
        assert!(!is_public_ip("192.168.1.1".parse().unwrap()));
        // Cloud-metadata endpoint (AWS/GCP/Azure) — link-local.
        assert!(!is_public_ip("169.254.169.254".parse().unwrap()));
        assert!(!is_public_ip("0.0.0.0".parse().unwrap()));
        assert!(!is_public_ip("::1".parse().unwrap()));
        assert!(!is_public_ip("fe80::1".parse().unwrap()));
        assert!(!is_public_ip("fc00::1".parse().unwrap()));
    }

    #[tokio::test]
    async fn fetch_rejects_a_loopback_url_before_connecting() {
        let err = fetch_markdown("http://127.0.0.1:1/", &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(err.contains("private/internal"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_rejects_non_http_schemes() {
        let err = fetch_markdown("file:///etc/passwd", &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(err.contains("http"), "got: {err}");
    }
}
