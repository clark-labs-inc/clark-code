use serde_json::Value;

pub(super) fn terminal_response(response: &Value) -> Result<Option<String>, String> {
    match response.get("status").and_then(Value::as_str) {
        Some("completed") => {
            let answer = response
                .get("output")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|item| {
                    item.get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .find_map(|content| {
                    matches!(
                        content.get("type").and_then(Value::as_str),
                        Some("output_text") | Some("text")
                    )
                    .then(|| content.get("text").and_then(Value::as_str))
                    .flatten()
                })
                .unwrap_or_default()
                .trim()
                .to_string();
            if answer.is_empty() {
                Err("Clark research returned no findings".to_string())
            } else {
                Ok(Some(answer))
            }
        }
        Some("failed") | Some("cancelled") => {
            let message = response
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Clark research failed");
            Err(message.to_string())
        }
        _ => Ok(None),
    }
}
