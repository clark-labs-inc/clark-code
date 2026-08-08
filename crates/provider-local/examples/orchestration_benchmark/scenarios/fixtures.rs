use std::collections::BTreeSet;

use super::{FaultInjection, FileFixture, HiddenCheck, ReaderTask, Scenario};

fn paths(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn file(path: &str, content: impl Into<String>) -> FileFixture {
    FileFixture::new(path, content)
}

fn reader(
    id: &str,
    scope: &[&str],
    instruction: &str,
    finding: &str,
    cheap: bool,
    cloud: bool,
) -> ReaderTask {
    ReaderTask {
        id: id.to_string(),
        scope: paths(scope),
        instruction: instruction.to_string(),
        expected_finding: finding.to_string(),
        cheap_model_eligible: cheap,
        cloud_eligible: cloud,
        dependencies: vec![],
    }
}

#[allow(clippy::too_many_arguments)]
fn scenario(
    id: &str,
    family: &str,
    variant: u32,
    title: &str,
    prompt: &str,
    initial_files: Vec<FileFixture>,
    solution: Vec<FileFixture>,
    readers: Vec<ReaderTask>,
    delegate: bool,
) -> Scenario {
    let allowed_changed_paths = solution.iter().map(|f| f.path.clone()).collect();
    let hidden_checks = solution
        .iter()
        .map(|f| HiddenCheck::Equals {
            path: f.path.clone(),
            expected: f.content.clone(),
        })
        .collect();
    Scenario {
        id: id.into(),
        family: family.into(),
        variant,
        title: title.into(),
        prompt: prompt.into(),
        git_repository: true,
        expected_delegate: delegate,
        cloud_agent_eligible: false,
        initial_files,
        dirty_user_files: vec![],
        solution,
        allowed_changed_paths,
        reader_tasks: readers,
        hidden_checks,
        faults: vec![],
    }
}

fn python(code: impl Into<String>) -> HiddenCheck {
    HiddenCheck::CommandSucceeds {
        program: "python3".into(),
        args: vec!["-c".into(), code.into()],
    }
}

fn trivial(variant: u32) -> Scenario {
    let old = variant + 1;
    let new = old + 1;
    let mut scenario = scenario(
        &format!("trivial-{variant}"),
        "trivial",
        variant,
        "One constant change",
        &format!("Change RETRY_LIMIT from {old} to {new} in src/policy.py. Keep the diff minimal."),
        vec![file("src/policy.py", format!("RETRY_LIMIT = {old}\n"))],
        vec![file("src/policy.py", format!("RETRY_LIMIT = {new}\n"))],
        vec![],
        false,
    );
    scenario.hidden_checks = vec![HiddenCheck::Contains {
        path: "src/policy.py".into(),
        needle: format!("RETRY_LIMIT = {new}"),
    }];
    scenario
}

fn independent_modules(variant: u32) -> Scenario {
    let marker = format!("v{variant}");
    let mut scenario = scenario(
        &format!("independent-modules-{variant}"),
        "independent_modules",
        variant,
        "Independent API, UI, and documentation changes",
        "Add request IDs to the API response, render them in the UI, and document the field. Preserve each module's existing public names.",
        vec![
            file("api/service.py", "def response():\n    return {'ok': True}\n"),
            file("ui/view.py", "def render(data):\n    return str(data['ok'])\n"),
            file("docs/api.md", "# API\n\nReturns `ok`.\n"),
        ],
        vec![
            file("api/service.py", format!("def response():\n    return {{'ok': True, 'request_id': '{marker}'}}\n")),
            file("ui/view.py", "def render(data):\n    return f\"{data['ok']}:{data['request_id']}\"\n"),
            file("docs/api.md", "# API\n\nReturns `ok` and `request_id`.\n"),
        ],
        vec![
            reader("api", &["api"], "Trace the API response contract.", "response owns request_id", true, false),
            reader("ui", &["ui"], "Trace UI consumers of the response.", "UI must render request_id", true, false),
            reader("docs", &["docs"], "Find documentation that describes the response.", "docs/api.md is authoritative", true, false),
        ],
        true,
    );
    scenario.hidden_checks = vec![
        HiddenCheck::Contains {
            path: "api/service.py".into(),
            needle: "request_id".into(),
        },
        HiddenCheck::Contains {
            path: "ui/view.py".into(),
            needle: "request_id".into(),
        },
        HiddenCheck::Contains {
            path: "docs/api.md".into(),
            needle: "request_id".into(),
        },
        HiddenCheck::CommandSucceeds {
            program: "python3".into(),
            args: vec![
                "-c".into(),
                "from api.service import response; from ui.view import render; r=response(); assert r['ok'] is True; assert r['request_id']; assert str(r['request_id']) in render(r)".into(),
            ],
        },
    ];
    scenario
}

fn hidden_contract(variant: u32) -> Scenario {
    let version = variant + 1;
    let mut scenario = scenario(
        &format!("hidden-contract-{variant}"),
        "hidden_contract",
        variant,
        "Encoder and decoder share a hidden protocol version",
        &format!("Upgrade the packet protocol to version {version}. Encoder and decoder must remain compatible."),
        vec![
            file("protocol.md", "Every packet begins with `v1:`.\n"),
            file("src/encode.py", "def encode(value):\n    return 'v1:' + value\n"),
            file("src/decode.py", "def decode(value):\n    assert value.startswith('v1:')\n    return value[3:]\n"),
        ],
        vec![
            file("protocol.md", format!("Every packet begins with `v{version}:`.\n")),
            file("src/encode.py", format!("def encode(value):\n    return 'v{version}:' + value\n")),
            file("src/decode.py", format!("def decode(value):\n    assert value.startswith('v{version}:')\n    return value[3:]\n")),
        ],
        vec![
            reader("producer", &["src/encode.py", "protocol.md"], "Find the producer-side protocol contract.", "encoder prefix follows protocol.md", false, false),
            reader("consumer", &["src/decode.py", "protocol.md"], "Find the consumer-side protocol contract.", "decoder prefix must match encoder", false, false),
        ],
        true,
    );
    scenario.hidden_checks = vec![
        HiddenCheck::Contains {
            path: "protocol.md".into(),
            needle: format!("v{version}:"),
        },
        python(format!(
            "import sys; sys.path.insert(0,'src'); from encode import encode; from decode import decode; encoded=encode('payload'); assert encoded.startswith('v{version}:'); assert decode(encoded)=='payload'"
        )),
    ];
    scenario
}

fn false_parallelism() -> Scenario {
    let mut scenario = scenario(
        "false-parallelism-1",
        "false_parallelism",
        1,
        "Changes look separate but share a schema",
        "Rename the wire field from userId to user_id across the producer and consumer.",
        vec![
            file("schema.json", "{\"field\": \"userId\"}\n"),
            file("producer.py", "def row(v): return {'userId': v}\n"),
            file("consumer.py", "def read(v): return v['userId']\n"),
        ],
        vec![
            file("schema.json", "{\"field\": \"user_id\"}\n"),
            file("producer.py", "def row(v): return {'user_id': v}\n"),
            file("consumer.py", "def read(v): return v['user_id']\n"),
        ],
        vec![reader(
            "contract",
            &["schema.json", "producer.py", "consumer.py"],
            "Identify the shared rename boundary and ordering.",
            "all three files form one atomic contract",
            false,
            false,
        )],
        true,
    );
    scenario.reader_tasks[0].dependencies = vec!["schema-decision".into()];
    scenario.hidden_checks = vec![
        HiddenCheck::Contains {
            path: "schema.json".into(),
            needle: "user_id".into(),
        },
        python("from producer import row; from consumer import read; value=row(7); assert value=={'user_id':7}; assert read(value)==7"),
    ];
    scenario
}

fn overlapping_edits() -> Scenario {
    let mut scenario = scenario(
        "overlapping-edits-1",
        "overlapping_edits",
        1,
        "Two requested routes share one registration file",
        "Add /health and /ready routes, preserving deterministic route order.",
        vec![file("src/router.py", "ROUTES = ['/']\n")],
        vec![file(
            "src/router.py",
            "ROUTES = ['/', '/health', '/ready']\n",
        )],
        vec![
            reader(
                "health",
                &["src/router.py"],
                "Locate the health route seam.",
                "router.py owns route registration",
                true,
                false,
            ),
            reader(
                "ready",
                &["src/router.py"],
                "Locate the readiness route seam.",
                "router.py owns route registration",
                true,
                false,
            ),
        ],
        true,
    );
    scenario.hidden_checks = vec![python(
        "import sys; sys.path.insert(0,'src'); from router import ROUTES; assert ROUTES==['/','/health','/ready']",
    )];
    scenario
}

fn dirty_user_changes() -> Scenario {
    let mut scenario = scenario(
        "dirty-user-changes-1",
        "dirty_user_changes",
        1,
        "Preserve an unrelated dirty user note",
        "Fix normalize() without touching notes/user-draft.md.",
        vec![
            file("src/text.py", "def normalize(v): return v\n"),
            file("notes/user-draft.md", "original draft\n"),
        ],
        vec![file(
            "src/text.py",
            "def normalize(v): return v.strip().lower()\n",
        )],
        vec![reader(
            "implementation",
            &["src/text.py"],
            "Find normalize behavior.",
            "normalize should strip and lowercase",
            true,
            false,
        )],
        true,
    );
    scenario.dirty_user_files = vec![file("notes/user-draft.md", "user's unsaved draft\n")];
    scenario.hidden_checks = vec![python(
        "import sys; sys.path.insert(0,'src'); from text import normalize; assert normalize('  MiXeD  ')=='mixed'",
    )];
    scenario
}

fn decoys(variant: u32) -> Scenario {
    let mut initial = vec![file("src/policy.py", "LIMIT = 2\n")];
    for index in 0..80 {
        initial.push(file(
            &format!("vendor/decoy-{variant}-{index}.py"),
            "LIMIT = 999\n",
        ));
    }
    let mut scenario = scenario(
        &format!("decoys-{variant}"),
        "decoys",
        variant,
        "Generated and vendor decoys",
        "Change the production LIMIT to 4. Do not modify vendor files.",
        initial,
        vec![file("src/policy.py", "LIMIT = 4\n")],
        vec![reader(
            "authority",
            &["src", "vendor"],
            "Locate the authoritative policy and identify decoys.",
            "src/policy.py is authoritative",
            true,
            false,
        )],
        true,
    );
    scenario.hidden_checks = vec![HiddenCheck::Contains {
        path: "src/policy.py".into(),
        needle: "LIMIT = 4".into(),
    }];
    scenario
}

fn generic_fault(id: &str, family: &str, fault: FaultInjection, delegate: bool) -> Scenario {
    let mut scenario = scenario(
        id,
        family,
        1,
        family,
        "Change feature_enabled to true and verify the repository outcome.",
        vec![file("src/feature.py", "feature_enabled = False\n")],
        vec![file("src/feature.py", "feature_enabled = True\n")],
        vec![reader(
            "feature",
            &["src/feature.py"],
            "Inspect the feature flag and required change.",
            "feature_enabled must become true",
            true,
            false,
        )],
        delegate,
    );
    scenario.faults.push(fault);
    scenario.hidden_checks = vec![python(
        "import sys; sys.path.insert(0,'src'); import feature; assert feature.feature_enabled is True",
    )];
    scenario
}

fn misleading_docs() -> Scenario {
    let mut scenario = scenario(
        "misleading-docs-1",
        "misleading_docs",
        1,
        "README contradicts executable contract",
        "Fix timeout_seconds to match the tested production contract.",
        vec![
            file("README.md", "Timeout is 10 seconds.\n"),
            file("src/config.py", "timeout_seconds = 10\n"),
            file("tests/contract.txt", "production_timeout_seconds=30\n"),
        ],
        vec![file("src/config.py", "timeout_seconds = 30\n")],
        vec![reader(
            "authority",
            &["README.md", "src", "tests"],
            "Determine which timeout contract is authoritative.",
            "tests/contract.txt overrides stale README",
            false,
            false,
        )],
        true,
    );
    scenario.hidden_checks = vec![HiddenCheck::Contains {
        path: "src/config.py".into(),
        needle: "timeout_seconds = 30".into(),
    }];
    scenario
}

fn context_truncation() -> Scenario {
    let mut initial = vec![
        file("src/target.py", "VALUE = 'old'\n"),
        file(
            "docs/contract.md",
            "The required value is `kept-near-tail`.\n",
        ),
    ];
    for index in 0..120 {
        initial.push(file(
            &format!("archive/noise-{index:03}.txt"),
            format!("irrelevant historical record {index} {}\n", "x".repeat(200)),
        ));
    }
    let mut scenario = scenario(
        "context-truncation-1",
        "context_truncation",
        1,
        "Large repository with one relevant tail contract",
        "Update src/target.py to the value required by docs/contract.md; ignore archive noise.",
        initial,
        vec![file("src/target.py", "VALUE = 'kept-near-tail'\n")],
        vec![reader(
            "target",
            &["src/target.py", "docs/contract.md"],
            "Find the relevant value without flooding context.",
            "kept-near-tail",
            true,
            false,
        )],
        true,
    );
    scenario.hidden_checks = vec![HiddenCheck::Contains {
        path: "src/target.py".into(),
        needle: "kept-near-tail".into(),
    }];
    scenario
}

fn remote_execution() -> Scenario {
    let mut scenario = scenario(
        "remote-execution-1",
        "remote_execution",
        1,
        "Executor parity marker",
        "Update the deployment marker without assuming a local absolute path.",
        vec![file("deploy/marker.txt", "remote-old\n")],
        vec![file("deploy/marker.txt", "remote-ready\n")],
        vec![reader(
            "remote",
            &["deploy"],
            "Inspect the project-relative deployment marker.",
            "all paths must remain project relative",
            true,
            false,
        )],
        true,
    );
    scenario.hidden_checks = vec![HiddenCheck::Contains {
        path: "deploy/marker.txt".into(),
        needle: "remote-ready".into(),
    }];
    scenario
}

fn non_git() -> Scenario {
    let mut scenario = scenario(
        "non-git-1",
        "non_git",
        1,
        "Non-Git workspace fallback",
        "Update the standalone configuration and report evidence without a Git checkpoint.",
        vec![file("config.txt", "mode=old\n")],
        vec![file("config.txt", "mode=new\n")],
        vec![],
        false,
    );
    scenario.git_repository = false;
    scenario.hidden_checks = vec![HiddenCheck::Contains {
        path: "config.txt".into(),
        needle: "mode=new".into(),
    }];
    scenario
}

fn substantial(variant: u32) -> Scenario {
    let version = variant + 2;
    let mut scenario = scenario(
        &format!("substantial-multifile-{variant}"),
        "substantial_multifile",
        variant,
        "Repository-wide API version upgrade",
        "Upgrade the service API version consistently across server, client, migration, tests, and documentation.",
        vec![
            file("server/api.py", "API_VERSION = 1\n"),
            file("client/sdk.py", "SUPPORTED_VERSION = 1\n"),
            file("db/migration.py", "LATEST_SCHEMA = 1\n"),
            file("tests/contract.py", "EXPECTED_VERSION = 1\n"),
            file("docs/version.md", "Current version: 1\n"),
        ],
        vec![
            file("server/api.py", format!("API_VERSION = {version}\n")),
            file("client/sdk.py", format!("SUPPORTED_VERSION = {version}\n")),
            file("db/migration.py", format!("LATEST_SCHEMA = {version}\n")),
            file("tests/contract.py", format!("EXPECTED_VERSION = {version}\n")),
            file("docs/version.md", format!("Current version: {version}\n")),
        ],
        vec![
            reader("server", &["server", "client"], "Trace server/client compatibility.", "versions must match", false, false),
            reader("storage", &["db"], "Trace storage migration requirements.", "schema version follows API", true, false),
            reader("verification", &["tests", "docs"], "Locate verification and documentation updates.", "tests and docs must match", true, false),
        ],
        true,
    );
    scenario.hidden_checks = vec![
        HiddenCheck::Contains {
            path: "server/api.py".into(),
            needle: format!("API_VERSION = {version}"),
        },
        HiddenCheck::Contains {
            path: "client/sdk.py".into(),
            needle: format!("SUPPORTED_VERSION = {version}"),
        },
        HiddenCheck::Contains {
            path: "db/migration.py".into(),
            needle: format!("LATEST_SCHEMA = {version}"),
        },
        HiddenCheck::Contains {
            path: "tests/contract.py".into(),
            needle: format!("EXPECTED_VERSION = {version}"),
        },
        HiddenCheck::Contains {
            path: "docs/version.md".into(),
            needle: version.to_string(),
        },
    ];
    scenario
}

fn product_cloud() -> Scenario {
    let mut scenario = scenario(
        "brokered-cloud-1",
        "product_cloud",
        1,
        "Cloud-assisted compatibility research",
        "Use the provided upstream compatibility notes to update the adapter without changing its public function name.",
        vec![
            file("upstream/compatibility.md", "Protocol 7 requires header X-Agent-Mode: safe.\n"),
            file("src/adapter.py", "def headers(): return {}\n"),
        ],
        vec![file("src/adapter.py", "def headers(): return {'X-Agent-Mode': 'safe'}\n")],
        vec![reader("cloud-research", &["upstream/compatibility.md", "src/adapter.py"], "Analyze the compatibility material and report the required adapter change.", "X-Agent-Mode must be safe", true, true)],
        true,
    );
    scenario.cloud_agent_eligible = true;
    scenario.hidden_checks = vec![python(
        "import sys; sys.path.insert(0,'src'); import adapter; assert adapter.headers().get('X-Agent-Mode')=='safe'",
    )];
    scenario
}

pub fn catalog() -> Vec<Scenario> {
    vec![
        trivial(1),
        trivial(2),
        independent_modules(1),
        independent_modules(2),
        hidden_contract(1),
        hidden_contract(2),
        false_parallelism(),
        overlapping_edits(),
        dirty_user_changes(),
        generic_fault(
            "stale-reads-1",
            "stale_reads",
            FaultInjection::StaleConcurrentChange,
            true,
        ),
        decoys(1),
        decoys(2),
        misleading_docs(),
        generic_fault(
            "flaky-tests-1",
            "flaky_tests",
            FaultInjection::FlakyVerification,
            true,
        ),
        generic_fault(
            "worker-crash-1",
            "worker_crash",
            FaultInjection::CrashFirstAttempt,
            true,
        ),
        generic_fault(
            "missing-handoff-1",
            "false_handoff",
            FaultInjection::MissingHandoff,
            true,
        ),
        generic_fault(
            "false-handoff-1",
            "false_handoff",
            FaultInjection::FalseHandoff,
            true,
        ),
        generic_fault(
            "reviewer-bug-1",
            "reviewer_bug",
            FaultInjection::ReviewerSeededBug,
            true,
        ),
        generic_fault(
            "permission-escalation-1",
            "permission_escalation",
            FaultInjection::PermissionEscalation,
            true,
        ),
        generic_fault(
            "budget-exhaustion-1",
            "budget_exhaustion",
            FaultInjection::BudgetExhaustion,
            true,
        ),
        context_truncation(),
        generic_fault(
            "restart-resume-1",
            "restart_resume",
            FaultInjection::RestartAfterReaders,
            true,
        ),
        generic_fault(
            "duplicate-report-1",
            "false_handoff",
            FaultInjection::DuplicateReport,
            true,
        ),
        remote_execution(),
        non_git(),
        substantial(1),
        substantial(2),
        product_cloud(),
    ]
}
