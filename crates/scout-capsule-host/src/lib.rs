//! Resource-enforced WebAssembly execution for pure Scout capsules.
//!
//! The host accepts only administrator-approved module digests and modules with
//! zero imports. Every invocation gets a fresh instance and store. Wasmi fuel,
//! linear-memory, table, instance, module, input, output, concurrency, and
//! caller-deadline bounds are enforced before a receipt is returned. Wasmi
//! does not hard-preempt a running interpreter thread: after a timeout, that
//! worker retains its concurrency slot until deterministic fuel terminates it.
//! This keeps post-timeout work finite without misrepresenting the deadline as
//! immediate process termination.
//!
//! Capsule ABI v1 exports:
//!
//! - `memory`
//! - `scout_alloc(i32) -> i32`
//! - `scout_run(i32, i32) -> i64`
//!
//! `scout_run` receives the input pointer and length. Its result packs the
//! output pointer in the low 32 bits and output length in the high 32 bits.
//! The instance is discarded after one invocation, so deallocation is neither
//! exported nor trusted.

#![forbid(unsafe_code)]

mod error;
mod host;
mod limits;
mod receipt;
mod registry;
mod service;
mod wire;

pub use error::{CapsuleHostError, CapsuleHostResult};
pub use host::{module_sha256, CapsuleHost};
pub use limits::CapsuleHostLimits;
pub use receipt::{CapsuleInvocation, CapsuleIsolationReceipt};
pub use registry::{CapsuleRegistryEntry, CapsuleRegistryPayload, SignedCapsuleRegistry};
pub use service::dispatch;
pub use wire::{
    CapsuleDescriptor, CapsulePolicyBinding, CapsuleServiceRequest, CapsuleServiceResponse,
    CensusCapsuleRequest, InvokeCapsuleRequest, CAPSULE_SERVICE_PROTOCOL_VERSION, SERVICE_NAME,
};

pub const CAPSULE_HOST_ABI_VERSION: u16 = 1;
pub const CAPSULE_HOST_RUNTIME: &str = "wasmi-0.40";
pub const CAPSULE_ISOLATION_RECEIPT_SCHEMA: &str = "scout-capsule-isolation-receipt-v1";

#[cfg(test)]
mod tests;
