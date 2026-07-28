//! Pure normalization boundary for untrusted Scout adapter payloads.
//!
//! [`normalize_json`] is deliberately deterministic and has no filesystem,
//! network, clock, random-number, process, or environment access. It compiles
//! for `wasm32-unknown-unknown`, where the core requests no WASI ambient
//! authority.
//! The input is capped and structurally scanned before deserialization; the
//! normalized output is capped before it crosses back to the host.
//!
//! The input contract contains adapter-produced *candidate records*, not raw
//! AWS, GCP, GitHub, or other provider response shapes. Provider-specific
//! extraction remains outside this boundary until individual adapter capsules
//! exist; this core isolates the common validation and canonicalization step.
//!
//! This crate is **not** a complete sandbox. A production WASM host must:
//!
//! - instantiate a fresh guest for each invocation with no WASI imports;
//! - reject imports that grant filesystem, network, environment, clock, random,
//!   process, or other ambient authority;
//! - statically review and explicitly allow only inert ABI plumbing imports
//!   (the current protocol dependency may contribute `wasm-bindgen` shims);
//! - cap linear memory and table growth independently of [`CapsuleLimits`];
//! - meter fuel and enforce an interruptible wall-clock deadline;
//! - copy no more than `max_input_bytes` into the guest;
//! - discard all guest state and output after any trap or contract error.
//!
//! The versioned serialized request and response types are the portable core
//! wire contract. Exporting [`normalize_json`] through a particular component
//! model or engine is intentionally left to the future host integration so this
//! crate does not imply enforcement it cannot provide.

#![forbid(unsafe_code)]

mod error;
mod limits;
mod model;
mod normalize;
mod scan;

pub use error::{CapsuleError, CapsuleResult};
pub use limits::{CapsuleLimits, CAPSULE_ABI_VERSION};
pub use model::{
    CandidateField, CandidateRecord, CapsuleRequest, CapsuleResponse, NormalizationReceipt,
    NormalizedPage, PAGE_SCHEMA, RECEIPT_SCHEMA, REQUEST_SCHEMA, RESPONSE_SCHEMA,
};
pub use normalize::normalize_json;

#[cfg(test)]
mod tests;
