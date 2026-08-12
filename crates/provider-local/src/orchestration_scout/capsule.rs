use std::sync::Arc;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use scout_capsule_host::{
    CapsulePolicyBinding, CapsuleServiceRequest, CapsuleServiceResponse, CensusCapsuleRequest,
    InvokeCapsuleRequest, CAPSULE_SERVICE_PROTOCOL_VERSION, SERVICE_NAME,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::ScoutToolState;
use crate::orchestration::ScoutCapsulePolicyConfig;
use crate::tools::{ToolCtx, ToolExecutor, ToolOutcome};

const INPUT_SCHEMA: &str = "scout-capsule-request-v1";
const OUTPUT_SCHEMA: &str = "scout-capsule-normalized-page-v1";
pub(super) struct ScoutCapsuleTool {
    pub(super) state: Arc<ScoutToolState>,
    pub(super) policy: ScoutCapsulePolicyConfig,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Args {
    Census {
        enterprise_id: String,
    },
    Invoke {
        capsule_id: String,
        enterprise_id: String,
        input: Value,
    },
}

#[async_trait]
impl ToolExecutor for ScoutCapsuleTool {
    fn name(&self) -> &str {
        "scout_capsule"
    }

    fn description(&self) -> &str {
        "List or invoke administrator-approved, target-local Scout WASM transforms. The model may select only a registered logical capsule and bounded input; module approval, bytes, digest, path, signer, limits, tenant, and execution-target identity are host-owned."
    }

    fn parameters(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "action":{"type":"string","enum":["census","invoke"]},
                "capsule_id":{"type":"string","description":"Logical capsule id returned by census."},
                "enterprise_id":{"type":"string","description":"Enterprise graph boundary authorized by the signed target registry."},
                "input":{"type":"object","description":"Bounded typed capsule input. Approval and executable fields are not accepted."}
            },
            "required":["action", "enterprise_id"],
            "additionalProperties":false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: Args = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(_) => return ToolOutcome::error("invalid Scout capsule request"),
        };
        let target = match self.state.target.lock().expect("Scout target lock").clone() {
            Some(target) => target,
            None => {
                return ToolOutcome::error(
                    "run scout_adapter census first; capsule target identity is host-cached only from a successful census",
                )
            }
        };
        let target_identity_sha256 = match target.fingerprint_sha256() {
            Ok(value) => value,
            Err(_) => return ToolOutcome::error("cached Scout target identity is invalid"),
        };
        let binding = CapsulePolicyBinding {
            protocol_version: CAPSULE_SERVICE_PROTOCOL_VERSION,
            authorized_tenant_id: self.policy.authorized_tenant_id.clone(),
            trusted_admin_key_sha256: self.policy.trusted_admin_key_sha256.clone(),
            minimum_registry_generation: self.policy.minimum_registry_generation,
            target_id: target.target_id.to_string(),
            target_identity_sha256,
        };
        let request = match args {
            Args::Census { enterprise_id } => CapsuleServiceRequest::Census(CensusCapsuleRequest {
                policy: binding,
                enterprise_id,
            }),
            Args::Invoke {
                capsule_id,
                enterprise_id,
                input,
            } => {
                let input = match serde_json::to_vec(&json!({
                    "schema": INPUT_SCHEMA,
                    "payload": input
                })) {
                    Ok(input) => input,
                    Err(_) => return ToolOutcome::error("Scout capsule input encoding failed"),
                };
                CapsuleServiceRequest::Invoke(InvokeCapsuleRequest {
                    policy: binding,
                    capsule_id,
                    enterprise_id,
                    input_schema: INPUT_SCHEMA.into(),
                    output_schema: OUTPUT_SCHEMA.into(),
                    input_base64: STANDARD.encode(input),
                })
            }
        };
        let root = match ctx
            .sandbox
            .resolve_host_managed(".agent/scout/capsules/private")
        {
            Ok(root) => root,
            Err(error) => return ToolOutcome::error(error),
        };
        let encoded = match serde_json::to_vec(&request) {
            Ok(encoded) => encoded,
            Err(_) => return ToolOutcome::error("Scout capsule request encoding failed"),
        };
        let response = match ctx
            .executor
            .target_service_call(SERVICE_NAME, &root, &encoded)
            .await
        {
            Ok(response) => response,
            Err(error) => return ToolOutcome::error(error),
        };
        let response: CapsuleServiceResponse = match serde_json::from_slice(&response) {
            Ok(response) => response,
            Err(_) => return ToolOutcome::error("Scout capsule returned an invalid response"),
        };
        outcome(response)
    }
}

fn outcome(response: CapsuleServiceResponse) -> ToolOutcome {
    match response {
        CapsuleServiceResponse::Census {
            registry_sha256,
            generation,
            capsules,
        } => ToolOutcome::ok(format!(
            "Target registry generation {generation} exposes {} administrator-approved capsules.",
            capsules.len()
        ))
        .with_details(json!({
            "registry_sha256": registry_sha256,
            "generation": generation,
            "capsules": capsules
        })),
        CapsuleServiceResponse::Invoked {
            registry_sha256,
            generation,
            capsule_id,
            enterprise_id,
            target_id,
            output_base64,
            isolation,
            deadline_is_hard_interrupt,
        } => {
            let output = match STANDARD.decode(output_base64) {
                Ok(output) => output,
                Err(_) => return ToolOutcome::error("Scout capsule output encoding is invalid"),
            };
            let output: Value = match serde_json::from_slice(&output) {
                Ok(output) => output,
                Err(_) => return ToolOutcome::error("Scout capsule output is invalid"),
            };
            ToolOutcome::ok(format!(
                "Administrator-approved capsule `{capsule_id}` completed for `{enterprise_id}`."
            ))
            .with_details(json!({
                "registry_sha256": registry_sha256,
                "generation": generation,
                "capsule_id": capsule_id,
                "enterprise_id": enterprise_id,
                "target_id": target_id,
                "output": output,
                "isolation": isolation,
                "deadline_is_hard_interrupt": deadline_is_hard_interrupt
            }))
        }
        CapsuleServiceResponse::Failed { code, message } => {
            ToolOutcome::error(format!("Scout capsule rejected safely ({code}): {message}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_schema_has_no_approval_or_authority_fields() {
        let schema = ScoutCapsuleTool {
            state: Arc::new(ScoutToolState {
                target: Default::default(),
                repositories: Default::default(),
                adapter_gate: Default::default(),
            }),
            policy: ScoutCapsulePolicyConfig {
                authorized_tenant_id: "tenant-a".into(),
                trusted_admin_key_sha256: "a".repeat(64),
                minimum_registry_generation: 1,
            },
        }
        .parameters();
        let properties = schema["properties"].as_object().unwrap();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "enterprise_id"));
        for forbidden in [
            "module",
            "module_bytes",
            "module_sha256",
            "path",
            "admin_key",
            "limits",
            "tenant_id",
            "target_id",
        ] {
            assert!(!properties.contains_key(forbidden));
        }
    }

    #[test]
    fn oversized_output_is_preserved_in_the_typed_result() {
        let payload = "x".repeat(256 * 1024 + 1);
        let output = serde_json::to_vec(&json!({"payload": payload})).unwrap();
        let receipt = scout_capsule_host::CapsuleIsolationReceipt {
            schema: "scout-capsule-isolation-receipt-v1".into(),
            abi_version: 1,
            runtime: "wasmi-test".into(),
            module_sha256: "a".repeat(64),
            import_set: Vec::new(),
            fresh_instance: true,
            wasi_enabled: false,
            limits: scout_capsule_host::CapsuleHostLimits::default(),
            input_sha256: "b".repeat(64),
            output_sha256: "c".repeat(64),
            fuel_consumed: 7,
            elapsed_micros: 11,
        };
        let result = outcome(CapsuleServiceResponse::Invoked {
            registry_sha256: "d".repeat(64),
            generation: 4,
            capsule_id: "normalize-page".into(),
            enterprise_id: "enterprise-a".into(),
            target_id: "target-a".into(),
            output_base64: STANDARD.encode(&output),
            isolation: Box::new(receipt),
            deadline_is_hard_interrupt: false,
        });

        assert!(!result.is_error);
        assert_eq!(result.details["output"]["payload"], payload);
        assert!(!result
            .details
            .as_object()
            .unwrap()
            .contains_key("output_omitted"));
        assert_eq!(result.details["isolation"]["output_sha256"], "c".repeat(64));
    }
}
