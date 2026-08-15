use agent_core::domain::ToolKind;
use async_trait::async_trait;
use scout_adapter_protocol::{
    AdapterId, AdapterPageRequest, AuthContextHandle, AuthContextId, CoverageBinding, RequestId,
    TargetId, TargetIdentity,
};
use scout_adapter_runtime::{
    AuthCandidateHandle, CensusRequest, CensusResponse, FetchPageResponse, ScoutAdapterRequest,
    ScoutAdapterResponse, VerifyAuthRequest, VerifyAuthResponse, RUNTIME_PROTOCOL_VERSION,
    SERVICE_NAME as ADAPTER_SERVICE,
};
use scout_cartography_adapter::AdapterPageTaskScope;
use serde::Deserialize;
use serde_json::{json, Value};

use super::enterprise_backend::CartographyBackendState;
use super::ScoutToolState;
use crate::tools::{PermissionScope, ToolCtx, ToolExecutor, ToolOutcome, ToolPermissionClass};
use std::sync::Arc;

pub(super) struct ScoutAdapterTool {
    pub(super) state: Arc<ScoutToolState>,
    pub(super) cartography: Option<Arc<CartographyBackendState>>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AdapterAction {
    Census,
    VerifyAuth,
    FetchPage,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterArgs {
    action: AdapterAction,
    #[serde(default)]
    data: Option<Value>,
}

#[async_trait]
impl ToolExecutor for ScoutAdapterTool {
    fn name(&self) -> &str {
        "scout_adapter"
    }

    fn description(&self) -> &str {
        "Run a fixed, read-only Scout control-plane adapter on the current execution target. Census returns opaque credential candidates without values; verify_auth proves one candidate against an exact authority; fetch_page executes only registered GitHub/AWS/GCP list operations and returns normalized metadata plus a target-bound receipt. Provider tokens and raw pagination cursors remain in target-private storage. Local enterprise persistence is retired; backend-fenced tasks must be uploaded through Clark Code's authoritative cartography API."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["census", "verify_auth", "fetch_page"],
                    "description": "Choose the target-bound adapter operation first."
                },
                "data": {
                    "type": "object",
                    "description": "verify_auth: target_id, target_identity_sha256, candidate_handle, adapter_id, and optional requested_authority_scope. fetch_page: task_scope copied verbatim from claim_task plus target_id, target_identity_sha256, auth_context_handle, and auth_context_id copied from verify_auth. Rust constructs the coverage binding and supplies the host-owned protocol version, request id, and request time. Omit for census."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::External
    }

    fn mutating(&self) -> bool {
        true
    }

    fn permission_scope(&self, args: &Value) -> Option<PermissionScope> {
        let action = args.get("action")?.as_str()?;
        let adapter = args
            .pointer("/data/adapter_id")
            .or_else(|| args.pointer("/data/request/adapter_id"))
            .and_then(Value::as_str)
            .unwrap_or("census");
        let authority = args
            .pointer("/data/requested_authority_scope")
            .or_else(|| args.pointer("/data/request/query/authority_scope"))
            .and_then(Value::as_str)
            .unwrap_or("target");
        Some(PermissionScope {
            key: format!("scout-adapter:{action}:{adapter}:{authority}"),
            title: Some(format!("Allow read-only Scout access to {authority}?")),
            always_label: Some(format!("Always allow this {adapter} authority")),
            reason: Some(
                "uses the current target's credential context to read control-plane metadata"
                    .into(),
            ),
            risk: None,
            remember: true,
            preapproved: false,
        })
    }

    fn permission_preflight(&self, args: &Value) -> Result<(), String> {
        serde_json::from_value::<AdapterArgs>(args.clone())
            .map(|_| ())
            .map_err(|_| "invalid Scout adapter permission request".to_string())
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let _adapter_gate = self.state.adapter_gate.lock().await;
        let args: AdapterArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(_) => return ToolOutcome::error("invalid Scout adapter request"),
        };
        let built = match build_request(args) {
            Ok(request) => request,
            Err(error) => return ToolOutcome::error(error),
        };
        let root = match ctx
            .sandbox
            .resolve_host_managed(".agent/scout/adapters/private")
        {
            Ok(root) => root,
            Err(error) => return ToolOutcome::error(error),
        };
        let encoded = match serde_json::to_vec(&built.request) {
            Ok(encoded) => encoded,
            Err(_) => return ToolOutcome::error("Scout adapter request encoding failed"),
        };
        let response = match ctx
            .executor
            .target_service_call(ADAPTER_SERVICE, &root, &encoded)
            .await
        {
            Ok(response) => response,
            Err(error) => return ToolOutcome::error(error),
        };
        let response: ScoutAdapterResponse = match serde_json::from_slice(&response) {
            Ok(response) => response,
            Err(_) => return ToolOutcome::error("Scout adapter returned an invalid response"),
        };
        if let ScoutAdapterResponse::Census(CensusResponse::Succeeded { target, .. }) = &response {
            *self.state.target.lock().expect("Scout target lock") = Some((**target).clone());
        }
        if let ScoutAdapterResponse::FetchPage(FetchPageResponse::Succeeded { receipt }) = &response
        {
            if let Some(cartography) = &self.cartography {
                if let Err(error) = cartography.record_receipt((**receipt).clone()) {
                    return ToolOutcome::error(error);
                }
            }
        }
        outcome(response)
    }
}

struct BuiltRequest {
    request: ScoutAdapterRequest,
}

fn build_request(args: AdapterArgs) -> Result<BuiltRequest, String> {
    match args.action {
        AdapterAction::Census => {
            if args.data.is_some() {
                return Err("Scout adapter census does not accept data".into());
            }
            Ok(BuiltRequest {
                request: ScoutAdapterRequest::Census(CensusRequest::default()),
            })
        }
        AdapterAction::VerifyAuth => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct VerifyAuthArgs {
                target_id: TargetId,
                target_identity_sha256: String,
                candidate_handle: AuthCandidateHandle,
                adapter_id: AdapterId,
                requested_authority_scope: Option<String>,
            }
            let request: VerifyAuthArgs = decode(args.data, "verify_auth")?;
            Ok(BuiltRequest {
                request: ScoutAdapterRequest::VerifyAuth(VerifyAuthRequest {
                    runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
                    target_id: request.target_id,
                    target_identity_sha256: request.target_identity_sha256,
                    candidate_handle: request.candidate_handle,
                    adapter_id: request.adapter_id,
                    requested_authority_scope: request.requested_authority_scope,
                }),
            })
        }
        AdapterAction::FetchPage => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct FetchArgs {
                task_scope: AdapterPageTaskScope,
                target_id: TargetId,
                target_identity_sha256: String,
                auth_context_handle: AuthContextHandle,
                auth_context_id: AuthContextId,
            }
            let request = decode::<FetchArgs>(args.data, "fetch_page")?;
            let scope = request.task_scope;
            let coverage = CoverageBinding {
                enterprise_id: scope.enterprise_id,
                charter_id: scope.charter_id,
                discovery_epoch: scope.discovery_epoch,
                sequence: scope.coverage_sequence,
                adapter_id: scope.adapter_id.clone(),
                auth_context_id: request.auth_context_id.clone(),
                tenant: scope.query.authority_scope.clone(),
                region_or_project: scope.region_or_project,
                resource_kind: scope.resource_kind,
            };
            Ok(BuiltRequest {
                request: ScoutAdapterRequest::FetchPage(Box::new(AdapterPageRequest {
                    protocol_version: scout_adapter_protocol::ADAPTER_PROTOCOL_VERSION,
                    request_id: RequestId::random(),
                    target_id: request.target_id,
                    target_identity_sha256: request.target_identity_sha256,
                    adapter_id: scope.adapter_id,
                    auth_context_handle: request.auth_context_handle,
                    auth_context_id: request.auth_context_id,
                    coverage,
                    query: scope.query,
                    page_ordinal: scope.page_ordinal,
                    cursor_handle: scope.cursor_handle,
                    limits: scope.limits,
                    requested_at_ms: now_ms(),
                })),
            })
        }
    }
}

fn outcome(response: ScoutAdapterResponse) -> ToolOutcome {
    let details = match serde_json::to_value(&response) {
        Ok(details) => details,
        Err(_) => return ToolOutcome::error("Scout adapter response encoding failed"),
    };
    let content = match &response {
        ScoutAdapterResponse::Census(CensusResponse::Succeeded {
            target,
            candidates,
            tools,
            ..
        }) => {
            let target_identity_sha256 = match target_identity_sha256(target) {
                Ok(digest) => digest,
                Err(error) => return ToolOutcome::error(error),
            };
            format!(
                "Target adapter census found {} opaque authentication candidates and {} registered \
                 tool routes. The required canonical target_identity_sha256 is \
                 `{target_identity_sha256}`. Copy that exact digest, safe target, and opaque \
                 candidate handles from the typed result into verify_auth; never derive or invent them.",
                candidates.len(),
                tools.iter().filter(|tool| tool.available).count(),
            )
        }
        ScoutAdapterResponse::VerifyAuth(VerifyAuthResponse::Succeeded {
            target,
            auth_context,
        }) => {
            let target_identity_sha256 = match target_identity_sha256(target) {
                Ok(digest) => digest,
                Err(error) => return ToolOutcome::error(error),
            };
            format!(
                "Verified target-bound `{}` authorization for authority `{}`. The required \
                 canonical target_identity_sha256 is `{target_identity_sha256}`. Copy that exact \
                 digest, safe target, and opaque auth-context fields from the typed result into \
                 fetch_page; never derive or invent them.",
                auth_context.adapter_id, auth_context.authority_scope,
            )
        }
        ScoutAdapterResponse::FetchPage(FetchPageResponse::Succeeded { receipt }) => format!(
            "Scout adapter page `{}` recorded {} normalized records with outcome {:?}. Use this \
             exact receipt_id with the claimed task_id in submit_adapter_receipt.",
            receipt.receipt_id,
            receipt.records.len(),
            receipt.outcome
        ),
        ScoutAdapterResponse::Census(CensusResponse::Failed { failure })
        | ScoutAdapterResponse::VerifyAuth(VerifyAuthResponse::Failed { failure })
        | ScoutAdapterResponse::FetchPage(FetchPageResponse::Failed { failure }) => {
            return ToolOutcome::error(format!(
                "Scout adapter operation failed safely: {:?}",
                failure.code
            ))
            .with_model_visible_details(details)
        }
    };
    ToolOutcome::ok(content).with_model_visible_details(details)
}

fn target_identity_sha256(target: &TargetIdentity) -> Result<String, String> {
    target
        .fingerprint_sha256()
        .map_err(|_| "Scout adapter target identity fingerprint failed".to_string())
}

fn decode<T: serde::de::DeserializeOwned>(data: Option<Value>, action: &str) -> Result<T, String> {
    serde_json::from_value(data.ok_or_else(|| format!("Scout adapter {action} requires data"))?)
        .map_err(|error| format!("invalid Scout adapter {action} data: {error}"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn census_rejects_model_payload_and_unknown_actions() {
        assert!(build_request(AdapterArgs {
            action: AdapterAction::Census,
            data: Some(json!({"token": "must-not-enter"})),
        })
        .is_err());
        let tool = ScoutAdapterTool {
            state: Arc::new(ScoutToolState {
                target: Default::default(),
                repositories: Default::default(),
                adapter_gate: Default::default(),
            }),
            cartography: None,
        };
        assert!(tool
            .permission_preflight(&json!({"action": "shell", "data": {}}))
            .is_err());
    }

    #[test]
    fn local_enterprise_append_actions_are_retired() {
        let tool = ScoutAdapterTool {
            state: Arc::new(ScoutToolState {
                target: Default::default(),
                repositories: Default::default(),
                adapter_gate: Default::default(),
            }),
            cartography: None,
        };
        for action in ["fetch_and_append", "exhaust_and_append"] {
            assert!(tool
                .permission_preflight(&json!({"action": action, "data": {}}))
                .is_err());
            assert!(!serde_json::to_string(&tool.parameters())
                .unwrap()
                .contains(action));
        }
    }

    #[test]
    fn model_visible_adapter_results_keep_typed_handles_out_of_ui_only_details() {
        let target_id = "target:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let candidate =
            "candidate:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let details = json!({
            "status": "succeeded",
            "target": {"target_id": target_id},
            "candidates": [{"handle": candidate}],
        });
        let outcome = ToolOutcome::ok("Copy the exact safe target and opaque candidate handles.")
            .with_model_visible_details(details);

        assert!(outcome.content.contains(target_id));
        assert!(outcome.content.contains(candidate));
    }

    #[test]
    fn verify_auth_model_input_omits_the_host_owned_protocol_version() {
        let built = build_request(AdapterArgs {
            action: AdapterAction::VerifyAuth,
            data: Some(json!({
                "target_id": format!("target:{}", "a".repeat(64)),
                "target_identity_sha256": "b".repeat(64),
                "candidate_handle": format!("candidate:{}", "c".repeat(64)),
                "adapter_id": "host/github-organization@1",
                "requested_authority_scope": "Neon-Mobile",
            })),
        })
        .unwrap();
        let ScoutAdapterRequest::VerifyAuth(request) = built.request else {
            panic!("expected verify_auth request");
        };
        assert_eq!(request.runtime_protocol_version, RUNTIME_PROTOCOL_VERSION);
        assert_eq!(
            request.requested_authority_scope.as_deref(),
            Some("Neon-Mobile")
        );
    }

    #[test]
    fn fetch_page_model_input_omits_all_host_owned_request_fields() {
        let built = build_request(AdapterArgs {
            action: AdapterAction::FetchPage,
            data: Some(json!({
                "task_scope": {
                    "schema_version": 1,
                    "first_source_sequence": 50_001,
                    "adapter_id": "host/github-organization@1",
                    "enterprise_id": "neonmobile-system-map",
                    "charter_id": "charter-1",
                    "discovery_epoch": 1,
                    "coverage_sequence": 5,
                    "region_or_project": "global",
                    "resource_kind": "repository",
                    "query": {
                        "operation": "list_repositories",
                        "authority_scope": "Neon-Mobile",
                        "provider_resource_type": "github.repository",
                        "filters": {},
                        "projection": ["name"],
                        "page_size": 100
                    },
                    "page_ordinal": 0,
                    "cursor_handle": null,
                    "limits": {
                        "max_records": 100,
                        "max_response_bytes": 16777216,
                        "max_duration_ms": 60000
                    }
                },
                "target_id": format!("target:{}", "a".repeat(64)),
                "target_identity_sha256": "b".repeat(64),
                "auth_context_handle": format!("auth:{}", Uuid::nil()),
                "auth_context_id": format!("authctx:{}", "c".repeat(64))
            })),
        })
        .unwrap();
        let ScoutAdapterRequest::FetchPage(request) = built.request else {
            panic!("expected fetch_page request");
        };
        assert_eq!(
            request.protocol_version,
            scout_adapter_protocol::ADAPTER_PROTOCOL_VERSION
        );
        assert_eq!(request.page_ordinal, 0);
        assert!(request.cursor_handle.is_none());
        assert!(request.requested_at_ms > 0);
    }

    #[test]
    fn canonical_target_fingerprint_is_model_visible_instead_of_inferred() {
        let target: TargetIdentity = serde_json::from_value(json!({
            "protocol_version": RUNTIME_PROTOCOL_VERSION,
            "target_id": format!("target:{}", "a".repeat(64)),
            "identity_key_sha256": "a".repeat(64),
            "session_nonce_sha256": "b".repeat(64),
            "root_sha256": "c".repeat(64),
            "adapter_host_sha256": "d".repeat(64),
            "platform": "linux",
            "architecture": "x86_64",
        }))
        .unwrap();

        let digest = target_identity_sha256(&target).unwrap();
        assert_eq!(digest.len(), 64);
        assert_ne!(digest, target.identity_key_sha256);
    }
}
