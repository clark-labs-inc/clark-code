mod client;
mod event;
mod plugin;
mod storage;
#[cfg(feature = "tracing")]
mod tracing_layer;

pub use client::{BeforeCapture, CaptureClient, CaptureClientBuilder, CaptureConfig, CaptureSpan};
pub use event::{
    Breadcrumb, CaptureEvent, EventInput, JsonMap, Level, RuntimeContext, TraceContext,
};
pub use plugin::{CapturePlugin, WandbMetricsPlugin};
pub use storage::{FileEventStore, StoredAttachment, sanitize_segment};
#[cfg(feature = "tracing")]
pub use tracing_layer::CaptureLayer;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("I/O error while capturing locally: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not serialize capture event: {0}")]
    Json(#[from] serde_json::Error),
    #[error("capture project must not be empty")]
    InvalidProject,
    #[error("metric value is not finite: {0}")]
    InvalidMetric(String),
    #[error("capture is disabled because CLARK_CAPTURED_LOGS is unset")]
    Disabled,
}
