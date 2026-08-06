use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::model::{error, DynError};

pub async fn one_shot(
    response_text: &str,
) -> Result<(String, JoinHandle<Result<Value, DynError>>), DynError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let body = final_body(response_text);
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let request = read_request_json(&mut socket).await?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await?;
        socket.flush().await?;
        Ok(request)
    });
    Ok((format!("http://{address}/v1"), handle))
}

pub fn all_message_text(request: &Value) -> String {
    request
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|message| message.get("content"))
        .filter_map(message_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn message_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    Some(
        content
            .as_array()?
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
    )
}

fn final_body(text: &str) -> String {
    [
        format!(
            "data: {}\n\n",
            json!({"choices": [{"delta": {"content": text}}]})
        ),
        format!(
            "data: {}\n\n",
            json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})
        ),
        "data: [DONE]\n\n".to_string(),
    ]
    .concat()
}

async fn read_request_json(socket: &mut TcpStream) -> Result<Value, DynError> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut content_length = None;
    loop {
        let count = socket.read(&mut chunk).await?;
        if count == 0 {
            return Err(error("model request ended before its JSON body"));
        }
        bytes.extend_from_slice(&chunk[..count]);
        if content_length.is_none() {
            if let Some(headers_end) = headers_end(&bytes) {
                let headers = String::from_utf8_lossy(&bytes[..headers_end]);
                content_length = headers.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                });
            }
        }
        if let (Some(headers_end), Some(length)) = (headers_end(&bytes), content_length) {
            let start = headers_end + 4;
            if bytes.len() >= start + length {
                return Ok(serde_json::from_slice(&bytes[start..start + length])?);
            }
        }
    }
}

fn headers_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}
