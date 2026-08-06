//! Clark Scientist/RSI provider adapter.
//!
//! The adapter owns a bounded `clark-code-headless` child for each turn and
//! translates its strict JSONL control protocol into normal `agent-core`
//! events. The WebView never owns a worker process, model credential, runtime
//! path, or private research trajectory.

mod config;
mod protocol;
mod provider;
mod transport;

pub use config::{prepare_native_config, SpecialistConnectConfig};
pub use provider::SpecialistProvider;
