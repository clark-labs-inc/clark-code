use std::collections::BTreeMap;

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

const MAX_REQUEST_HEADER_BYTES: usize = 2 * 1024 * 1024;
const FORWARDED_REQUEST_HEADERS: &[&str] = &[
    "idempotency-key",
    "x-session-id",
    "http-referer",
    "x-title",
    "user-agent",
    "x-openrouter-cache",
    "x-openrouter-cache-ttl",
    "x-openrouter-cache-clear",
];
const FORWARDED_RESPONSE_HEADERS: &[&str] = &[
    "x-request-id",
    "request-id",
    "cf-ray",
    "x-generation-id",
    "x-openrouter-cache-status",
    "x-openrouter-cache-age",
    "x-openrouter-cache-ttl",
];

pub(super) struct Request {
    pub(super) method: String,
    pub(super) target: String,
    pub(super) body: Vec<u8>,
    headers: BTreeMap<String, String>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

pub(super) struct ForwardedResponse {
    pub(super) status: u16,
    pub(super) content_type: &'static str,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

pub(super) async fn forward(
    request: &Request,
    client: &reqwest::Client,
    upstream_base_url: &str,
    api_key: &str,
) -> Result<ForwardedResponse, String> {
    let mut upstream = reqwest::Url::parse(upstream_base_url)
        .map_err(|error| format!("invalid upstream URL: {error}"))?;
    let target = reqwest::Url::parse(&format!("http://benchmark{}", request.target))
        .map_err(|error| error.to_string())?;
    upstream.set_path(target.path());
    upstream.set_query(target.query());
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|error| error.to_string())?;
    let mut builder = client.request(method, upstream).bearer_auth(api_key);
    for name in FORWARDED_REQUEST_HEADERS {
        if let Some(value) = request.header(name) {
            builder = builder.header(*name, value);
        }
    }
    if !request.body.is_empty() {
        builder = builder
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(request.body.clone());
    }
    let response = builder
        .send()
        .await
        .map_err(|error| format!("upstream request failed: {error}"))?;
    let status = response.status().as_u16();
    let content_type = if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"))
    {
        "text/event-stream"
    } else {
        "application/json"
    };
    let headers = FORWARDED_RESPONSE_HEADERS
        .iter()
        .filter_map(|name| {
            response
                .headers()
                .get(*name)
                .and_then(|value| value.to_str().ok())
                .map(|value| ((*name).to_string(), value.to_string()))
        })
        .collect();
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("upstream body failed: {error}"))?
        .to_vec();
    Ok(ForwardedResponse {
        status,
        content_type,
        headers,
        body,
    })
}

pub(super) async fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    let header_end = loop {
        let count = stream
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("request ended before headers".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() > MAX_REQUEST_HEADER_BYTES {
            return Err("request headers exceed benchmark limit".into());
        }
    };
    let raw_headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let headers = raw_headers
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect::<BTreeMap<_, _>>();
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let count = stream
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("request ended before its declared body length".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let first = raw_headers.lines().next().ok_or("request line missing")?;
    let mut parts = first.split_whitespace();
    Ok(Request {
        method: parts.next().ok_or("request method missing")?.to_string(),
        target: parts.next().ok_or("request target missing")?.to_string(),
        body: bytes[header_end..header_end + content_length].to_vec(),
        headers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn forward_preserves_turn_identity_and_response_receipts() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await.unwrap();
            let body = b"done";
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nX-Request-ID: req-1\r\nX-OpenRouter-Cache-Status: HIT\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len(),
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            stream.write_all(body).await.unwrap();
            request
        });
        let request = Request {
            method: "POST".into(),
            target: "/v1/chat/completions".into(),
            body: b"{}".to_vec(),
            headers: BTreeMap::from([
                ("idempotency-key".into(), "logical-turn-1".into()),
                ("x-session-id".into(), "conversation-1".into()),
            ]),
        };
        let client = clark_http::build_client(clark_http::ClientOptions::default()).unwrap();
        let response = forward(
            &request,
            &client,
            &format!("http://{address}/v1"),
            "benchmark-key",
        )
        .await
        .unwrap();

        let captured = upstream.await.unwrap();
        assert_eq!(captured.header("idempotency-key"), Some("logical-turn-1"));
        assert_eq!(captured.header("x-session-id"), Some("conversation-1"));
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/event-stream");
        assert_eq!(
            response.headers,
            [
                ("x-request-id".into(), "req-1".into()),
                ("x-openrouter-cache-status".into(), "HIT".into()),
            ]
        );
        assert_eq!(response.body, b"done");
    }
}
