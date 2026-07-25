use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::ScoutToolState;
use crate::tools::{ToolCtx, ToolExecutor, ToolOutcome};

#[path = "dotenv.rs"]
mod dotenv;
#[cfg(test)]
use dotenv::dotenv_keys;
use dotenv::scan_dotenv;

const ROUTING_TOOLS: &[&str] = &[
    "git",
    "gh",
    "aws",
    "rg",
    "fd",
    "jq",
    "curl",
    "wget",
    "ssh",
    "cargo",
    "rustc",
    "node",
    "npm",
    "pnpm",
    "python3",
    "python",
    "docker",
    "podman",
    "kubectl",
    "helm",
    "terraform",
    "pulumi",
    "bwrap",
    "wasmtime",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CapabilityReport {
    pub id: String,
    pub schema_version: String,
    pub platform: String,
    pub architecture: String,
    pub scope: String,
    pub executable_names: Vec<String>,
    pub environment: Vec<NamedCapability>,
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

pub(super) struct ScoutCapabilitiesTool {
    pub state: Arc<ScoutToolState>,
}

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
        "Run Scout's mandatory secret-safe capability census before starting a Scout ledger. Enumerates executable names, environment-variable names, known credential surfaces, and .env file key names without executing discovered binaries or returning any environment/.env values. Returns a census id required by scout_ledger start."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "description": "Directory to inspect for .env files, relative to the project root. Use \".\" for the full project."
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
        let id = format!("census-{}", Uuid::new_v4());
        let environment = system
            .environment_variable_names
            .into_iter()
            .map(named_capability)
            .collect::<Vec<_>>();
        let routing = routing_capabilities(
            &system.executable_names,
            &environment,
            &system.credential_surfaces,
            &dotenv_files,
        );
        let mut report = CapabilityReport {
            id: id.clone(),
            schema_version: "scout-capability-census-v1".into(),
            platform: system.platform,
            architecture: system.architecture,
            scope,
            executable_names: system.executable_names,
            environment,
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
        self.state
            .censuses
            .lock()
            .expect("Scout census lock")
            .insert(id.clone(), report);

        ToolOutcome::ok(format!(
            "Scout capability census `{id}` recorded: {} executable names, {} environment-variable names, and {} .env files. Values were not read into the report. Use this census id when starting scout_ledger.",
            details["executable_names"].as_array().map_or(0, Vec::len),
            details["environment"].as_array().map_or(0, Vec::len),
            details["dotenv_files"].as_array().map_or(0, Vec::len),
        ))
        .with_details(details)
    }
}

fn named_capability(name: String) -> NamedCapability {
    NamedCapability {
        credential_candidate: credential_candidate(&name),
        name,
    }
}

fn credential_candidate(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "PRIVATE_KEY",
        "ACCESS_KEY",
        "API_KEY",
        "AUTH",
        "COOKIE",
        "SESSION",
    ]
    .iter()
    .any(|marker| upper.contains(marker))
}

fn safe_fingerprint(report: &CapabilityReport) -> String {
    let mut clone = report.clone();
    clone.id.clear();
    clone.fingerprint.clear();
    let encoded = serde_json::to_vec(&clone).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}

fn routing_capabilities(
    executables: &[String],
    environment: &[NamedCapability],
    credential_surfaces: &[String],
    dotenv_files: &[DotenvFile],
) -> BTreeMap<String, RoutingCapability> {
    let executable_set = executables
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let env_set = environment
        .iter()
        .map(|entry| entry.name.as_str())
        .chain(
            dotenv_files
                .iter()
                .flat_map(|file| file.keys.iter().map(|entry| entry.name.as_str())),
        )
        .collect::<BTreeSet<_>>();
    let mut routing = ROUTING_TOOLS
        .iter()
        .map(|tool| {
            let present = executable_set.contains(tool);
            (
                (*tool).to_string(),
                RoutingCapability {
                    state: if present { "present" } else { "missing" }.into(),
                    evidence: if present {
                        vec![format!("executable:{tool}")]
                    } else {
                        Vec::new()
                    },
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let aws_auth = [
        "AWS_ACCESS_KEY_ID",
        "AWS_PROFILE",
        "AWS_WEB_IDENTITY_TOKEN_FILE",
    ]
    .iter()
    .any(|name| env_set.contains(name))
        || credential_surfaces
            .iter()
            .any(|surface| surface.starts_with("aws_"));
    routing.insert(
        "aws_secrets_manager".into(),
        RoutingCapability {
            state: if aws_auth {
                "auth_candidate_unverified"
            } else {
                "missing_auth_candidate"
            }
            .into(),
            evidence: if aws_auth {
                vec!["credential_source:name_only".into()]
            } else {
                Vec::new()
            },
        },
    );
    let github_auth = ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .any(|name| env_set.contains(name))
        || credential_surfaces
            .iter()
            .any(|surface| surface == "github_cli_hosts");
    routing.insert(
        "github_api".into(),
        RoutingCapability {
            state: if github_auth {
                "auth_candidate_unverified"
            } else {
                "missing_auth_candidate"
            }
            .into(),
            evidence: if github_auth {
                vec!["credential_source:name_only".into()]
            } else {
                Vec::new()
            },
        },
    );
    routing
}

fn rust_fallbacks() -> Vec<RustFallback> {
    vec![
        RustFallback {
            capability: "dotenv_and_environment_inventory".into(),
            implementation: "scout_capabilities".into(),
            state: "available".into(),
            constraints: vec!["names and locations only; values are never returned".into()],
        },
        RustFallback {
            capability: "json_query_and_measurement".into(),
            implementation: "serde_json plus Scout's typed measurement runner".into(),
            state: "available".into(),
            constraints: vec!["bounded project-scope inputs".into()],
        },
        RustFallback {
            capability: "github_api".into(),
            implementation: "reqwest GitHub REST adapter".into(),
            state: "design_ready_not_enabled".into(),
            constraints: vec![
                "requires an authorized token source".into(),
                "requires explicit network authorization".into(),
            ],
        },
        RustFallback {
            capability: "aws_secrets_manager".into(),
            implementation: "AWS SDK for Rust adapter".into(),
            state: "design_ready_not_enabled".into(),
            constraints: vec![
                "census may test identity/authorization but never fetch secret payloads".into(),
                "secret reads require a separate explicit user-authorized tool".into(),
            ],
        },
        RustFallback {
            capability: "shell_process_isolation".into(),
            implementation: "capability-attested native runner".into(),
            state: "unavailable_without_os_boundary".into(),
            constraints: vec![
                "WASM is suitable for pure transforms, not ambient host inspection".into(),
                "remote shell execution is not treated as isolated".into(),
            ],
        },
    ]
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
