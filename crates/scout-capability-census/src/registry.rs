use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

const TOOL_GROUPS: &[(&str, &[&str])] = &[
    (
        "source_and_ci",
        &[
            "git",
            "gh",
            "glab",
            "bb",
            "circleci",
            "buildkite-agent",
            "jenkins-cli",
            "jfrog",
            "tkn",
        ],
    ),
    (
        "cloud",
        &[
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
        ],
    ),
    ("inspection", &["rg", "jq", "yq", "curl", "openssl", "ssh"]),
    (
        "containers_and_orchestration",
        &[
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
            "crane",
            "cosign",
            "oras",
        ],
    ),
    (
        "infrastructure_as_code",
        &[
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
        ],
    ),
    (
        "secrets_and_identity",
        &[
            "vault",
            "boundary",
            "sops",
            "doppler",
            "infisical",
            "okta",
            "auth0",
            "op",
            "bw",
        ],
    ),
    (
        "edge_and_hosting",
        &[
            "cloudflared",
            "wrangler",
            "vercel",
            "netlify",
            "flyctl",
            "heroku",
            "railway",
        ],
    ),
    (
        "observability",
        &[
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
        ],
    ),
    (
        "data",
        &[
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
        ],
    ),
    ("business_services", &["stripe", "twilio", "dig"]),
    (
        "security",
        &[
            "trivy",
            "grype",
            "syft",
            "osv-scanner",
            "semgrep",
            "checkov",
        ],
    ),
    ("isolation", &["bwrap", "wasmtime"]),
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CuratedExecutable {
    pub name: String,
    pub category: String,
    pub state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RustFallbackGap {
    pub missing_tool: String,
    pub capability: String,
    pub implementation: String,
    pub state: String,
    pub constraints: Vec<String>,
}

pub(super) fn curated_executables(available: &BTreeSet<String>) -> Vec<CuratedExecutable> {
    TOOL_GROUPS
        .iter()
        .flat_map(|(category, tools)| {
            tools.iter().map(|tool| CuratedExecutable {
                name: (*tool).into(),
                category: (*category).into(),
                state: if available.contains(*tool) {
                    "present"
                } else {
                    "missing"
                }
                .into(),
            })
        })
        .collect()
}

pub(super) fn rust_fallback_gaps(executables: &[CuratedExecutable]) -> Vec<RustFallbackGap> {
    let missing = executables
        .iter()
        .filter(|entry| entry.state == "missing")
        .map(|entry| entry.name.as_str())
        .collect::<BTreeSet<_>>();
    let definitions = [
        (
            "jq",
            "json_query_and_measurement",
            "serde_json typed projection",
            "available",
            &["bounded structured inputs"][..],
        ),
        (
            "rg",
            "bounded_content_search",
            "grep-searcher and grep-regex",
            "available",
            &["explicit roots and host-enforced byte/count limits"][..],
        ),
        (
            "curl",
            "fixed_http_queries",
            "reqwest with rustls",
            "available_after_authorization",
            &[
                "fixed allowlisted routes only",
                "network and target authorization still required",
            ][..],
        ),
        (
            "gh",
            "github_control_plane",
            "Scout fixed GitHub REST adapter",
            "available_after_authorization",
            &[
                "authorized token source required",
                "opaque cursors and normalized metadata only",
            ][..],
        ),
        (
            "aws",
            "aws_control_plane",
            "native AWS API adapter",
            "missing",
            &[
                "AWS CLI fixed routes remain the current instrument",
                "pure-Rust SDK fallback is not implemented",
                "secret payload reads are out of scope",
            ][..],
        ),
        (
            "gcloud",
            "gcp_control_plane",
            "native Google Cloud API adapter",
            "missing",
            &[
                "gcloud fixed routes remain the current instrument",
                "pure-Rust OAuth/API fallback is not implemented",
                "credential values are out of scope",
            ][..],
        ),
        (
            "bwrap",
            "os_process_isolation",
            "portable isolation boundary",
            "partial",
            &[
                "OS sandboxing is platform-specific",
                "WASM isolates pure transforms but cannot safely inspect ambient hosts",
            ][..],
        ),
        (
            "wasmtime",
            "wasm_transform_capsule",
            "embedded WASM runtime",
            "missing",
            &[
                "no embedded runtime is linked into this census executable",
                "WASM is appropriate only for pure parsing and reconciliation",
            ][..],
        ),
    ];
    definitions
        .into_iter()
        .filter(|(tool, ..)| missing.contains(*tool))
        .map(
            |(missing_tool, capability, implementation, state, constraints)| RustFallbackGap {
                missing_tool: missing_tool.into(),
                capability: capability.into(),
                implementation: implementation.into(),
                state: state.into(),
                constraints: constraints.iter().map(|value| (*value).into()).collect(),
            },
        )
        .collect()
}

pub(super) fn relevant_environment_name(name: &str) -> bool {
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
