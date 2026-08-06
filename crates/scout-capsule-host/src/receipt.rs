use serde::{Deserialize, Serialize};

use crate::{CapsuleHostLimits, CAPSULE_HOST_ABI_VERSION, CAPSULE_ISOLATION_RECEIPT_SCHEMA};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapsuleInvocation {
    pub output: Vec<u8>,
    pub receipt: CapsuleIsolationReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleIsolationReceipt {
    pub schema: String,
    pub abi_version: u16,
    pub runtime: String,
    pub module_sha256: String,
    pub import_set: Vec<String>,
    pub fresh_instance: bool,
    pub wasi_enabled: bool,
    pub limits: CapsuleHostLimits,
    pub input_sha256: String,
    pub output_sha256: String,
    pub fuel_consumed: u64,
    pub elapsed_micros: u64,
}

impl CapsuleIsolationReceipt {
    pub(crate) fn new(
        runtime: &str,
        module_sha256: String,
        limits: CapsuleHostLimits,
        input_sha256: String,
        output_sha256: String,
        fuel_consumed: u64,
        elapsed_micros: u64,
    ) -> Self {
        Self {
            schema: CAPSULE_ISOLATION_RECEIPT_SCHEMA.to_owned(),
            abi_version: CAPSULE_HOST_ABI_VERSION,
            runtime: runtime.to_owned(),
            module_sha256,
            import_set: Vec::new(),
            fresh_instance: true,
            wasi_enabled: false,
            limits,
            input_sha256,
            output_sha256,
            fuel_consumed,
            elapsed_micros,
        }
    }
}
