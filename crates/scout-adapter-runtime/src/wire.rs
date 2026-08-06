use std::path::Path;

use scout_adapter_protocol::AdapterPageRequest;
use serde::{Deserialize, Serialize};

use crate::{
    CensusRequest, CensusResponse, FetchPageResponse, RuntimeConfig, SafeFailure,
    ScoutAdapterService, VerifyAuthRequest, VerifyAuthResponse,
};

pub const SERVICE_NAME: &str = "scout-adapter-v1";
pub const MAX_ADAPTER_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ScoutAdapterRequest {
    Census(CensusRequest),
    VerifyAuth(VerifyAuthRequest),
    FetchPage(Box<AdapterPageRequest>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "result",
    content = "response",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ScoutAdapterResponse {
    Census(CensusResponse),
    VerifyAuth(VerifyAuthResponse),
    FetchPage(FetchPageResponse),
}

/// Dispatch one bounded adapter operation on the execution target.
///
/// `root` is target-private state. Credentials and provider cursors are
/// resolved and retained by the runtime and never become individual RPC file
/// operations.
pub async fn dispatch(service: &str, root: &Path, request: &[u8]) -> Result<Vec<u8>, String> {
    if service != SERVICE_NAME {
        return Err(format!("unsupported Scout adapter service: {service}"));
    }
    if request.len() > MAX_ADAPTER_REQUEST_BYTES {
        return Err(format!(
            "Scout adapter request exceeds the {MAX_ADAPTER_REQUEST_BYTES}-byte limit"
        ));
    }
    let request: ScoutAdapterRequest =
        serde_json::from_slice(request).map_err(|_| "invalid Scout adapter request".to_string())?;
    let service = match ScoutAdapterService::open(RuntimeConfig::new(root)) {
        Ok(service) => service,
        Err(failure) => return encode(open_failure(request, failure)),
    };
    let response = match request {
        ScoutAdapterRequest::Census(request) => {
            ScoutAdapterResponse::Census(service.census(request).await)
        }
        ScoutAdapterRequest::VerifyAuth(request) => {
            ScoutAdapterResponse::VerifyAuth(service.verify_auth(request).await)
        }
        ScoutAdapterRequest::FetchPage(request) => {
            ScoutAdapterResponse::FetchPage(service.fetch_page(*request).await)
        }
    };
    encode(response)
}

fn open_failure(request: ScoutAdapterRequest, failure: SafeFailure) -> ScoutAdapterResponse {
    match request {
        ScoutAdapterRequest::Census(_) => {
            ScoutAdapterResponse::Census(CensusResponse::Failed { failure })
        }
        ScoutAdapterRequest::VerifyAuth(_) => {
            ScoutAdapterResponse::VerifyAuth(VerifyAuthResponse::Failed { failure })
        }
        ScoutAdapterRequest::FetchPage(_) => {
            ScoutAdapterResponse::FetchPage(FetchPageResponse::Failed { failure })
        }
    }
}

fn encode(response: ScoutAdapterResponse) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&response).map_err(|_| "Scout adapter response encoding failed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatch_rejects_unknown_services_and_fields_without_echoing_input() {
        let temp = tempfile::tempdir().unwrap();
        let error = dispatch("other", temp.path(), b"{}").await.unwrap_err();
        assert!(error.contains("unsupported"));

        let error = dispatch(
            SERVICE_NAME,
            temp.path(),
            br#"{"action":"census","request":{"runtime_protocol_version":1,"token":"secret"}}"#,
        )
        .await
        .unwrap_err();
        assert_eq!(error, "invalid Scout adapter request");
        assert!(!error.contains("secret"));
    }

    #[tokio::test]
    async fn dispatch_census_round_trip_keeps_target_private_state_opaque() {
        let temp = tempfile::tempdir().unwrap();
        let request =
            serde_json::to_vec(&ScoutAdapterRequest::Census(CensusRequest::default())).unwrap();
        let response_bytes = dispatch(SERVICE_NAME, temp.path(), &request).await.unwrap();
        let response: ScoutAdapterResponse = serde_json::from_slice(&response_bytes).unwrap();
        let ScoutAdapterResponse::Census(CensusResponse::Succeeded { target, .. }) = response
        else {
            panic!("wrong adapter census response");
        };
        assert!(target.target_id.as_str().starts_with("target:"));
        let response_text = String::from_utf8(response_bytes).unwrap();
        assert!(!response_text.contains("vault.key"));
        assert!(!response_text.contains(&temp.path().display().to_string()));
    }
}
