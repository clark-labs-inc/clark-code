//! Target-bound, secret-safe wire types for enterprise Scout adapters.
//!
//! This crate deliberately contains no executor, network, cloud SDK, filesystem,
//! or provider implementation. It defines the portable contract that a trusted
//! target-side adapter host must validate before emitting normalized evidence.

mod contract;
mod cursor;
mod error;
mod fingerprint;
mod ids;
mod receipt;
mod record;
mod validate;

pub use contract::{
    AdapterPageLimits, AdapterPageRequest, AdapterQuery, AuthContextDescriptor, AuthSourceKind,
    CoverageBinding, TargetIdentity, ADAPTER_PROTOCOL_VERSION,
};
pub use cursor::CursorVaultBinding;
pub use error::{ProtocolError, ProtocolResult};
pub use ids::{
    AdapterId, AuthContextHandle, AuthContextId, CursorHandle, ReceiptId, RecordId, RequestId,
    TargetId,
};
pub use receipt::{
    AdapterPageOutcome, AdapterPageReceipt, FailureReason, RedactionSummary, TruncationReason,
};
pub use record::{NormalizedLink, NormalizedRecord, SafeFieldValue};

#[cfg(test)]
mod tests;
