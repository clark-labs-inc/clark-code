use crate::{CaptureClient, JsonMap, Level};
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

pub struct CaptureLayer {
    client: CaptureClient,
}

impl CaptureLayer {
    pub fn new(client: CaptureClient) -> Self {
        Self { client }
    }
}

#[derive(Default)]
struct JsonVisitor {
    fields: JsonMap,
}

impl Visit for JsonVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().into(), Value::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().into(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().into(), Value::Number(value.into()));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields.insert(
            field.name().into(),
            serde_json::Number::from_f64(value)
                .map(Value::Number)
                .unwrap_or_else(|| Value::String(value.to_string())),
        );
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().into(), Value::String(value.into()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().into(), Value::String(format!("{value:?}")));
    }
}
impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);
        let message = visitor
            .fields
            .remove("message")
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| metadata.name().to_owned());
        visitor
            .fields
            .insert("target".into(), Value::String(metadata.target().into()));
        if let Some(module) = metadata.module_path() {
            visitor
                .fields
                .insert("module".into(), Value::String(module.into()));
        }
        if let Some(file) = metadata.file() {
            visitor
                .fields
                .insert("file".into(), Value::String(file.into()));
        }
        if let Some(line) = metadata.line() {
            visitor
                .fields
                .insert("line".into(), Value::Number(line.into()));
        }
        let level = match *metadata.level() {
            tracing::Level::TRACE => Level::Trace,
            tracing::Level::DEBUG => Level::Debug,
            tracing::Level::INFO => Level::Info,
            tracing::Level::WARN => Level::Warning,
            tracing::Level::ERROR => Level::Error,
        };
        let _ = self.client.capture_log(message, level, visitor.fields);
    }
}
