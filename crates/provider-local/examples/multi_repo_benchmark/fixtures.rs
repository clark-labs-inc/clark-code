use std::collections::BTreeSet;

use super::model::{
    Capability, ContractEdge, FaultInjection, FileFixture, HiddenCheck, RepositorySpec, Scenario,
};

pub fn catalog() -> Vec<Scenario> {
    vec![
        api_sdk_web(),
        rolling_event(),
        generated_client(),
        targeted_recovery(),
        cloud_auth(),
        dependency_chain(),
        baseline_drift(),
    ]
}

fn api_sdk_web() -> Scenario {
    scenario(
        "api-sdk-web",
        "cross_repo_contract",
        "Propagate request IDs from API to web",
        "Add a request_id to the API response, expose it through the SDK, and render it in the web client without breaking the existing message field.",
        vec![
            repo(
                "api",
                &[('p', "api.py", "def create(message):\n    return {'message': message}\n")],
                &[('p', "api.py", "def create(message):\n    return {'message': message, 'request_id': 'req-42'}\n")],
                false,
            ),
            repo(
                "sdk",
                &[('p', "client.py", "def normalize(payload):\n    return {'message': payload['message']}\n")],
                &[('p', "client.py", "def normalize(payload):\n    return {'message': payload['message'], 'request_id': payload['request_id']}\n")],
                false,
            ),
            repo(
                "web",
                &[('p', "view.py", "def render(model):\n    return model['message']\n")],
                &[('p', "view.py", "def render(model):\n    return f\"{model['message']} [{model['request_id']}]\"\n")],
                false,
            ),
        ],
        vec![edge("request-envelope", "api", &["sdk", "web"], "JSON response", "message remains compatible; request_id is propagated unchanged")],
        vec![python(
            "end_to_end_request_id",
            r#"
import importlib.util, pathlib, sys
root = pathlib.Path(sys.argv[1]) / "repos"
def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod); return mod
api = load("api_mod", root / "api" / "api.py")
sdk = load("sdk_mod", root / "sdk" / "client.py")
web = load("web_mod", root / "web" / "view.py")
assert web.render(sdk.normalize(api.create("saved"))) == "saved [req-42]"
"#,
        )],
        standard_capabilities(true),
        true,
        true,
        FaultInjection::None,
    )
}

fn rolling_event() -> Scenario {
    scenario(
        "rolling-event-compatibility",
        "rolling_compatibility",
        "Roll an event contract across old and new consumers",
        "Publish event v2 with display_name while keeping the old consumer working during a rolling deploy. Update the new consumer to prefer display_name.",
        vec![
            repo(
                "producer",
                &[('p', "event.py", "def emit(name):\n    return {'version': 1, 'name': name}\n")],
                &[('p', "event.py", "def emit(name):\n    return {'version': 2, 'name': name, 'display_name': name.upper()}\n")],
                false,
            ),
            repo(
                "old-consumer",
                &[('p', "consume.py", "def consume(event):\n    return event['name']\n")],
                &[],
                false,
            ),
            repo(
                "new-consumer",
                &[('p', "consume.py", "def consume(event):\n    return event['name']\n")],
                &[('p', "consume.py", "def consume(event):\n    return event.get('display_name', event['name'])\n")],
                false,
            ),
        ],
        vec![edge("event-v2", "producer", &["old-consumer", "new-consumer"], "event payload", "v2 retains name until old-consumer is retired")],
        vec![python(
            "old_and_new_consumers",
            r#"
import importlib.util, pathlib, sys
root = pathlib.Path(sys.argv[1]) / "repos"
def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod); return mod
event = load("producer", root / "producer" / "event.py").emit("Ada")
assert load("old", root / "old-consumer" / "consume.py").consume(event) == "Ada"
assert load("new", root / "new-consumer" / "consume.py").consume(event) == "ADA"
"#,
        )],
        standard_capabilities(true),
        true,
        true,
        FaultInjection::None,
    )
}

fn generated_client() -> Scenario {
    scenario(
        "generated-client-staleness",
        "generated_artifacts",
        "Keep a generated client synchronized with its schema",
        "Add priority to the service schema and implementation, then regenerate the client contract marker. Hand editing only the generated client is invalid.",
        vec![
            repo(
                "service",
                &[
                    ('p', "schema.txt", "task:v1:id,title\n"),
                    ('p', "service.py", "def task():\n    return {'id': 7, 'title': 'ship'}\n"),
                ],
                &[
                    ('p', "schema.txt", "task:v2:id,title,priority\n"),
                    ('p', "service.py", "def task():\n    return {'id': 7, 'title': 'ship', 'priority': 'high'}\n"),
                ],
                false,
            ),
            repo(
                "generated-client",
                &[('p', "generated.py", "SCHEMA = 'task:v1:id,title'\nFIELDS = ('id', 'title')\n")],
                &[('p', "generated.py", "SCHEMA = 'task:v2:id,title,priority'\nFIELDS = ('id', 'title', 'priority')\n")],
                false,
            ),
        ],
        vec![edge("task-schema", "service", &["generated-client"], "schema.txt", "generated SCHEMA must exactly match the service schema")],
        vec![python(
            "schema_and_client_match",
            r#"
import importlib.util, pathlib, sys
root = pathlib.Path(sys.argv[1]) / "repos"
spec = importlib.util.spec_from_file_location("generated", root / "generated-client" / "generated.py")
generated = importlib.util.module_from_spec(spec); spec.loader.exec_module(generated)
schema = (root / "service" / "schema.txt").read_text().strip()
assert generated.SCHEMA == schema
assert generated.FIELDS == ('id', 'title', 'priority')
"#,
        )],
        standard_capabilities(true),
        true,
        true,
        FaultInjection::StaleGeneratedClient,
    )
}

fn targeted_recovery() -> Scenario {
    let mut capabilities = standard_capabilities(true);
    capabilities.insert(Capability::TargetedRecovery);
    scenario(
        "targeted-child-recovery",
        "failure_recovery",
        "Recover one failed writer without discarding good work",
        "Update the worker retry contract and CLI output. One writer will fail after producing an artifact; retry only that workstream and preserve unrelated work and user notes.",
        vec![
            repo(
                "worker",
                &[
                    ('p', "retry.py", "MAX_ATTEMPTS = 2\n"),
                    ('d', "notes/local.txt", "user debugging notes\n"),
                ],
                &[('p', "retry.py", "MAX_ATTEMPTS = 4\n")],
                false,
            ),
            repo(
                "cli",
                &[('p', "status.py", "def status(attempts):\n    return f'attempts={attempts}'\n")],
                &[('p', "status.py", "def status(attempts):\n    return f'retry-attempts={attempts}'\n")],
                false,
            ),
        ],
        vec![edge("retry-count", "worker", &["cli"], "retry policy", "CLI reports the worker retry count")],
        vec![
            HiddenCheck::FileContains { repo: "worker".into(), path: "retry.py".into(), needle: "MAX_ATTEMPTS = 4".into() },
            HiddenCheck::FileContains { repo: "cli".into(), path: "status.py".into(), needle: "retry-attempts".into() },
        ],
        capabilities,
        true,
        false,
        FaultInjection::ChildCrashAfterArtifact,
    )
}

fn cloud_auth() -> Scenario {
    scenario(
        "cloud-local-auth-rollout",
        "cloud_coordination",
        "Coordinate auth changes across local and cloud-only repositories",
        "Add audience validation to auth-api, send the audience from mobile, and update the cloud deployment policy. The infra repository is only available to a cloud worker.",
        vec![
            repo("auth-api", &[('p', "auth.py", "def validate(token):\n    return token == 'ok'\n")], &[('p', "auth.py", "def validate(token, audience):\n    return token == 'ok' and audience == 'mobile'\n")], false),
            repo("mobile", &[('p', "request.py", "def auth_request(token):\n    return {'token': token}\n")], &[('p', "request.py", "def auth_request(token):\n    return {'token': token, 'audience': 'mobile'}\n")], false),
            repo("infra", &[('p', "policy.txt", "audience=legacy\n")], &[('p', "policy.txt", "audience=mobile\n")], true),
        ],
        vec![edge("auth-audience", "auth-api", &["mobile", "infra"], "audience claim", "mobile and deployed policy use the same audience")],
        vec![
            HiddenCheck::FileContains { repo: "auth-api".into(), path: "auth.py".into(), needle: "audience == 'mobile'".into() },
            HiddenCheck::FileContains { repo: "mobile".into(), path: "request.py".into(), needle: "'audience': 'mobile'".into() },
            HiddenCheck::FileEquals { repo: "infra".into(), path: "policy.txt".into(), expected: "audience=mobile\n".into() },
        ],
        standard_capabilities(true),
        true,
        true,
        FaultInjection::None,
    )
}

fn dependency_chain() -> Scenario {
    let mut capabilities = standard_capabilities(false);
    capabilities.insert(Capability::TriggerDiscipline);
    scenario(
        "sequential-dependency-chain",
        "trigger_discipline",
        "Avoid harmful delegation for a tightly coupled change",
        "Rename the core parser function and update its only consumer. The consumer contract cannot be determined until the core edit is complete; keep this as one sequential workstream.",
        vec![
            repo("core", &[('p', "parser.py", "def parse_old(value):\n    return value.strip()\n")], &[('p', "parser.py", "def parse(value):\n    return value.strip()\n")], false),
            repo("app", &[('p', "app.py", "from parser import parse_old\ndef run(value):\n    return parse_old(value)\n")], &[('p', "app.py", "from parser import parse\ndef run(value):\n    return parse(value)\n")], false),
        ],
        vec![edge("parser-api", "core", &["app"], "Python function", "consumer changes only after the parser signature is fixed")],
        vec![
            HiddenCheck::FileContains { repo: "core".into(), path: "parser.py".into(), needle: "def parse(".into() },
            HiddenCheck::FileContains { repo: "app".into(), path: "app.py".into(), needle: "from parser import parse\n".into() },
        ],
        capabilities,
        false,
        false,
        FaultInjection::None,
    )
}

fn baseline_drift() -> Scenario {
    let mut capabilities = standard_capabilities(true);
    capabilities.insert(Capability::TargetedRecovery);
    scenario(
        "baseline-drift-replan",
        "failure_recovery",
        "Reject a stale patch and replan only its repository",
        "Update the shared feature flag in the library and service. The service baseline changes after planning; reject its stale artifact and regenerate only that package.",
        vec![
            repo("library", &[('p', "flags.py", "FEATURE = False\n")], &[('p', "flags.py", "FEATURE = True\n")], false),
            repo("service", &[('p', "config.py", "FEATURE = False\n")], &[('p', "config.py", "FEATURE = True\n")], false),
        ],
        vec![edge("feature-flag", "library", &["service"], "flag name", "both repositories expose FEATURE=True")],
        vec![
            HiddenCheck::FileContains { repo: "library".into(), path: "flags.py".into(), needle: "FEATURE = True".into() },
            HiddenCheck::FileContains { repo: "service".into(), path: "config.py".into(), needle: "FEATURE = True".into() },
        ],
        capabilities,
        true,
        false,
        FaultInjection::BaselineDrift,
    )
}

#[allow(clippy::too_many_arguments)]
fn scenario(
    id: &str,
    family: &str,
    title: &str,
    prompt: &str,
    repositories: Vec<RepositorySpec>,
    edges: Vec<ContractEdge>,
    hidden_checks: Vec<HiddenCheck>,
    required_capabilities: BTreeSet<Capability>,
    expected_delegate: bool,
    single_agent_trap: bool,
    fault: FaultInjection,
) -> Scenario {
    Scenario {
        id: id.into(),
        family: family.into(),
        title: title.into(),
        prompt: prompt.into(),
        repositories,
        edges,
        hidden_checks,
        required_capabilities,
        expected_delegate,
        single_agent_trap,
        fault,
    }
}

fn repo(
    id: &str,
    initial: &[(char, &str, &str)],
    solution: &[(char, &str, &str)],
    cloud_eligible: bool,
) -> RepositorySpec {
    let mut initial_files = Vec::new();
    let mut dirty_user_files = Vec::new();
    for (kind, path, content) in initial {
        let target = if *kind == 'd' {
            &mut dirty_user_files
        } else {
            &mut initial_files
        };
        target.push(FileFixture::new(path, *content));
    }
    let solution_files = solution
        .iter()
        .map(|(_, path, content)| FileFixture::new(path, *content))
        .collect::<Vec<_>>();
    let allowed_changed_paths = solution_files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    RepositorySpec {
        id: id.into(),
        initial_files,
        dirty_user_files,
        solution_files,
        allowed_changed_paths,
        public_checks: vec!["repository-specific tests".into()],
        cloud_eligible,
    }
}

fn edge(id: &str, producer: &str, consumers: &[&str], artifact: &str, rule: &str) -> ContractEdge {
    ContractEdge {
        id: id.into(),
        producer_repo: producer.into(),
        consumer_repos: consumers.iter().map(|value| (*value).into()).collect(),
        artifact: artifact.into(),
        compatibility_rule: rule.into(),
    }
}

fn python(name: &str, script: &str) -> HiddenCheck {
    HiddenCheck::Python {
        name: name.into(),
        script: script.into(),
    }
}

fn standard_capabilities(parallel: bool) -> BTreeSet<Capability> {
    if !parallel {
        return BTreeSet::from([
            Capability::TriggerDiscipline,
            Capability::NonTechnicalDefaultFlow,
            Capability::AuthoritativePlanningReceipt,
        ]);
    }
    let mut capabilities = BTreeSet::from([
        Capability::RepositoryGraph,
        Capability::AuthoritativePlanningReceipt,
        Capability::PinnedBaselines,
        Capability::ContractDecisionLedger,
        Capability::IsolatedWriterArtifacts,
        Capability::FreshIntegrationReplay,
        Capability::TriggerDiscipline,
        Capability::NonTechnicalDefaultFlow,
    ]);
    capabilities.insert(Capability::ParallelWriters);
    capabilities
}
