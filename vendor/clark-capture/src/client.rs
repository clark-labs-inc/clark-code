use crate::CaptureError;
use crate::event::{
    Breadcrumb, CaptureEvent, EventInput, JsonMap, Level, RuntimeContext, TraceContext,
};
use crate::plugin::CapturePlugin;
use crate::storage::FileEventStore;
use serde_json::{Map, Value, json};
use std::backtrace::Backtrace;
use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use uuid::Uuid;

pub type BeforeCapture = Arc<dyn Fn(CaptureEvent) -> Option<CaptureEvent> + Send + Sync>;

#[derive(Clone)]
pub struct CaptureConfig {
    pub project: String,
    pub root: Option<PathBuf>,
    pub release: Option<String>,
    pub environment: Option<String>,
    pub session_id: Option<String>,
    pub max_breadcrumbs: usize,
    pub before_capture: Option<BeforeCapture>,
}

impl CaptureConfig {
    pub fn from_env(project: impl Into<String>) -> Result<Self, CaptureError> {
        let root = std::env::var_os("CLARK_CAPTURED_LOGS").map(PathBuf::from);
        Ok(Self {
            project: project.into(),
            root,
            release: None,
            environment: None,
            session_id: None,
            max_breadcrumbs: 100,
            before_capture: None,
        })
    }

    pub fn with_root(project: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        Self {
            project: project.into(),
            root: Some(root.into()),
            release: None,
            environment: None,
            session_id: None,
            max_breadcrumbs: 100,
            before_capture: None,
        }
    }
}

#[derive(Default)]
struct Scope {
    tags: BTreeMap<String, String>,
    user: Option<JsonMap>,
    contexts: JsonMap,
    extra: JsonMap,
    breadcrumbs: VecDeque<Breadcrumb>,
}

struct Inner {
    config: CaptureConfig,
    store: Option<FileEventStore>,
    scope: Mutex<Scope>,
    plugins: Vec<Arc<dyn CapturePlugin>>,
}

#[derive(Clone)]
pub struct CaptureClient {
    inner: Arc<Inner>,
}

pub struct CaptureClientBuilder {
    config: CaptureConfig,
    plugins: Vec<Arc<dyn CapturePlugin>>,
}

impl CaptureClientBuilder {
    pub fn plugin(mut self, plugin: Arc<dyn CapturePlugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    pub fn build(self) -> Result<CaptureClient, CaptureError> {
        CaptureClient::with_plugins(self.config, self.plugins)
    }
}

impl CaptureClient {
    pub fn new(config: CaptureConfig) -> Result<Self, CaptureError> {
        Self::with_plugins(config, Vec::new())
    }

    pub fn builder(config: CaptureConfig) -> CaptureClientBuilder {
        CaptureClientBuilder {
            config,
            plugins: Vec::new(),
        }
    }

    fn with_plugins(
        config: CaptureConfig,
        plugins: Vec<Arc<dyn CapturePlugin>>,
    ) -> Result<Self, CaptureError> {
        if config.project.trim().is_empty() {
            return Err(CaptureError::InvalidProject);
        }
        let store = config
            .root
            .clone()
            .map(|root| FileEventStore::new(root, &config.project));
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                store,
                scope: Mutex::new(Scope::default()),
                plugins,
            }),
        })
    }

    pub fn enabled(&self) -> bool {
        self.inner.store.is_some()
    }
    pub fn project(&self) -> &str {
        &self.inner.config.project
    }

    pub fn set_tag(&self, key: impl Into<String>, value: impl Into<String>) {
        self.scope().tags.insert(key.into(), value.into());
    }

    pub fn set_user(&self, user: Option<JsonMap>) {
        self.scope().user = user;
    }
    pub fn set_context(&self, key: impl Into<String>, value: Value) {
        self.scope().contexts.insert(key.into(), value);
    }
    pub fn set_extra(&self, key: impl Into<String>, value: Value) {
        self.scope().extra.insert(key.into(), value);
    }

    pub fn add_breadcrumb(&self, breadcrumb: Breadcrumb) {
        let maximum = self.inner.config.max_breadcrumbs;
        let mut scope = self.scope();
        scope.breadcrumbs.push_back(breadcrumb);
        while scope.breadcrumbs.len() > maximum {
            scope.breadcrumbs.pop_front();
        }
    }

    pub fn capture_message(
        &self,
        message: impl Into<String>,
        level: Level,
    ) -> Result<Option<String>, CaptureError> {
        let mut payload = Map::new();
        payload.insert("message".into(), Value::String(message.into()));
        self.capture(EventInput::new("message", level, payload))
    }

    pub fn capture_log(
        &self,
        message: impl Into<String>,
        level: Level,
        fields: JsonMap,
    ) -> Result<Option<String>, CaptureError> {
        let mut payload = Map::new();
        payload.insert("message".into(), Value::String(message.into()));
        payload.insert("fields".into(), Value::Object(fields));
        self.capture(EventInput::new("log", level, payload))
    }

    pub fn capture_error(
        &self,
        error: &(dyn Error + 'static),
    ) -> Result<Option<String>, CaptureError> {
        let mut exceptions = Vec::new();
        exceptions.push(json!({
            "type": std::any::type_name_of_val(error),
            "value": error.to_string(),
            "stacktrace": Backtrace::force_capture().to_string(),
        }));
        let mut source = error.source();
        while let Some(cause) = source {
            exceptions.push(
                json!({ "type": std::any::type_name_of_val(cause), "value": cause.to_string() }),
            );
            source = cause.source();
        }
        let mut payload = Map::new();
        payload.insert("exceptions".into(), Value::Array(exceptions));
        self.capture(EventInput::new("exception", Level::Error, payload))
    }

    pub fn capture_attachment(
        &self,
        source: impl AsRef<Path>,
        content_type: &str,
    ) -> Result<Option<String>, CaptureError> {
        let Some(store) = &self.inner.store else {
            return Ok(None);
        };
        let timestamp = chrono::Utc::now().to_rfc3339();
        let stored = store.attach(source.as_ref(), &timestamp)?;
        let mut payload = Map::new();
        payload.insert(
            "filename".into(),
            Value::String(
                source
                    .as_ref()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("attachment")
                    .to_owned(),
            ),
        );
        payload.insert(
            "content_type".into(),
            Value::String(content_type.to_owned()),
        );
        payload.insert(
            "path".into(),
            Value::String(stored.relative_path.to_string_lossy().into_owned()),
        );
        payload.insert("sha256".into(), Value::String(stored.sha256));
        payload.insert("size".into(), Value::Number(stored.size.into()));
        let mut input = EventInput::new("attachment", Level::Info, payload);
        input.timestamp = Some(timestamp);
        self.capture(input)
    }

    pub fn start_span(
        &self,
        name: impl Into<String>,
        operation: impl Into<String>,
        parent: Option<&TraceContext>,
    ) -> CaptureSpan {
        CaptureSpan::new(self.clone(), name.into(), operation.into(), parent)
    }

    pub fn capture(&self, input: EventInput) -> Result<Option<String>, CaptureError> {
        let Some(store) = &self.inner.store else {
            return Ok(None);
        };
        let scope = self.scope();
        let mut event = CaptureEvent {
            schema_version: 1,
            event_id: Uuid::new_v4().simple().to_string(),
            timestamp: input
                .timestamp
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
            kind: input.kind,
            level: input.level,
            project: self.inner.config.project.clone(),
            platform: "rust".into(),
            release: self.inner.config.release.clone(),
            environment: self.inner.config.environment.clone(),
            session_id: self.inner.config.session_id.clone(),
            runtime: RuntimeContext {
                language: "rust".into(),
                version: option_env!("CARGO_PKG_RUST_VERSION")
                    .unwrap_or("unknown")
                    .into(),
                pid: std::process::id(),
                executable: std::env::current_exe()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                hostname: hostname::get()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
            },
            tags: scope.tags.clone(),
            user: input.user.or_else(|| scope.user.clone()),
            contexts: scope.contexts.clone(),
            extra: scope.extra.clone(),
            breadcrumbs: input
                .breadcrumbs
                .unwrap_or_else(|| scope.breadcrumbs.iter().cloned().collect()),
            trace: input.trace,
            payload: input.payload,
        };
        drop(scope);
        event.tags.extend(input.tags);
        event.contexts.extend(input.contexts);
        event.extra.extend(input.extra);
        let mut candidate = Some(event);
        if let Some(before_capture) = &self.inner.config.before_capture
            && let Some(current) = candidate.take()
        {
            candidate = before_capture(current);
        }
        for plugin in &self.inner.plugins {
            if let Some(current) = candidate.take() {
                candidate = plugin.before_capture(current);
            } else {
                break;
            }
        }
        let Some(event) = candidate else {
            return Ok(None);
        };
        let path = store.append(&event)?;
        for plugin in &self.inner.plugins {
            plugin.after_capture(&event, &path);
        }
        Ok(Some(event.event_id))
    }

    pub fn install_panic_hook(&self) {
        let client = self.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let location = panic_info.location().map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            });
            let message = panic_info
                .payload()
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| {
                    panic_info
                        .payload()
                        .downcast_ref::<String>()
                        .map(String::as_str)
                })
                .unwrap_or("non-string panic payload");
            let mut payload = Map::new();
            payload.insert("exceptions".into(), json!([{ "type": "panic", "value": message, "location": location, "stacktrace": Backtrace::force_capture().to_string() }]));
            let _ = client.capture(EventInput::new("exception", Level::Fatal, payload));
            previous(panic_info);
        }));
    }

    fn scope(&self) -> std::sync::MutexGuard<'_, Scope> {
        self.inner
            .scope
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct CaptureSpan {
    client: CaptureClient,
    name: String,
    operation: String,
    trace: TraceContext,
    started_at: chrono::DateTime<chrono::Utc>,
    started: Instant,
    attributes: JsonMap,
    ended: bool,
}

impl CaptureSpan {
    fn new(
        client: CaptureClient,
        name: String,
        operation: String,
        parent: Option<&TraceContext>,
    ) -> Self {
        Self {
            client,
            name,
            operation,
            trace: TraceContext {
                trace_id: parent
                    .map(|trace| trace.trace_id.clone())
                    .unwrap_or_else(|| Uuid::new_v4().simple().to_string()),
                span_id: Uuid::new_v4().simple().to_string()[..16].to_owned(),
                parent_span_id: parent.map(|trace| trace.span_id.clone()),
            },
            started_at: chrono::Utc::now(),
            started: Instant::now(),
            attributes: JsonMap::new(),
            ended: false,
        }
    }

    pub fn trace(&self) -> &TraceContext {
        &self.trace
    }
    pub fn set_attribute(&mut self, key: impl Into<String>, value: Value) {
        self.attributes.insert(key.into(), value);
    }

    pub fn end(mut self, status: &str) -> Result<Option<String>, CaptureError> {
        self.ended = true;
        let mut payload = Map::new();
        payload.insert("name".into(), Value::String(self.name.clone()));
        payload.insert("operation".into(), Value::String(self.operation.clone()));
        payload.insert("status".into(), Value::String(status.to_owned()));
        payload.insert(
            "started_at".into(),
            Value::String(self.started_at.to_rfc3339()),
        );
        payload.insert(
            "duration_ms".into(),
            json!(self.started.elapsed().as_secs_f64() * 1000.0),
        );
        payload.insert(
            "attributes".into(),
            Value::Object(std::mem::take(&mut self.attributes)),
        );
        let mut input = EventInput::new(
            "span",
            if status == "error" {
                Level::Error
            } else {
                Level::Info
            },
            payload,
        );
        input.trace = Some(self.trace.clone());
        self.client.capture(input)
    }
}
