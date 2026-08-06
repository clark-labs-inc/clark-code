use crate::{CaptureClient, CaptureError, CaptureEvent};
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

pub trait CapturePlugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn before_capture(&self, event: CaptureEvent) -> Option<CaptureEvent> {
        Some(event)
    }
    fn after_capture(&self, _event: &CaptureEvent, _path: &std::path::Path) {}
}
#[derive(Clone, Default)]
pub struct WandbMetricsPlugin;

impl WandbMetricsPlugin {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }

    pub fn log<I, K>(
        &self,
        client: &CaptureClient,
        metrics: I,
        step: Option<u64>,
    ) -> Result<Vec<String>, CaptureError>
    where
        I: IntoIterator<Item = (K, f64)>,
        K: Into<String>,
    {
        metrics
            .into_iter()
            .map(|(name, value)| {
                let number = Number::from_f64(value)
                    .ok_or_else(|| CaptureError::InvalidMetric(value.to_string()))?;
                let mut payload = Map::new();
                payload.insert("name".into(), Value::String(name.into()));
                payload.insert("value".into(), Value::Number(number));
                if let Some(step) = step {
                    payload.insert("step".into(), Value::Number(step.into()));
                }
                let mut tags = BTreeMap::new();
                tags.insert("source".into(), "wandb".into());
                let mut input = crate::EventInput::new("metric", crate::Level::Info, payload);
                input.tags = tags;
                client.capture(input)?.ok_or(CaptureError::Disabled)
            })
            .collect()
    }
}

impl CapturePlugin for WandbMetricsPlugin {
    fn name(&self) -> &'static str {
        "wandb-metrics"
    }
}
