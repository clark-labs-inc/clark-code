use serde_json::Value;
use tauri::{AppHandle, State};

use crate::product::ProductRequestContext;
use crate::state::AppState;

fn valid_operation(operation: &str) -> bool {
    !operation.is_empty()
        && operation.len() <= 128
        && operation.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

#[tauri::command]
pub async fn product_request(
    operation: String,
    payload: Value,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    dispatch_product_request(&operation, payload, &app, state.inner()).await
}

pub(crate) async fn dispatch_product_request(
    operation: &str,
    payload: Value,
    app: &AppHandle,
    state: &AppState,
) -> Result<Value, String> {
    if !valid_operation(operation) {
        return Err("product operation is invalid".to_string());
    }
    state
        .product
        .request(operation, payload, ProductRequestContext { app, state })
        .await
}

#[cfg(test)]
mod tests {
    use super::valid_operation;

    #[test]
    fn product_operation_is_an_opaque_bounded_identifier() {
        assert!(valid_operation("access.snapshot"));
        assert!(valid_operation("specialist_query-v1"));
        assert!(!valid_operation(""));
        assert!(!valid_operation("../private-operation"));
        assert!(!valid_operation("Uppercase"));
        assert!(!valid_operation(&"a".repeat(129)));
    }
}
