use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

use super::HostedIngestConfig;

pub(super) struct RequestHead {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) initial_body: Vec<u8>,
}

pub(super) fn read_request_head(
    stream: &mut TcpStream,
    config: &HostedIngestConfig,
    deadline: Instant,
) -> Result<RequestHead, HttpProblem> {
    let mut bytes = Vec::with_capacity(4096);
    let header_end = loop {
        if let Some(index) = find_header_end(&bytes) {
            if index + 4 > config.max_header_bytes {
                return Err(HttpProblem::HeadersTooLarge);
            }
            break index;
        }
        if bytes.len() >= config.max_header_bytes {
            return Err(HttpProblem::HeadersTooLarge);
        }
        let remaining = remaining(deadline)?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| HttpProblem::BadRequest)?;
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).map_err(map_read_error)?;
        if read == 0 {
            return Err(HttpProblem::BadRequest);
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    let header_bytes = &bytes[..header_end];
    let header = std::str::from_utf8(header_bytes).map_err(|_| HttpProblem::BadRequest)?;
    if !header.is_ascii() {
        return Err(HttpProblem::BadRequest);
    }
    let mut lines = header.split("\r\n");
    let mut request_line = lines
        .next()
        .ok_or(HttpProblem::BadRequest)?
        .split_ascii_whitespace();
    let method = request_line.next().ok_or(HttpProblem::BadRequest)?;
    let path = request_line.next().ok_or(HttpProblem::BadRequest)?;
    let version = request_line.next().ok_or(HttpProblem::BadRequest)?;
    if request_line.next().is_some()
        || version != "HTTP/1.1"
        || !path.starts_with('/')
        || path.contains(['?', '#'])
    {
        return Err(HttpProblem::BadRequest);
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        if line.starts_with([' ', '\t']) {
            return Err(HttpProblem::BadRequest);
        }
        let (name, value) = line.split_once(':').ok_or(HttpProblem::BadRequest)?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() && byte != b'\t')
        {
            return Err(HttpProblem::BadRequest);
        }
        let name = name.to_ascii_lowercase();
        if headers.insert(name, value.trim().to_owned()).is_some() {
            return Err(HttpProblem::BadRequest);
        }
    }
    if !headers
        .get("host")
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Err(HttpProblem::BadRequest);
    }
    Ok(RequestHead {
        method: method.to_owned(),
        path: path.to_owned(),
        headers,
        initial_body: bytes[header_end + 4..].to_vec(),
    })
}

pub(super) fn read_body(
    stream: &mut TcpStream,
    mut body: Vec<u8>,
    content_length: usize,
    deadline: Instant,
    max_body_bytes: usize,
) -> Result<Vec<u8>, HttpProblem> {
    if content_length > max_body_bytes {
        return Err(HttpProblem::PayloadTooLarge);
    }
    if body.len() > content_length {
        return Err(HttpProblem::BadRequest);
    }
    body.reserve(content_length.saturating_sub(body.len()));
    while body.len() < content_length {
        let remaining = remaining(deadline)?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| HttpProblem::BadRequest)?;
        let mut chunk = [0_u8; 8192];
        let wanted = chunk.len().min(content_length - body.len());
        let read = stream.read(&mut chunk[..wanted]).map_err(map_read_error)?;
        if read == 0 {
            return Err(HttpProblem::BadRequest);
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Ok(body)
}

fn remaining(deadline: Instant) -> Result<Duration, HttpProblem> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(HttpProblem::Timeout)
}

fn map_read_error(error: io::Error) -> HttpProblem {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => HttpProblem::Timeout,
        _ => HttpProblem::BadRequest,
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

pub(super) enum HttpProblem {
    BadRequest,
    HeadersTooLarge,
    PayloadTooLarge,
    Timeout,
}

pub(super) struct HttpResponse {
    status: u16,
    reason: &'static str,
    body: Vec<u8>,
    authenticate: bool,
}

impl HttpResponse {
    pub(super) fn json<T: serde::Serialize>(status: u16, reason: &'static str, value: &T) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => Self {
                status,
                reason,
                body,
                authenticate: false,
            },
            Err(_) => Self::internal_error(),
        }
    }

    pub(super) fn json_value(status: u16, reason: &'static str, value: serde_json::Value) -> Self {
        Self::json(status, reason, &value)
    }

    fn error(status: u16, reason: &'static str, code: &'static str) -> Self {
        Self::json_value(
            status,
            reason,
            serde_json::json!({ "error": { "code": code } }),
        )
    }

    pub(super) fn problem(problem: HttpProblem) -> Self {
        match problem {
            HttpProblem::BadRequest => Self::bad_request("malformed_http_request"),
            HttpProblem::HeadersTooLarge => {
                Self::error(431, "Request Header Fields Too Large", "headers_too_large")
            }
            HttpProblem::PayloadTooLarge => Self::payload_too_large(),
            HttpProblem::Timeout => Self::error(408, "Request Timeout", "request_timeout"),
        }
    }

    pub(super) fn bad_request(code: &'static str) -> Self {
        Self::error(400, "Bad Request", code)
    }

    pub(super) fn unauthorized() -> Self {
        let mut response = Self::error(401, "Unauthorized", "unauthorized");
        response.authenticate = true;
        response
    }

    pub(super) fn forbidden() -> Self {
        Self::error(403, "Forbidden", "tenant_mismatch")
    }

    pub(super) fn not_found() -> Self {
        Self::error(404, "Not Found", "not_found")
    }

    pub(super) fn method_not_allowed() -> Self {
        Self::error(405, "Method Not Allowed", "method_not_allowed")
    }

    pub(super) fn conflict(code: &'static str) -> Self {
        Self::error(409, "Conflict", code)
    }

    pub(super) fn length_required() -> Self {
        Self::error(411, "Length Required", "content_length_required")
    }

    pub(super) fn payload_too_large() -> Self {
        Self::error(413, "Payload Too Large", "payload_too_large")
    }

    pub(super) fn unsupported_media_type() -> Self {
        Self::error(415, "Unsupported Media Type", "application_json_required")
    }

    pub(super) fn internal_error() -> Self {
        Self::error(500, "Internal Server Error", "internal_error")
    }

    pub(super) fn service_unavailable() -> Self {
        Self::error(503, "Service Unavailable", "capacity_exhausted")
    }
}

pub(super) fn write_response(stream: &mut TcpStream, response: HttpResponse) -> io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n",
        response.status,
        response.reason,
        response.body.len()
    )?;
    if response.authenticate {
        write!(stream, "WWW-Authenticate: Bearer\r\n")?;
    }
    write!(stream, "\r\n")?;
    stream.write_all(&response.body)?;
    stream.flush()?;

    // A bounded, nonblocking drain prevents an unread request from turning the
    // response FIN into a reset. Never wait for a saturated or hostile peer.
    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_nonblocking(true);
    let mut drained = 0_usize;
    let mut buffer = [0_u8; 4096];
    while drained < 64 * 1024 {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => drained += read,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    Ok(())
}
