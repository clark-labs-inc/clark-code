use crate::tools::{ToolCtx, ToolExecutor, ToolOutcome};
use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[path = "dotenv.rs"]
mod dotenv;
#[cfg(test)]
use dotenv::dotenv_keys;
use dotenv::scan_dotenv;

#[path = "capabilities_registry.rs"]
mod registry;

use registry::{
    adapter_environment_name, adapter_executables, named_capability, routing_capabilities,
    rust_fallbacks, safe_fingerprint, safe_names_hash,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CapabilityReport {
    pub schema_version: String,
    pub platform: String,
    pub architecture: String,
    pub scope: String,
    pub adapter_executable_names: Vec<String>,
    pub path_executable_count: usize,
    pub path_executable_names_sha256: String,
    pub environment: Vec<NamedCapability>,
    pub environment_name_count: usize,
    pub environment_names_sha256: String,
    pub dotenv_files: Vec<DotenvFile>,
    pub credential_surfaces: Vec<String>,
    pub routing: BTreeMap<String, RoutingCapability>,
    pub fallbacks: Vec<RustFallback>,
    pub truncated: CensusTruncation,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct NamedCapability {
    pub name: String,
    pub credential_candidate: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct DotenvFile {
    pub path: String,
    pub keys: Vec<NamedCapability>,
    pub keys_truncated: bool,
    pub schema_sha256: String,
    pub template: bool,
    pub skipped_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RoutingCapability {
    pub state: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RustFallback {
    pub capability: String,
    pub implementation: String,
    pub state: String,
    pub constraints: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CensusTruncation {
    pub executables: bool,
    pub environment_names: bool,
    pub dotenv_files: bool,
}

pub(super) struct ScoutCapabilitiesTool;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityArgs {
    scope: String,
}

#[async_trait]
impl ToolExecutor for ScoutCapabilitiesTool {
    fn name(&self) -> &str {
        "scout_capabilities"
    }

    fn description(&self) -> &str {
        "Run Scout's secret-safe adapter bootstrap. Reports curated DevOps/cloud adapter executables, relevant environment-variable names, known credential surfaces, and scoped .env key schemas without executing discovered binaries or returning values. Other PATH entries contribute only a count and digest. This is a non-authoritative capability observation, not a host or business-system map."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "description": "Declared workspace or graph-discovered component root to inspect for configuration schemas. Use `.` or one exact host-approved read_only_root from environment context; never use an unrelated home or system root."
                }
            },
            "required": ["scope"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: CapabilityArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => {
                return ToolOutcome::error(format!("invalid Scout capability request: {error}"))
            }
        };
        let scope = match ctx.sandbox.resolve_existing(&args.scope) {
            Ok(scope) => scope,
            Err(error) => return ToolOutcome::error(error),
        };
        let system = match ctx.executor.system_capability_census().await {
            Ok(system) => system,
            Err(error) => {
                return ToolOutcome::error(format!("system capability census failed: {error}"))
            }
        };
        let (dotenv_files, dotenv_truncated) = scan_dotenv(ctx, &scope).await;
        let scope = {
            let displayed = ctx.sandbox.display(&scope);
            if displayed.is_empty() {
                ".".to_string()
            } else {
                displayed
            }
        };
        let path_executable_count = system.executable_names.len();
        let path_executable_names_sha256 = safe_names_hash(&system.executable_names);
        let adapter_executable_names = adapter_executables(&system.executable_names);
        let environment_name_count = system.environment_variable_names.len();
        let environment_names_sha256 = safe_names_hash(&system.environment_variable_names);
        let environment = system
            .environment_variable_names
            .iter()
            .filter(|name| adapter_environment_name(name))
            .cloned()
            .map(named_capability)
            .collect::<Vec<_>>();
        let routing = routing_capabilities(
            &system.executable_names,
            &environment,
            &system.credential_surfaces,
            &dotenv_files,
        );
        let mut report = CapabilityReport {
            schema_version: "scout-adapter-census-v2".into(),
            platform: system.platform,
            architecture: system.architecture,
            scope,
            adapter_executable_names,
            path_executable_count,
            path_executable_names_sha256,
            environment,
            environment_name_count,
            environment_names_sha256,
            dotenv_files,
            credential_surfaces: system.credential_surfaces,
            routing,
            fallbacks: rust_fallbacks(),
            truncated: CensusTruncation {
                executables: system.executables_truncated,
                environment_names: system.environment_names_truncated,
                dotenv_files: dotenv_truncated,
            },
            fingerprint: String::new(),
        };
        report.fingerprint = safe_fingerprint(&report);
        let details = match serde_json::to_value(&report) {
            Ok(details) => details,
            Err(error) => {
                return ToolOutcome::error(format!(
                    "capability report serialization failed: {error}"
                ))
            }
        };
        ToolOutcome::ok(format!(
            "Scout adapter bootstrap observed {} curated adapter executables, {} relevant environment-variable names, and {} scoped .env schemas. Other PATH entries were retained only as a count and digest. Values were not returned. This observation is not enterprise graph authority.",
            details["adapter_executable_names"].as_array().map_or(0, Vec::len),
            details["environment"].as_array().map_or(0, Vec::len),
            details["dotenv_files"].as_array().map_or(0, Vec::len),
        ))
        .with_model_visible_details(details)
    }
}

impl PartialEq for NamedCapability {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for NamedCapability {}

impl PartialOrd for NamedCapability {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NamedCapability {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
