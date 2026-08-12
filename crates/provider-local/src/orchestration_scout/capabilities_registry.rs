use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::{CapabilityReport, DotenvFile, NamedCapability, RoutingCapability, RustFallback};

const ROUTING_TOOLS: &[&str] = &[
    "git",
    "gh",
    "glab",
    "bb",
    "circleci",
    "buildkite-agent",
    "jenkins-cli",
    "jfrog",
    "tkn",
    "aws",
    "az",
    "gcloud",
    "oci",
    "doctl",
    "ibmcloud",
    "hcloud",
    "linode-cli",
    "scw",
    "upctl",
    "civo",
    "vultr-cli",
    "fastly",
    "rg",
    "jq",
    "yq",
    "curl",
    "openssl",
    "ssh",
    "docker",
    "podman",
    "kubectl",
    "helm",
    "kustomize",
    "istioctl",
    "linkerd",
    "cilium",
    "etcdctl",
    "argocd",
    "flux",
    "terraform",
    "tofu",
    "terragrunt",
    "pulumi",
    "cdk",
    "sam",
    "serverless",
    "packer",
    "ansible",
    "ansible-playbook",
    "nomad",
    "consul",
    "vault",
    "boundary",
    "sops",
    "doppler",
    "infisical",
    "okta",
    "op",
    "bw",
    "cloudflared",
    "wrangler",
    "vercel",
    "netlify",
    "flyctl",
    "heroku",
    "railway",
    "sentry-cli",
    "datadog-ci",
    "grafana",
    "promtool",
    "amtool",
    "logcli",
    "mimirtool",
    "newrelic",
    "honeycomb",
    "pd",
    "psql",
    "mysql",
    "mongosh",
    "redis-cli",
    "clickhouse-client",
    "cqlsh",
    "duckdb",
    "bq",
    "gsutil",
    "dbt",
    "snow",
    "databricks",
    "kafka-topics",
    "rpk",
    "stripe",
    "twilio",
    "dig",
    "crane",
    "cosign",
    "oras",
    "trivy",
    "grype",
    "syft",
    "osv-scanner",
    "semgrep",
    "checkov",
    "bwrap",
    "wasmtime",
];

pub(super) fn named_capability(name: String) -> NamedCapability {
    NamedCapability {
        credential_candidate: credential_candidate(&name),
        name,
    }
}

pub(super) fn adapter_executables(executable_names: &[String]) -> Vec<String> {
    let available = executable_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    ROUTING_TOOLS
        .iter()
        .filter(|name| available.contains(**name))
        .map(|name| (*name).to_string())
        .collect()
}

pub(super) fn adapter_environment_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "DATABASE_URL",
        "REDIS_URL",
        "MONGODB_URI",
        "KUBECONFIG",
        "DOCKER_HOST",
        "VAULT_ADDR",
    ]
    .contains(&upper.as_str())
        || [
            "AWS_",
            "AZURE_",
            "ARM_",
            "GOOGLE_",
            "GCLOUD_",
            "CLOUDSDK_",
            "GH_",
            "GITHUB_",
            "GITLAB_",
            "BITBUCKET_",
            "BUILDKITE_",
            "CIRCLECI_",
            "JENKINS_",
            "JFROG_",
            "KUBE_",
            "DOCKER_",
            "CONTAINER_",
            "TF_",
            "TERRAFORM_",
            "PULUMI_",
            "TERRAGRUNT_",
            "VAULT_",
            "BOUNDARY_",
            "NOMAD_",
            "CONSUL_",
            "OKTA_",
            "AUTH0_",
            "ONEPASSWORD_",
            "OP_",
            "SENTRY_",
            "DATADOG_",
            "DD_",
            "OTEL_",
            "NEW_RELIC_",
            "HONEYCOMB_",
            "GRAFANA_",
            "ELASTIC_",
            "SPLUNK_",
            "PAGERDUTY_",
            "CLOUDFLARE_",
            "FASTLY_",
            "VERCEL_",
            "NETLIFY_",
            "FLY_",
            "HEROKU_",
            "RAILWAY_",
            "DOPPLER_",
            "INFISICAL_",
            "OCI_",
            "IBM_CLOUD_",
            "DIGITALOCEAN_",
            "LINODE_",
            "HETZNER_",
            "CIVO_",
            "VULTR_",
            "POSTGRES_",
            "MYSQL_",
            "MONGO_",
            "REDIS_",
            "CLICKHOUSE_",
            "CASSANDRA_",
            "KAFKA_",
            "CONFLUENT_",
            "DATABRICKS_",
            "SNOWFLAKE_",
            "DBT_",
            "STRIPE_",
            "SLACK_",
            "TWILIO_",
            "ATLASSIAN_",
            "JIRA_",
            "LAUNCHDARKLY_",
        ]
        .iter()
        .any(|prefix| upper.starts_with(prefix))
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

pub(super) fn safe_names_hash(names: &[String]) -> String {
    let mut digest = Sha256::new();
    for name in names {
        digest.update(name.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

pub(super) fn safe_fingerprint(report: &CapabilityReport) -> String {
    let mut clone = report.clone();
    clone.fingerprint.clear();
    clone.path_executable_count = 0;
    clone.path_executable_names_sha256.clear();
    clone.environment_name_count = 0;
    clone.environment_names_sha256.clear();
    let encoded = serde_json::to_vec(&clone).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}

pub(super) fn routing_capabilities(
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
    let gcp_auth = [
        "GOOGLE_APPLICATION_CREDENTIALS",
        "CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE",
    ]
    .iter()
    .any(|name| env_set.contains(name))
        || credential_surfaces
            .iter()
            .any(|surface| surface == "gcloud_config");
    routing.insert(
        "gcp_api".into(),
        RoutingCapability {
            state: if gcp_auth {
                "auth_candidate_unverified"
            } else {
                "missing_auth_candidate"
            }
            .into(),
            evidence: if gcp_auth {
                vec!["credential_source:name_only".into()]
            } else {
                Vec::new()
            },
        },
    );
    routing
}

pub(super) fn rust_fallbacks() -> Vec<RustFallback> {
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
            implementation: "target-native scout_adapter GitHub REST with fixed gh fallback".into(),
            state: "available_after_target_auth_verification".into(),
            constraints: vec![
                "requires an authorized token source".into(),
                "requires explicit network authorization".into(),
                "returns only normalized projected metadata and opaque cursor handles".into(),
            ],
        },
        RustFallback {
            capability: "aws_control_plane".into(),
            implementation:
                "target-native scout_adapter with fixed AWS CLI STS, Organizations, and Resource Explorer operations"
                    .into(),
            state: "available_when_target_aws_cli_and_auth_verify".into(),
            constraints: vec![
                "operations and projections are allowlisted; no arbitrary argv".into(),
                "AWS SDK fallback without the CLI remains a missing instrument".into(),
                "Secrets Manager payload reads are never part of Scout".into(),
            ],
        },
        RustFallback {
            capability: "gcp_control_plane".into(),
            implementation:
                "target-native scout_adapter with fixed gcloud organization, folder, project, and Cloud Asset operations"
                    .into(),
            state: "available_when_target_gcloud_and_auth_verify".into(),
            constraints: vec![
                "operations and projections are allowlisted; no arbitrary argv".into(),
                "native Google API/OAuth fallback without gcloud remains a missing instrument"
                    .into(),
                "credential values never enter the census or graph".into(),
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
