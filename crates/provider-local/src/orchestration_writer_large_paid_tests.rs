use super::*;

const LARGE_PATHS: [&str; 4] = [
    "src/catalog.py",
    "src/pricing.py",
    "src/eligibility.py",
    "src/notifications.py",
];

fn seed_large_parallel_project(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(root.join("src/__init__.py"), "").unwrap();
    std::fs::write(
        root.join("src/catalog.py"),
        r#"def normalize_sku(value):
    """Trim, uppercase, and replace each run of whitespace with one dash."""
    return ""


def index_products(rows):
    """Return products keyed by normalized SKU with trimmed name and active flag."""
    return {}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("src/pricing.py"),
        r#"def unit_price(cents, quantity):
    """Apply 10% discount at 10 units and 20% at 50, using integer cents."""
    return 0


def invoice_total(lines):
    """Sum unit_price(cents, quantity) * quantity for (cents, quantity) lines."""
    return 0
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("src/eligibility.py"),
        r#"def eligible_regions(user_regions, product_regions):
    """Return a sorted uppercase intersection, ignoring surrounding whitespace."""
    return []


def can_purchase(user, product):
    """Require active product, sufficient age, and an eligible home region."""
    return False
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("src/notifications.py"),
        r#"def render_receipt(name, items, total_cents):
    """Render: Hello NAME; ITEMS item(s); total $D.CC"""
    return ""


def chunk_recipients(addresses, size):
    """Deduplicate case-insensitively, preserve first spelling, then chunk."""
    return []
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("tests/test_commerce.py"),
        r#"import unittest

from src.catalog import index_products, normalize_sku
from src.eligibility import can_purchase, eligible_regions
from src.notifications import chunk_recipients, render_receipt
from src.pricing import invoice_total, unit_price


class CatalogTests(unittest.TestCase):
    def test_normalize_sku(self):
        self.assertEqual(normalize_sku("  ab   12 cd "), "AB-12-CD")

    def test_index_products(self):
        rows = [{"sku": " a 1 ", "name": " Alpha ", "active": 0},
                {"sku": "b-2", "name": "Beta"}]
        self.assertEqual(index_products(rows), {
            "A-1": {"name": "Alpha", "active": False},
            "B-2": {"name": "Beta", "active": True},
        })


class PricingTests(unittest.TestCase):
    def test_discount_boundaries(self):
        self.assertEqual(unit_price(999, 9), 999)
        self.assertEqual(unit_price(999, 10), 899)
        self.assertEqual(unit_price(999, 50), 799)

    def test_invoice_total(self):
        self.assertEqual(invoice_total([(1000, 2), (500, 10)]), 6500)


class EligibilityTests(unittest.TestCase):
    def test_region_intersection(self):
        self.assertEqual(eligible_regions([" us ", "CA", "gb"], ["GB", "US"]), ["GB", "US"])

    def test_purchase_contract(self):
        user = {"age": 21, "region": " us "}
        product = {"active": True, "min_age": 18, "regions": ["CA", "US"]}
        self.assertTrue(can_purchase(user, product))
        self.assertFalse(can_purchase({**user, "age": 17}, product))
        self.assertFalse(can_purchase(user, {**product, "active": False}))


class NotificationTests(unittest.TestCase):
    def test_render_receipt(self):
        self.assertEqual(render_receipt("Ada", 2, 1234), "Hello Ada; 2 items; total $12.34")
        self.assertEqual(render_receipt("Lin", 1, 5), "Hello Lin; 1 item; total $0.05")

    def test_chunk_recipients(self):
        self.assertEqual(chunk_recipients(["A@x.com", "a@X.com", "b@x.com", "c@x.com"], 2),
                         [["A@x.com", "b@x.com"], ["c@x.com"]])
        with self.assertRaises(ValueError):
            chunk_recipients(["a@x.com"], 0)


if __name__ == "__main__":
    unittest.main()
"#,
    )
    .unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Clark Large Paid Eval"]);
    git(root, &["config", "user.email", "large-eval@invalid.local"]);
    git(root, &["add", "--all"]);
    git(
        root,
        &["commit", "--quiet", "-m", "large synthetic baseline"],
    );
    std::fs::write(root.join("notes.user"), "preserve this user note\n").unwrap();
}

fn large_objective() -> &'static str {
    "Implement all four independent commerce modules so the existing unittest contract passes. Do not change tests, add dependencies, or touch notes.user. Preserve every public function and run the existing suite before finishing."
}

fn verify_large(root: &Path) {
    let verification = Command::new("python3")
        .current_dir(root)
        .args(["-m", "unittest", "discover", "-s", "tests"])
        .output()
        .unwrap();
    assert!(
        verification.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&verification.stdout),
        String::from_utf8_lossy(&verification.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(root.join("notes.user")).unwrap(),
        "preserve this user note\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[ignore = "paid large parallel-writer evaluation; run only with explicit user authorization"]
async fn paid_large_four_writer_workstreams() {
    let (api_key, model, base_url) = paid_config();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("large-parallel-paid");
    seed_large_parallel_project(&root);
    let shared = Arc::new(SharedState {
        config: OrchestrationToolsConfig {
            policy: crate::orchestration::OrchestrationConfig {
                enabled: true,
                max_agents: 4,
                max_attempts: 1,
                token_budget: 320_000,
                ..Default::default()
            },
            base_url,
            api_key: Some(api_key),
            headers: HashMap::new(),
            root_model: model.clone(),
            reasoning_effort: Some("low".into()),
        },
        pending: Mutex::new(HashMap::new()),
    });
    let ctx = ToolCtx {
        sandbox: Arc::new(crate::sandbox::Sandbox::new(&root).unwrap()),
        executor: Arc::new(LocalExecutor),
        reads: Arc::new(Mutex::new(ReadTracker::default())),
        cancel: tokio_util::sync::CancellationToken::new(),
        background: Arc::new(BackgroundTasks::default()),
        session: Arc::new(tokio::sync::Mutex::new(SessionState::default())),
        progress: None,
        agent_progress: None,
        call_progress: None,
    };
    let workstreams = [
        (
            "catalog-writer",
            "Implement src/catalog.py to its docstrings and the existing CatalogTests.",
            LARGE_PATHS[0],
        ),
        (
            "pricing-writer",
            "Implement src/pricing.py to its docstrings and the existing PricingTests.",
            LARGE_PATHS[1],
        ),
        (
            "eligibility-writer",
            "Implement src/eligibility.py to its docstrings and the existing EligibilityTests.",
            LARGE_PATHS[2],
        ),
        (
            "notifications-writer",
            "Implement src/notifications.py to its docstrings and the existing NotificationTests.",
            LARGE_PATHS[3],
        ),
    ]
    .into_iter()
    .map(|(id, objective, path)| WorkstreamArgs {
        id: id.into(),
        objective: objective.into(),
        paths: BTreeSet::from([path.into()]),
        dependencies: BTreeSet::new(),
    })
    .collect();
    let started = Instant::now();
    let outcome = run_workstreams(
        &shared,
        DelegateArgs {
            objective: large_objective().into(),
            workstreams,
            resources: vec![resources::ResourceArgs {
                id: "commerce-environment".into(),
                command: "sleep 1; printf COMMERCE_ENV_READY; sleep 30".into(),
                output_contains: Some("COMMERCE_ENV_READY".into()),
                workdir: None,
                timeout_ms: 5_000,
            }],
            integration_checks: vec![CheckArgs {
                id: "commerce-contract".into(),
                argv: vec![
                    "python3".into(),
                    "-m".into(),
                    "unittest".into(),
                    "discover".into(),
                    "-s".into(),
                    "tests".into(),
                ],
                timeout_ms: 30_000,
            }],
            independent_review: false,
        },
        &ctx,
    )
    .await
    .unwrap();
    assert!(!outcome.is_error, "{}", outcome.content);
    let body: Value = serde_json::from_str(&outcome.content).unwrap();
    let run_id = body["run_id"].as_str().unwrap();
    let pending = shared.pending.lock().unwrap().get(run_id).cloned().unwrap();
    let application = pending
        .selection
        .apply_verified_packages(
            ctx.executor.as_ref(),
            &pending.plan,
            &pending.result.change_packages,
            &pending.scratch_root,
        )
        .await
        .unwrap();
    verify_large(&root);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "kind": "large_parallel_four_writer",
            "model": model,
            "passed": true,
            "wall_ms": started.elapsed().as_millis(),
            "task_receipts": pending.result.tasks,
            "budget": pending.result.budget,
            "resources": pending.resources,
            "application": application
        }))
        .unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "paid large single-agent control; run only with explicit user authorization"]
async fn paid_large_single_agent_control() {
    let (api_key, model, base_url) = paid_config();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("large-single-paid");
    seed_large_parallel_project(&root);
    let mut provider = LocalAgentProvider::new();
    provider
        .connect(ProviderConfig {
            cwd: Some(root.to_string_lossy().into_owned()),
            auth_token: Some(api_key),
            extra: json!({
                "base_url": base_url,
                "model": model,
                "reasoning_effort": "low",
                "temperature": 0.0,
                "max_iterations": 128,
                "permissions": {
                    "write_file": "allow",
                    "edit_file": "allow",
                    "apply_patch": "allow",
                    "bash": "allow",
                    "bash_input": "allow",
                    "bash_kill": "allow"
                },
                "orchestration": false,
                "research": false,
                "memories": false,
                "project_knowledge": false,
                "browser_enabled": false,
                "mcp_servers": []
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let session = provider
        .new_session(SessionOptions {
            cwd: Some(root.to_string_lossy().into_owned()),
            mode: Some("auto".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let started = Instant::now();
    let mut stream = provider
        .prompt(&session.id, PromptInput::text(large_objective()))
        .await
        .unwrap();
    let mut usage = RunUsage::default();
    let mut status = None;
    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::RunFinished { outcome, .. } => {
                usage = outcome.usage.unwrap_or_default();
                status = Some(outcome.status);
            }
            AgentEvent::PermissionRequest { request } => {
                panic!("unexpected permission request: {}", request.title)
            }
            _ => {}
        }
    }
    assert_eq!(status, Some(RunStatus::Done));
    verify_large(&root);
    let changed = Command::new("git")
        .current_dir(&root)
        .args(["diff", "--name-only"])
        .output()
        .unwrap();
    let changed = String::from_utf8(changed.stdout).unwrap();
    assert_eq!(
        changed.lines().collect::<BTreeSet<_>>(),
        LARGE_PATHS.into_iter().collect()
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "kind": "large_single_control",
            "model": model,
            "passed": true,
            "wall_ms": started.elapsed().as_millis(),
            "usage": usage,
            "changed_paths": changed.lines().collect::<Vec<_>>()
        }))
        .unwrap()
    );
}
