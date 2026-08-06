use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::contract::validate_identifier;

/// Immutable metadata advertised by a plugin. Operations are explicit so a
/// caller can discover the extension surface before sending a command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub operations: BTreeSet<String>,
    #[serde(default)]
    pub capabilities: BTreeSet<String>,
}

impl PluginManifest {
    pub fn validate(&self) -> Result<(), PluginError> {
        validate_identifier("plugin", &self.id)
            .map_err(|error| PluginError::InvalidManifest(error.to_string()))?;
        if self.version.trim().is_empty() || self.description.trim().is_empty() {
            return Err(PluginError::InvalidManifest(
                "plugin version and description are required".into(),
            ));
        }
        if self.operations.is_empty() {
            return Err(PluginError::InvalidManifest(
                "plugin must advertise at least one operation".into(),
            ));
        }
        if self
            .operations
            .iter()
            .chain(self.capabilities.iter())
            .any(|value| value.trim().is_empty() || value.len() > 128)
        {
            return Err(PluginError::InvalidManifest(
                "plugin operation and capability names must be bounded and non-empty".into(),
            ));
        }
        Ok(())
    }
}

/// Per-invocation capabilities supplied by the host. A plugin receives a
/// resolved project root, not an untrusted path from the wire.
#[derive(Clone, Debug)]
pub struct PluginContext {
    pub request_id: String,
    pub project_id: Option<String>,
    pub project_root: Option<PathBuf>,
    pub trajectory_root: PathBuf,
    pub cancellation: CancellationToken,
    pub progress: ProgressReporter,
}

/// Request-scoped, ordered progress channel. The bounded worker output channel
/// applies backpressure all the way into the plugin rather than accumulating
/// an unbounded event transcript in either process.
#[derive(Clone, Debug)]
pub struct ProgressReporter {
    request_id: String,
    output: Option<mpsc::Sender<crate::Response>>,
    next_sequence: Arc<AtomicU64>,
    capture: Option<ProgressCapture>,
}

const MAX_REPLAY_FRAMES: usize = 4_096;
const MAX_REPLAY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub(crate) struct ProgressCapture(Arc<Mutex<CapturedProgress>>);

#[derive(Debug, Default)]
struct CapturedProgress {
    frames: Vec<crate::Response>,
    bytes: usize,
    overflowed: bool,
}

impl ProgressCapture {
    pub(crate) async fn record(&self, frame: &crate::Response) {
        let mut captured = self.0.lock().await;
        if captured.overflowed {
            return;
        }
        let bytes = serde_json::to_vec(frame).map_or(MAX_REPLAY_BYTES + 1, |value| value.len());
        if captured.frames.len() >= MAX_REPLAY_FRAMES
            || captured.bytes.saturating_add(bytes) > MAX_REPLAY_BYTES
        {
            captured.frames.clear();
            captured.bytes = 0;
            captured.overflowed = true;
            return;
        }
        captured.bytes += bytes;
        captured.frames.push(frame.clone());
    }

    pub(crate) async fn finish(&self) -> Result<Vec<crate::Response>, ()> {
        let mut captured = self.0.lock().await;
        if captured.overflowed {
            return Err(());
        }
        captured.frames.sort_by_key(|frame| match frame {
            crate::Response::Progress { sequence, .. } => *sequence,
            crate::Response::Result { .. } | crate::Response::Error { .. } => u64::MAX,
        });
        Ok(std::mem::take(&mut captured.frames))
    }
}

impl ProgressReporter {
    pub(crate) fn new(request_id: String, output: Option<mpsc::Sender<crate::Response>>) -> Self {
        Self {
            request_id,
            output,
            next_sequence: Arc::new(AtomicU64::new(0)),
            capture: None,
        }
    }

    pub(crate) fn with_capture(mut self, capture: ProgressCapture) -> Self {
        self.capture = Some(capture);
        self
    }

    pub fn enabled(&self) -> bool {
        self.output.is_some()
    }

    pub async fn emit(&self, kind: &str, data: Value) -> Result<(), PluginError> {
        validate_identifier("progress kind", kind)
            .map_err(|error| PluginError::Failed(error.to_string()))?;
        let Some(output) = &self.output else {
            return Err(PluginError::Failed(
                "worker progress channel is unavailable".into(),
            ));
        };
        let sequence = self.next_sequence.fetch_add(1, Ordering::SeqCst);
        let frame = crate::Response::progress(self.request_id.clone(), sequence, kind, data);
        if let Some(capture) = &self.capture {
            capture.record(&frame).await;
        }
        output
            .send(frame)
            .await
            .map_err(|_| PluginError::Failed("worker progress channel closed".into()))
    }
}

#[async_trait]
pub trait HeadlessPlugin: Send + Sync {
    fn manifest(&self) -> &PluginManifest;

    async fn invoke(
        &self,
        context: PluginContext,
        operation: &str,
        input: Value,
    ) -> Result<Value, PluginError>;
}

#[derive(Clone, Default)]
pub struct PluginRegistry {
    plugins: BTreeMap<String, Arc<dyn HeadlessPlugin>>,
}

impl PluginRegistry {
    pub fn register<P>(&mut self, plugin: P) -> Result<(), PluginError>
    where
        P: HeadlessPlugin + 'static,
    {
        let manifest = plugin.manifest().clone();
        manifest.validate()?;
        if self.plugins.contains_key(&manifest.id) {
            return Err(PluginError::Duplicate(manifest.id));
        }
        self.plugins.insert(manifest.id.clone(), Arc::new(plugin));
        Ok(())
    }

    pub fn catalog(&self) -> Vec<PluginManifest> {
        self.plugins
            .values()
            .map(|plugin| plugin.manifest().clone())
            .collect()
    }

    pub(crate) async fn invoke(
        &self,
        plugin_id: &str,
        operation: &str,
        context: PluginContext,
        input: Value,
    ) -> Result<Value, PluginError> {
        let plugin = self
            .plugins
            .get(plugin_id)
            .ok_or_else(|| PluginError::UnknownPlugin(plugin_id.to_string()))?;
        if !plugin.manifest().operations.contains(operation) {
            return Err(PluginError::UnsupportedOperation {
                plugin: plugin_id.to_string(),
                operation: operation.to_string(),
            });
        }
        plugin.invoke(context, operation, input).await
    }
}

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("unknown plugin: {0}")]
    UnknownPlugin(String),
    #[error("plugin {plugin} does not support operation {operation}")]
    UnsupportedOperation { plugin: String, operation: String },
    #[error("duplicate plugin: {0}")]
    Duplicate(String),
    #[error("invalid plugin manifest: {0}")]
    InvalidManifest(String),
    #[error("invalid plugin input: {0}")]
    InvalidInput(String),
    #[error("plugin cancelled")]
    Cancelled,
    #[error("plugin failed: {0}")]
    Failed(String),
}
