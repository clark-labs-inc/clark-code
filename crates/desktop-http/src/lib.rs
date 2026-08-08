//! One transport policy for Agent Desktop's Reqwest clients.
//!
//! Callers still own endpoint-specific request semantics and retry policy. This
//! crate owns connection reuse, HTTP/2 negotiation, timeouts, redirects, and
//! cookie-jar construction so model, platform, and cloud clients do not drift.

use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const TCP_KEEPALIVE: Duration = Duration::from_secs(30);
const MAX_IDLE_CONNECTIONS_PER_HOST: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedirectPolicy {
    /// Authenticated and manually validated requests must never redirect.
    None,
    /// Public downloads may follow a small, explicit number of redirects.
    Limited(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientOptions<'a> {
    pub request_timeout: Option<Duration>,
    pub redirect_policy: RedirectPolicy,
    pub cookie_store: bool,
    pub user_agent: Option<&'a str>,
}

impl Default for ClientOptions<'_> {
    fn default() -> Self {
        Self {
            request_timeout: None,
            redirect_policy: RedirectPolicy::None,
            cookie_store: false,
            user_agent: None,
        }
    }
}

/// Start a Reqwest builder with Agent Desktop's shared connection policy.
///
/// HTTP/2 remains negotiated rather than forced so localhost, test servers,
/// and HTTP/1-only APIs continue to work. Adaptive HTTP/2 flow control and a
/// reusable per-host pool improve long streaming turns without changing wire
/// semantics.
pub fn client_builder(options: ClientOptions<'_>) -> reqwest::ClientBuilder {
    let redirect = match options.redirect_policy {
        RedirectPolicy::None => reqwest::redirect::Policy::none(),
        RedirectPolicy::Limited(limit) => reqwest::redirect::Policy::limited(limit),
    };
    let mut builder = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .pool_max_idle_per_host(MAX_IDLE_CONNECTIONS_PER_HOST)
        .tcp_keepalive(TCP_KEEPALIVE)
        .http2_adaptive_window(true)
        .redirect(redirect)
        .cookie_store(options.cookie_store);
    if let Some(timeout) = options.request_timeout {
        builder = builder.timeout(timeout);
    }
    if let Some(user_agent) = options.user_agent {
        builder = builder.user_agent(user_agent);
    }
    builder
}

pub fn build_client(options: ClientOptions<'_>) -> Result<reqwest::Client, reqwest::Error> {
    client_builder(options).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_default_is_redirect_free_and_has_no_total_timeout() {
        assert_eq!(
            ClientOptions::default(),
            ClientOptions {
                request_timeout: None,
                redirect_policy: RedirectPolicy::None,
                cookie_store: false,
                user_agent: None,
            }
        );
    }

    #[test]
    fn every_supported_profile_builds() {
        build_client(ClientOptions::default()).unwrap();
        build_client(ClientOptions {
            request_timeout: Some(Duration::from_secs(5)),
            redirect_policy: RedirectPolicy::Limited(5),
            cookie_store: true,
            user_agent: Some("desktop-http-test"),
        })
        .unwrap();
    }
}
