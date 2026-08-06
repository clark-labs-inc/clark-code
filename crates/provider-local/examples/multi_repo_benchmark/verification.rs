use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::model::{CandidateResult, CheckResult, HiddenCheck, Scenario};
use super::workspace::{
    apply_patch, clone_repository, result_tree_sha256, sha256, DynError, SeededWorkspace,
};

pub struct ReplayGrade {
    pub checks: Vec<CheckResult>,
    pub correctness: f64,
    pub packages_valid: bool,
}

pub fn replay_packages(
    scenario: &Scenario,
    workspace: &SeededWorkspace,
    run_root: &Path,
    result: &CandidateResult,
) -> Result<ReplayGrade, DynError> {
    let replay_root = run_root.join("fresh-replay");
    fs::create_dir_all(replay_root.join("repos"))?;
    for repo in &scenario.repositories {
        clone_repository(
            &workspace.repositories[&repo.id].root,
            &replay_root.join("repos").join(&repo.id),
        )?;
    }

    let mut checks = Vec::new();
    let mut seen_repositories = BTreeSet::new();
    let mut packages_valid = true;
    for package in &result.change_packages {
        let Some(seed) = workspace.repositories.get(&package.repo_id) else {
            push(
                &mut checks,
                "package-repository",
                false,
                "package names an unknown repository",
            );
            packages_valid = false;
            continue;
        };
        let Some(spec) = scenario
            .repositories
            .iter()
            .find(|repo| repo.id == package.repo_id)
        else {
            packages_valid = false;
            continue;
        };
        let patch = fs::read(PathBuf::from(&package.patch_path)).unwrap_or_default();
        let valid = seen_repositories.insert(package.repo_id.clone())
            && package.base_sha == seed.baseline_sha
            && !patch.is_empty()
            && sha256(&patch) == package.patch_sha256
            && package.changed_paths == spec.allowed_changed_paths;
        push(
            &mut checks,
            &format!("package::{}", package.repo_id),
            valid,
            "unique package must pin the baseline, exact paths, and patch digest",
        );
        if !valid {
            packages_valid = false;
            continue;
        }
        let replay_repo = replay_root.join("repos").join(&package.repo_id);
        if apply_patch(&replay_repo, &patch).is_err() {
            packages_valid = false;
            push(
                &mut checks,
                &format!("package-apply::{}", package.repo_id),
                false,
                "patch did not apply to a fresh clone of its declared baseline",
            );
            continue;
        }
        let result_tree_matches = result_tree_sha256(
            &replay_repo,
            &package.base_sha,
            &package.patch_sha256,
            &package.changed_paths,
        )? == package.result_tree_sha256;
        push(
            &mut checks,
            &format!("package-tree::{}", package.repo_id),
            result_tree_matches,
            "declared result tree must equal the independently replayed tree",
        );
        packages_valid &= result_tree_matches;
    }

    let hidden = run_hidden_checks(scenario, &replay_root)?;
    let behavior = pass_fraction(&hidden);
    checks.extend(hidden.into_iter().map(|mut check| {
        check.id = format!("replay::{}", check.id);
        check
    }));
    Ok(ReplayGrade {
        checks,
        correctness: if packages_valid { behavior } else { 0.0 },
        packages_valid,
    })
}

pub fn run_hidden_checks(
    scenario: &Scenario,
    workspace_root: &Path,
) -> Result<Vec<CheckResult>, DynError> {
    scenario
        .hidden_checks
        .iter()
        .enumerate()
        .map(|(index, check)| {
            let (passed, detail) = match check {
                HiddenCheck::FileContains { repo, path, needle } => {
                    let content =
                        fs::read_to_string(workspace_root.join("repos").join(repo).join(path))
                            .unwrap_or_default();
                    (
                        content.contains(needle),
                        format!("{repo}/{path} contains required contract marker"),
                    )
                }
                HiddenCheck::FileEquals {
                    repo,
                    path,
                    expected,
                } => {
                    let content =
                        fs::read_to_string(workspace_root.join("repos").join(repo).join(path))
                            .unwrap_or_default();
                    (
                        content == *expected,
                        format!("{repo}/{path} exactly matches the required artifact"),
                    )
                }
                HiddenCheck::Python { name, script } => {
                    let output = Command::new("python3")
                        .args(["-c", script])
                        .arg(workspace_root)
                        .output()?;
                    (
                        output.status.success(),
                        format!("{name}: {}", String::from_utf8_lossy(&output.stderr).trim()),
                    )
                }
            };
            Ok(CheckResult {
                id: format!("hidden-{index}"),
                passed,
                detail,
            })
        })
        .collect()
}

pub fn pass_fraction(checks: &[CheckResult]) -> f64 {
    if checks.is_empty() {
        1.0
    } else {
        checks.iter().filter(|check| check.passed).count() as f64 / checks.len() as f64
    }
}

fn push(checks: &mut Vec<CheckResult>, id: &str, passed: bool, detail: &str) {
    checks.push(CheckResult {
        id: id.into(),
        passed,
        detail: detail.into(),
    });
}
