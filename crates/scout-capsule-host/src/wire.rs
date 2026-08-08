use serde::{Deserialize, Serialize};

use crate::CapsuleIsolationReceipt;

pub const SERVICE_NAME: &str = "scout-capsule-v1";
pub const CAPSULE_SERVICE_PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsulePolicyBinding {
    pub protocol_version: u16,
    pub authorized_tenant_id: String,
    pub trusted_admin_key_sha256: String,
    pub minimum_registry_generation: u64,
    pub target_id: String,
    pub target_identity_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CensusCapsuleRequest {
    pub policy: CapsulePolicyBinding,
    pub enterprise_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvokeCapsuleRequest {
    pub policy: CapsulePolicyBinding,
    pub capsule_id: String,
    pub enterprise_id: String,
    pub input_schema: String,
    pub output_schema: String,
    pub input_base64: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CapsuleServiceRequest {
    Census(CensusCapsuleRequest),
    Invoke(InvokeCapsuleRequest),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleDescriptor {
    pub capsule_id: String,
    pub abi_version: u16,
    pub input_schema: String,
    pub output_schema: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapsuleServiceResponse {
    Census {
        registry_sha256: String,
        generation: u64,
        capsules: Vec<CapsuleDescriptor>,
    },
    Invoked {
        registry_sha256: String,
        generation: u64,
        capsule_id: String,
        enterprise_id: String,
        target_id: String,
        output_base64: String,
        isolation: Box<CapsuleIsolationReceipt>,
        deadline_is_hard_interrupt: bool,
    },
    Failed {
        code: String,
        message: String,
    },
}
