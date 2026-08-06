//! Private Kimi K3 strategy guidance for first-party specialist sessions.
//!
//! The desktop supplies the organization, specialist, workflow, execution
//! residency, and training-consent binding. The model supplies only the
//! current decision packet. Clark performs model routing, billing, and durable
//! telemetry collection server-side.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{arg_str, arg_str_opt, ToolCtx, ToolExecutor, ToolOutcome, ToolPermissionClass};

const ADVISOR_MODEL: &str = "moonshotai/kimi-k3";
const ADVISOR_VERSION: &str = "cloud-advisor.v1";
const MAX_GOAL_BYTES: usize = 64 * 1024;
const MAX_EVIDENCE_ITEMS: usize = 128;
const MAX_ACTIONS: usize = 32;

#[derive(Clone, Debug)]
pub struct CloudAdvisorConfig {
    base_url: String,
    api_key: String,
    organization_id: String,
    specialist: String,
    workflow: String,
    execution_residency: &'static str,
    training_consent: String,
    session_id: String,
}

impl CloudAdvisorConfig {
    pub(crate) fn from_extra(
        extra: &Value,
        base_url: &str,
        api_key: Option<&str>,
        cwd: Option<&str>,
        remote_worker: bool,
    ) -> Option<Self> {
        let binding = extra.get("cloud_advisor")?.as_object()?;
        let field = |name: &str| {
            binding
                .get(name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        };
        let organization_id = field("organization_id")?;
        Uuid::parse_str(organization_id).ok()?;
        let specialist = field("specialist")?;
        if !matches!(specialist, "scout" | "security") {
            return None;
        }
        let workflow = field("workflow")?;
        if workflow.len() > 128 {
            return None;
        }
        let execution_residency = match field("execution_residency")? {
            "local_only" if !remote_worker => "local_only",
            "remote_worker" if remote_worker => "remote_worker",
            _ => return None,
        };
        let training_consent = match field("training_consent").unwrap_or("none") {
            "explicit_user" => "explicit_user",
            "organization_contract" => "organization_contract",
            _ => "none",
        };
        let api_key = api_key?.trim();
        if api_key.is_empty() {
            return None;
        }
        let target = cwd.unwrap_or("");
        let digest = Sha256::digest(
            format!("{organization_id}\0{specialist}\0{workflow}\0{execution_residency}\0{target}")
                .as_bytes(),
        );
        Some(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            organization_id: organization_id.to_string(),
            specialist: specialist.to_string(),
            workflow: workflow.to_string(),
            execution_residency,
            training_consent: training_consent.to_string(),
            session_id: format!("specialist-{:x}", digest)[..75].to_string(),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdvisorResponse {
    schema_version: u32,
    request_id: String,
    advisor_version: String,
    advisor_model: String,
    advice: Value,
    usage: Value,
    receipt: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdvisorFeedbackResponse {
    schema_version: u32,
    feedback_id: String,
    feedback_sha256: String,
    telemetry_sha256: String,
    telemetry_version_id: String,
    training_eligible: bool,
}

pub struct CloudAdvisorTool {
    config: CloudAdvisorConfig,
    client: reqwest::Client,
}

impl CloudAdvisorTool {
    pub fn new(config: CloudAdvisorConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(4 * 60))
            .build()
            .unwrap_or_default();
        Self { config, client }
    }
}

#[async_trait]
impl ToolExecutor for CloudAdvisorTool {
    fn name(&self) -> &str {
        "cloud_advisor"
    }

    fn description(&self) -> &str {
        "Ask Clark's private Kimi K3 advisor for one bounded specialist strategy decision. Supply current evidence and typed candidate capabilities, never secrets or executable shell text. Advice is guidance, not repository evidence, and execution remains local or on the selected SSH host."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "phase": {
                    "type": "string",
                    "maxLength": 64,
                    "description": "Current specialist phase, such as discovery_strategy or threat_model."
                },
                "goal": {
                    "type": "string",
                    "maxLength": MAX_GOAL_BYTES,
                    "description": "The bounded decision to advise on. Do not include credentials or raw secret values."
                },
                "evidence": {
                    "type": "array",
                    "maxItems": MAX_EVIDENCE_ITEMS,
                    "items": {},
                    "description": "Current observations and receipts. Treat advice returned from prior calls separately from evidence."
                },
                "candidate_actions": {
                    "type": "array",
                    "maxItems": MAX_ACTIONS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "capability": {"type": "string", "maxLength": 128},
                            "description": {"type": "string", "maxLength": 4096},
                            "constraints": {}
                        },
                        "required": ["capability", "description", "constraints"],
                        "additionalProperties": false
                    },
                    "description": "Typed capabilities the specialist may execute after independently validating the advice."
                },
                "previous_advice_receipt": {
                    "type": "string",
                    "pattern": "^[0-9a-fA-F]{64}$",
                    "description": "Optional SHA-256 advice receipt from the previous advisor decision."
                }
            },
            "required": ["phase", "goal", "evidence", "candidate_actions"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::BrokeredClarkCloud
    }

    async fn invoke(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        match self.invoke_inner(args).await {
            Ok(value) => ToolOutcome::ok(format!(
                "[Clark Cloud Advisor guidance; advisory strategy, not repository evidence]\n{}",
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
            )),
            Err(error) => ToolOutcome::error(format!(
                "Clark Cloud Advisor is unavailable; continue with the baseline specialist workflow: {}",
                bounded(error)
            )),
        }
    }
}

impl CloudAdvisorTool {
    async fn invoke_inner(&self, args: Value) -> Result<Value, String> {
        let phase = arg_str(&args, "phase")?;
        let goal = arg_str(&args, "goal")?;
        if phase.is_empty() || phase.len() > 64 {
            return Err("phase must contain at most 64 bytes".into());
        }
        if goal.is_empty() || goal.len() > MAX_GOAL_BYTES {
            return Err("goal must contain between 1 and 65536 bytes".into());
        }
        let evidence = args
            .get("evidence")
            .and_then(Value::as_array)
            .ok_or_else(|| "evidence must be an array".to_string())?;
        let candidate_actions = args
            .get("candidate_actions")
            .and_then(Value::as_array)
            .ok_or_else(|| "candidate_actions must be an array".to_string())?;
        if evidence.len() > MAX_EVIDENCE_ITEMS || candidate_actions.len() > MAX_ACTIONS {
            return Err("advisor evidence or candidate action count exceeds its bound".into());
        }
        let previous = arg_str_opt(&args, "previous_advice_receipt");
        let request_id = format!("advisor-{}", Uuid::new_v4());
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let eligible = self.config.training_consent != "none";
        let body = json!({
            "schemaVersion": 1,
            "requestId": request_id,
            "organizationId": self.config.organization_id,
            "sessionId": self.config.session_id,
            "specialist": self.config.specialist,
            "workflow": self.config.workflow,
            "executionResidency": self.config.execution_residency,
            "phase": phase,
            "goal": goal,
            "evidence": evidence,
            "candidateActions": candidate_actions,
            "budgets": {"advisorCalls": 1},
            "previousAdviceReceipt": previous,
            "trainingConsent": {
                "eligible": eligible,
                "basis": self.config.training_consent,
                "policyVersion": "advisor-training.v1",
                "recordedAtMs": now_ms,
            },
            "dataClasses": ["customer_source_summary", "specialist_trajectory"],
        });
        let response = self
            .client
            .post(format!("{}/specialists/advisor", self.config.base_url))
            .bearer_auth(&self.config.api_key)
            .header("x-clark-client", "clark-code-desktop")
            .header("idempotency-key", &request_id)
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("could not read response: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "HTTP {status}: {}",
                bounded(String::from_utf8_lossy(&bytes).into_owned())
            ));
        }
        let response: AdvisorResponse = serde_json::from_slice(&bytes)
            .map_err(|error| format!("response is invalid: {error}"))?;
        if response.schema_version != 1
            || response.request_id != request_id
            || response.advisor_version != ADVISOR_VERSION
            || response.advisor_model != ADVISOR_MODEL
            || !response.advice.is_object()
            || !response.receipt.is_object()
        {
            return Err("response does not match the v1 Kimi K3 contract".into());
        }
        Ok(json!({
            "requestId": request_id,
            "advisorModel": response.advisor_model,
            "advisorVersion": response.advisor_version,
            "advice": response.advice,
            "usage": response.usage,
            "receipt": response.receipt,
        }))
    }
}

pub struct CloudAdvisorFeedbackTool {
    config: CloudAdvisorConfig,
    client: reqwest::Client,
}

impl CloudAdvisorFeedbackTool {
    pub fn new(config: CloudAdvisorConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self { config, client }
    }
}

#[async_trait]
impl ToolExecutor for CloudAdvisorFeedbackTool {
    fn name(&self) -> &str {
        "cloud_advisor_feedback"
    }

    fn description(&self) -> &str {
        "Record what actually happened after Clark Cloud Advisor guidance so authorized future advisor training can distinguish useful advice from failed or ignored advice. This does not call a model or incur model-token charges."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "request_id": {"type":"string","maxLength":128},
                "advice_sha256": {"type":"string","pattern":"^[0-9a-fA-F]{64}$"},
                "telemetry_version_id": {"type":"string","maxLength":1024},
                "receipt_signature": {"type":"string","pattern":"^[0-9a-fA-F]{64}$"},
                "status": {"type":"string","enum":["completed","failed","cancelled","partial"]},
                "actual_actions": {"type":"array","maxItems":128,"items":{}},
                "outcome": {},
                "evidence_refs": {"type":"array","maxItems":256,"items":{}}
            },
            "required": ["request_id", "advice_sha256", "telemetry_version_id", "receipt_signature", "status", "actual_actions", "outcome", "evidence_refs"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Think
    }

    fn permission_class(&self) -> ToolPermissionClass {
        ToolPermissionClass::BrokeredClarkCloud
    }

    async fn invoke(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        match self.invoke_inner(args).await {
            Ok(receipt) => ToolOutcome::ok(format!(
                "Clark Cloud Advisor outcome feedback was durably recorded.\n{}",
                serde_json::to_string_pretty(&receipt).unwrap_or_else(|_| receipt.to_string())
            )),
            Err(error) => ToolOutcome::error(format!(
                "Clark Cloud Advisor outcome feedback could not be recorded: {}",
                bounded(error)
            )),
        }
    }
}

impl CloudAdvisorFeedbackTool {
    async fn invoke_inner(&self, args: Value) -> Result<Value, String> {
        let request_id = arg_str(&args, "request_id")?;
        let advice_sha256 = arg_str(&args, "advice_sha256")?;
        let telemetry_version_id = arg_str(&args, "telemetry_version_id")?;
        let receipt_signature = arg_str(&args, "receipt_signature")?;
        let status = arg_str(&args, "status")?;
        let actual_actions = args
            .get("actual_actions")
            .and_then(Value::as_array)
            .ok_or_else(|| "actual_actions must be an array".to_string())?;
        let evidence_refs = args
            .get("evidence_refs")
            .and_then(Value::as_array)
            .ok_or_else(|| "evidence_refs must be an array".to_string())?;
        let outcome = args.get("outcome").cloned().unwrap_or(Value::Null);
        let feedback_id = format!("feedback-{}", Uuid::new_v4());
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let eligible = self.config.training_consent != "none";
        let body = json!({
            "schemaVersion": 1,
            "feedbackId": feedback_id,
            "requestId": request_id,
            "organizationId": self.config.organization_id,
            "sessionId": self.config.session_id,
            "specialist": self.config.specialist,
            "workflow": self.config.workflow,
            "executionResidency": self.config.execution_residency,
            "adviceSha256": advice_sha256,
            "telemetryVersionId": telemetry_version_id,
            "receiptSignature": receipt_signature,
            "status": status,
            "actualActions": actual_actions,
            "outcome": outcome,
            "evidenceRefs": evidence_refs,
            "trainingConsent": {
                "eligible": eligible,
                "basis": self.config.training_consent,
                "policyVersion": "advisor-training.v1",
                "recordedAtMs": now_ms,
            },
            "dataClasses": ["specialist_trajectory", "advisor_outcome"],
        });
        let response = self
            .client
            .post(format!(
                "{}/specialists/advisor/feedback",
                self.config.base_url
            ))
            .bearer_auth(&self.config.api_key)
            .header("x-clark-client", "clark-code-desktop")
            .header("idempotency-key", &feedback_id)
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|error| error.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "HTTP {status}: {}",
                bounded(String::from_utf8_lossy(&bytes).into_owned())
            ));
        }
        let response: AdvisorFeedbackResponse = serde_json::from_slice(&bytes)
            .map_err(|error| format!("response is invalid: {error}"))?;
        if response.schema_version != 1
            || response.feedback_id != feedback_id
            || response.feedback_sha256.len() != 64
            || response.telemetry_sha256.len() != 64
            || response.telemetry_version_id.is_empty()
        {
            return Err("response does not match the v1 feedback contract".into());
        }
        Ok(json!({
            "feedbackId": response.feedback_id,
            "feedbackSha256": response.feedback_sha256,
            "telemetrySha256": response.telemetry_sha256,
            "telemetryVersionId": response.telemetry_version_id,
            "trainingEligible": response.training_eligible,
        }))
    }
}

fn bounded(mut value: String) -> String {
    if value.len() > 2048 {
        let mut boundary = 2048;
        while !value.is_char_boundary(boundary) {
            boundary -= 1;
        }
        value.truncate(boundary);
        value.push('…');
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn config(extra: Value, remote_worker: bool) -> Option<CloudAdvisorConfig> {
        CloudAdvisorConfig::from_extra(
            &extra,
            "https://api.clarkslabs.com/v1",
            Some("ck_live_test"),
            Some("/local/project"),
            remote_worker,
        )
    }

    #[test]
    fn host_binding_requires_matching_residency_and_registered_specialist() {
        let extra = json!({"cloud_advisor": {
            "organization_id": "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
            "specialist": "scout",
            "workflow": "scout:map",
            "execution_residency": "local_only",
            "training_consent": "none"
        }});
        assert!(config(extra.clone(), false).is_some());
        assert!(config(extra, true).is_none());
    }

    #[test]
    fn schema_orders_decision_before_payload_and_is_bounded() {
        let config = config(
            json!({"cloud_advisor": {
                "organization_id": "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
                "specialist": "security",
                "workflow": "security:scan",
                "execution_residency": "local_only"
            }}),
            false,
        )
        .unwrap();
        let schema = CloudAdvisorTool::new(config).parameters();
        let names = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "phase",
                "goal",
                "evidence",
                "candidate_actions",
                "previous_advice_receipt"
            ]
        );
        assert_eq!(schema["properties"]["evidence"]["maxItems"], 128);
    }

    #[tokio::test]
    async fn paid_advisor_contract_round_trips_through_the_platform_boundary() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            let header_end = loop {
                let read = stream.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
                if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_string)
                })
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            while request.len() < header_end + content_length {
                let read = stream.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
            }
            let body: Value =
                serde_json::from_slice(&request[header_end..header_end + content_length]).unwrap();
            let response = json!({
                "schemaVersion": 1,
                "requestId": body["requestId"],
                "advisorVersion": ADVISOR_VERSION,
                "advisorModel": ADVISOR_MODEL,
                "advice": {
                    "schema_version": 1,
                    "assessment": "Inventory before choosing a control.",
                    "recommended_action": {"capability":"security.inventory","arguments":{},"rationale":"Coverage is unknown."},
                    "alternatives": [],
                    "evidence_requirements": ["inventory receipt"],
                    "stop_conditions": ["repository unavailable"],
                    "risk_level": "low",
                    "confidence": 0.91
                },
                "usage": {"cost": 0.02},
                "receipt": {"adviceSha256":"a".repeat(64),"telemetrySha256":"b".repeat(64),"telemetryVersionId":"v1","trainingEligible":false}
            });
            let encoded = response.to_string();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        encoded.len(),
                        encoded
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            body
        });
        let mut config = config(
            json!({"cloud_advisor": {
                "organization_id": "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
                "specialist": "security",
                "workflow": "security:scan",
                "execution_residency": "local_only"
            }}),
            false,
        )
        .unwrap();
        config.base_url = format!("http://{address}/v1");
        let result = CloudAdvisorTool::new(config)
            .invoke_inner(json!({
                "phase": "threat_model",
                "goal": "Choose the next bounded security action",
                "evidence": [{"kind":"inventory","files":42}],
                "candidate_actions": [{
                    "capability":"security.inventory",
                    "description":"Inventory the repository",
                    "constraints":{"read_only":true}
                }]
            }))
            .await
            .unwrap();
        assert_eq!(result["advisorModel"], ADVISOR_MODEL);
        assert_eq!(result["usage"]["cost"], 0.02);
        let sent = server.await.unwrap();
        assert_eq!(sent["executionResidency"], "local_only");
        assert_eq!(sent["trainingConsent"]["eligible"], false);
        assert_eq!(
            sent["candidateActions"][0]["capability"],
            "security.inventory"
        );
    }
}
