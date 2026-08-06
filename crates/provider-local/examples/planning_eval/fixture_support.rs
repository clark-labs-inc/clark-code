#[cfg(test)]
use crate::model::ReferenceFn;
use crate::model::Verification;
use std::path::Path;
use std::process::Command;

pub(super) fn write(root: &Path, path: &str, text: &str) -> Result<(), String> {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().expect("fixture path has parent"))
        .map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}

pub(super) fn seed_scaffold(root: &Path, name: &str) -> Result<(), String> {
    let files = [
        ("package.json", format!("{{\"name\":\"{name}\",\"private\":true,\"type\":\"module\",\"scripts\":{{\"test\":\"node --test tests/*.test.mjs\"}}}}\n")),
        ("README.md", format!("# {name}\n\nMulti-component production simulation. Run `npm test` for visible regressions.\n")),
        ("CODEOWNERS", "/repos/api/ @api\n/repos/worker/ @runtime\n/repos/infra/ @platform\n".into()),
        ("docs/oncall.md", "# On-call\n\nEvery rollout needs a rollback owner and a saturation alert.\n".into()),
        ("docs/security.md", "# Security\n\nNever persist credentials in events or exported artifacts.\n".into()),
        ("config/environments.json", "{\"environments\":[\"dev\",\"staging\",\"production\"]}\n".into()),
        ("config/feature-flags.json", "{\"flags\":{}}\n".into()),
        ("scripts/check.mjs", "console.log('fixture checks available through npm test');\n".into()),
        ("repos/shared/src/logger.mjs", "export const log = (event, fields = {}) => ({ event, fields });\n".into()),
        ("repos/shared/src/clock.mjs", "export const now = () => new Date().toISOString();\n".into()),
        ("repos/shared/test/noop.test.mjs", "import test from 'node:test'; import assert from 'node:assert/strict'; test('scaffold',()=>assert.equal(1,1));\n".into()),
        ("docs/future-ideas.md", "# Uncommitted ideas\n\nBroker, theme, and dashboard ideas here are not requirements.\n".into()),
    ];
    for (path, body) in files {
        write(root, path, &body)?;
    }

    // Real projects contain many nearby but irrelevant components. These are
    // deliberately coherent enough to be plausible search results rather than
    // numbered filler files.
    for component in [
        "auth",
        "catalog",
        "search",
        "reporting",
        "scheduler",
        "notifications-admin",
        "entitlements",
        "observability",
        "support-tools",
        "data-retention",
        "release-control",
        "tenant-directory",
    ] {
        write(
            root,
            &format!("repos/{component}/src/index.mjs"),
            &format!(
                "export const component = '{component}';\nexport const health = () => ({{component,status:'ok'}});\n"
            ),
        )?;
        write(
            root,
            &format!("repos/{component}/config/service.json"),
            &format!(
                "{{\"name\":\"{component}\",\"tier\":\"supporting\",\"owner\":\"platform\"}}\n"
            ),
        )?;
        write(
            root,
            &format!("repos/{component}/README.md"),
            &format!(
                "# {component}\n\nSupporting service outside this migration's ownership boundary.\n"
            ),
        )?;
    }
    for environment in ["dev", "staging", "production"] {
        for surface in ["services", "alerts", "budgets", "access", "rollback"] {
            write(
                root,
                &format!("deploy/{environment}/{surface}.json"),
                &format!(
                    "{{\"environment\":\"{environment}\",\"surface\":\"{surface}\",\"managed\":true}}\n"
                ),
            )?;
        }
    }
    Ok(())
}

fn node_check(root: &Path, body: &str) -> (bool, String) {
    let prelude = "import assert from 'node:assert/strict'; import {pathToFileURL} from 'node:url'; import {join} from 'node:path'; const root=process.env.EVAL_ROOT; const load=(p)=>import(pathToFileURL(join(root,p)).href);";
    let output = Command::new("node")
        .args(["--input-type=module", "-e", &format!("{prelude}{body}")])
        .env("EVAL_ROOT", root)
        .output();
    match output {
        Ok(output) if output.status.success() => (true, "behavioral assertion passed".into()),
        Ok(output) => (
            false,
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(500)
                .collect(),
        ),
        Err(error) => (false, format!("node unavailable: {error}")),
    }
}

pub(super) fn push_node(result: &mut Verification, root: &Path, id: &str, script: &str) {
    let (passed, detail) = node_check(root, script);
    result.push(id, passed, detail);
}

pub(super) fn seed_repository_history(root: &Path, subject: &str) -> Result<(), String> {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Clark Benchmark"],
        vec!["config", "user.email", "benchmark@example.invalid"],
        vec!["add", "."],
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .map_err(|error| format!("git fixture setup failed: {error}"))?;
        if !status.success() {
            return Err("git fixture setup returned a non-zero status".into());
        }
    }
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["commit", "-q", "-m", subject])
        .env("GIT_AUTHOR_DATE", "2026-06-01T12:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-06-01T12:00:00Z")
        .status()
        .map_err(|error| format!("git fixture commit failed: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "git fixture commit returned a non-zero status".into())
}

/// Build a behaviorally equivalent but structurally different reference:
/// changed JavaScript entrypoints become facade modules over sibling
/// implementation modules. This prevents fixture eligibility from depending
/// on one exact source layout.
#[cfg(test)]
pub(super) fn apply_alternate_module_layout(
    root: &Path,
    reference_apply: ReferenceFn,
) -> Result<Vec<String>, String> {
    reference_apply(root)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--name-only", "HEAD"])
        .output()
        .map_err(|error| format!("alternate reference inventory failed: {error}"))?;
    if !output.status.success() {
        return Err("alternate reference inventory returned a non-zero status".into());
    }
    let mut facades = Vec::new();
    for relative in String::from_utf8_lossy(&output.stdout).lines() {
        let path = Path::new(relative);
        if path.extension().and_then(|value| value.to_str()) != Some("mjs") {
            continue;
        }
        let source = std::fs::read_to_string(root.join(path)).map_err(|error| error.to_string())?;
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("invalid JavaScript fixture path: {relative}"))?;
        let implementation_name = format!("{stem}.alternate.mjs");
        let implementation_path = path.with_file_name(&implementation_name);
        write(
            root,
            implementation_path
                .to_str()
                .ok_or_else(|| format!("invalid alternate fixture path: {relative}"))?,
            &source,
        )?;
        write(
            root,
            relative,
            &format!("export * from './{implementation_name}';\n"),
        )?;
        facades.push(relative.to_string());
    }
    if facades.is_empty() {
        return Err("alternate reference created no JavaScript facades".into());
    }
    Ok(facades)
}
