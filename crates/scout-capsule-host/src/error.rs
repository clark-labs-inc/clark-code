use thiserror::Error;

pub type CapsuleHostResult<T> = Result<T, CapsuleHostError>;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CapsuleHostError {
    #[error("invalid capsule host limit: {0}")]
    InvalidLimit(&'static str),
    #[error("capsule host policy requires at least one approved module")]
    EmptyApprovalPolicy,
    #[error("capsule host policy contains an invalid module digest")]
    InvalidApprovedDigest,
    #[error("capsule module exceeds its byte limit")]
    ModuleTooLarge,
    #[error("capsule input exceeds its byte limit")]
    InputTooLarge,
    #[error("capsule module is not approved")]
    ModuleNotApproved,
    #[error("capsule module imports an ambient capability")]
    ImportedCapability,
    #[error("capsule module is invalid")]
    InvalidModule,
    #[error("capsule module does not implement the required ABI")]
    InvalidAbi,
    #[error("capsule host has no free concurrency slot")]
    ConcurrencyLimit,
    #[error("capsule invocation exceeded its wall-clock deadline")]
    DeadlineExceeded,
    #[error("capsule invocation exhausted deterministic fuel")]
    FuelExhausted,
    #[error("capsule invocation trapped")]
    GuestTrap,
    #[error("capsule output exceeds its byte limit")]
    OutputTooLarge,
    #[error("capsule memory access is outside the guest allocation")]
    MemoryBounds,
    #[error("capsule worker could not be started or observed")]
    WorkerFailed,
}
