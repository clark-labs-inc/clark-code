use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

mod fixtures;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileFixture {
    pub path: String,
    pub content: String,
}

impl FileFixture {
    pub fn new(path: &str, content: impl Into<String>) -> Self {
        Self {
            path: path.to_string(),
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReaderTask {
    pub id: String,
    pub scope: BTreeSet<String>,
    pub instruction: String,
    pub expected_finding: String,
    pub cheap_model_eligible: bool,
    pub cloud_eligible: bool,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultInjection {
    BudgetExhaustion,
    CrashFirstAttempt,
    DuplicateReport,
    FalseHandoff,
    FlakyVerification,
    MissingHandoff,
    PermissionEscalation,
    RestartAfterReaders,
    ReviewerSeededBug,
    StaleConcurrentChange,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HiddenCheck {
    Absent { path: String },
    Contains { path: String, needle: String },
    Equals { path: String, expected: String },
    CommandSucceeds { program: String, args: Vec<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    pub family: String,
    pub variant: u32,
    pub title: String,
    pub prompt: String,
    pub git_repository: bool,
    pub expected_delegate: bool,
    pub cloud_agent_eligible: bool,
    pub initial_files: Vec<FileFixture>,
    pub dirty_user_files: Vec<FileFixture>,
    pub solution: Vec<FileFixture>,
    pub allowed_changed_paths: BTreeSet<String>,
    pub reader_tasks: Vec<ReaderTask>,
    pub hidden_checks: Vec<HiddenCheck>,
    pub faults: Vec<FaultInjection>,
}

#[derive(Clone, Debug)]
pub struct SeededRepository {
    pub root: PathBuf,
    pub git_baseline: Option<String>,
    pub before_agent: TreeSnapshot,
    pub user_dirty_paths: BTreeSet<String>,
}

pub type TreeSnapshot = BTreeMap<String, Vec<u8>>;

#[derive(Clone, Debug)]
pub struct GradeResult {
    pub checks: Vec<(String, bool, String)>,
    pub changed_paths: BTreeSet<String>,
    pub unexpected_changed_paths: BTreeSet<String>,
    pub lost_user_changes: BTreeSet<String>,
    pub correctness: f64,
    pub changed_path_precision: f64,
}

pub fn catalog() -> Vec<Scenario> {
    fixtures::catalog()
}

pub fn trigger_policy(scenario: &Scenario) -> bool {
    let prompt = scenario.prompt.to_ascii_lowercase();
    let coordination_terms = [
        "across",
        "compatible",
        "consistently",
        "repository-wide",
        "upstream",
    ];
    let mut score = 0;
    if scenario.allowed_changed_paths.len() >= 2 {
        score += 2;
    }
    if scenario.initial_files.len() >= 3 {
        score += 2;
    }
    if !scenario.dirty_user_files.is_empty() {
        score += 2;
    }
    if scenario.cloud_agent_eligible {
        score += 3;
    }
    if coordination_terms.iter().any(|term| prompt.contains(term)) {
        score += 1;
    }
    score >= 2
}

#[cfg(test)]
pub fn find(id: &str) -> Option<Scenario> {
    catalog().into_iter().find(|scenario| scenario.id == id)
}

pub fn seed(root: &Path, scenario: &Scenario) -> Result<SeededRepository, String> {
    std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
    for file in &scenario.initial_files {
        write_fixture(root, file)?;
    }

    let git_baseline = if scenario.git_repository {
        run_git(root, &["init", "--quiet"])?;
        run_git(
            root,
            &["config", "user.name", "Clark orchestration benchmark"],
        )?;
        run_git(root, &["config", "user.email", "benchmark@clark.local"])?;
        run_git(root, &["add", "-A"])?;
        run_git(root, &["commit", "--quiet", "-m", "synthetic baseline"])?;
        Some(run_git(root, &["rev-parse", "HEAD"])?.trim().to_string())
    } else {
        None
    };

    for file in &scenario.dirty_user_files {
        write_fixture(root, file)?;
    }
    let before_agent = snapshot(root)?;
    let user_dirty_paths = scenario
        .dirty_user_files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    Ok(SeededRepository {
        root: root.to_path_buf(),
        git_baseline,
        before_agent,
        user_dirty_paths,
    })
}

#[cfg(test)]
pub fn apply_solution(root: &Path, scenario: &Scenario) -> Result<(), String> {
    for file in &scenario.solution {
        write_fixture(root, file)?;
    }
    Ok(())
}

pub fn snapshot(root: &Path) -> Result<TreeSnapshot, String> {
    let mut snapshot = TreeSnapshot::new();
    collect_files(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn collect_files(root: &Path, dir: &Path, out: &mut TreeSnapshot) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(
                relative,
                std::fs::read(&path).map_err(|error| error.to_string())?,
            );
        }
    }
    Ok(())
}

pub fn grade(scenario: &Scenario, seeded: &SeededRepository) -> Result<GradeResult, String> {
    let after = snapshot(&seeded.root)?;
    let all_paths: BTreeSet<String> = seeded
        .before_agent
        .keys()
        .chain(after.keys())
        .cloned()
        .collect();
    let changed_paths: BTreeSet<String> = all_paths
        .into_iter()
        .filter(|path| seeded.before_agent.get(path) != after.get(path))
        .collect();
    let unexpected_changed_paths = changed_paths
        .difference(&scenario.allowed_changed_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    let lost_user_changes = seeded
        .user_dirty_paths
        .iter()
        .filter(|path| seeded.before_agent.get(*path) != after.get(*path))
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut checks = Vec::new();
    for check in &scenario.hidden_checks {
        match check {
            HiddenCheck::Absent { path } => {
                let passed = !after.contains_key(path);
                checks.push((
                    format!("file_absent:{path}"),
                    passed,
                    format!("{path} should not exist"),
                ));
            }
            HiddenCheck::Contains { path, needle } => {
                let actual = after
                    .get(path)
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                let passed = actual.as_deref().is_some_and(|text| text.contains(needle));
                checks.push((
                    format!("file_contains:{path}"),
                    passed,
                    format!("{path} should contain {needle:?}"),
                ));
            }
            HiddenCheck::Equals { path, expected } => {
                let actual = after
                    .get(path)
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                let passed = actual.as_deref() == Some(expected.as_str());
                checks.push((
                    format!("file_equals:{path}"),
                    passed,
                    format!("{path} should exactly match the hidden expected content"),
                ));
            }
            HiddenCheck::CommandSucceeds { program, args } => {
                let output = std::process::Command::new(program)
                    .args(args)
                    .current_dir(&seeded.root)
                    .env("PYTHONDONTWRITEBYTECODE", "1")
                    .output();
                let (passed, detail) = match output {
                    Ok(output) => (
                        output.status.success(),
                        format!(
                            "exit={:?}; stdout={}; stderr={}",
                            output.status.code(),
                            String::from_utf8_lossy(&output.stdout).trim(),
                            String::from_utf8_lossy(&output.stderr).trim()
                        ),
                    ),
                    Err(error) => (false, format!("could not run {program}: {error}")),
                };
                checks.push((format!("command_succeeds:{program}"), passed, detail));
            }
        }
    }
    checks.push((
        "changed_paths_in_scope".into(),
        unexpected_changed_paths.is_empty(),
        format!("unexpected paths: {unexpected_changed_paths:?}"),
    ));
    checks.push((
        "user_changes_preserved".into(),
        lost_user_changes.is_empty(),
        format!("lost user paths: {lost_user_changes:?}"),
    ));

    let passed = checks.iter().filter(|(_, passed, _)| *passed).count();
    let correctness = passed as f64 / checks.len().max(1) as f64;
    let expected_agent_changes = scenario.allowed_changed_paths.len().max(1);
    let in_scope_changes = changed_paths
        .intersection(&scenario.allowed_changed_paths)
        .count();
    let changed_path_precision =
        in_scope_changes as f64 / changed_paths.len().max(expected_agent_changes).max(1) as f64;
    Ok(GradeResult {
        checks,
        changed_paths,
        unexpected_changed_paths,
        lost_user_changes,
        correctness,
        changed_path_precision,
    })
}

fn write_fixture(root: &Path, fixture: &FileFixture) -> Result<(), String> {
    let path = root.join(&fixture.path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, fixture.content.as_bytes()).map_err(|error| error.to_string())
}

fn run_git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_required_failure_and_trigger_families() {
        let scenarios = catalog();
        let families: BTreeSet<_> = scenarios.iter().map(|s| s.family.as_str()).collect();
        for required in [
            "trivial",
            "independent_modules",
            "hidden_contract",
            "false_parallelism",
            "overlapping_edits",
            "dirty_user_changes",
            "stale_reads",
            "decoys",
            "misleading_docs",
            "flaky_tests",
            "worker_crash",
            "false_handoff",
            "reviewer_bug",
            "permission_escalation",
            "budget_exhaustion",
            "context_truncation",
            "restart_resume",
            "remote_execution",
            "non_git",
            "substantial_multifile",
            "clark_cloud",
        ] {
            assert!(families.contains(required), "missing family {required}");
        }
        assert!(scenarios.len() >= 24);
        assert!(scenarios.iter().any(|s| !s.expected_delegate));
        assert!(scenarios.iter().any(|s| s.cloud_agent_eligible));
    }

    #[test]
    fn scripted_solution_passes_every_hidden_rubric() {
        for scenario in catalog() {
            let dir = tempfile::tempdir().unwrap();
            let seeded = seed(dir.path(), &scenario).unwrap();
            apply_solution(dir.path(), &scenario).unwrap();
            let grade = grade(&scenario, &seeded).unwrap();
            assert_eq!(
                grade.correctness, 1.0,
                "{}: {:?}",
                scenario.id, grade.checks
            );
            assert!(grade.unexpected_changed_paths.is_empty(), "{}", scenario.id);
            assert!(grade.lost_user_changes.is_empty(), "{}", scenario.id);
        }
    }

    #[test]
    fn trigger_policy_is_independent_from_the_expected_label() {
        assert!(!trigger_policy(&find("trivial-1").unwrap()));
        assert!(trigger_policy(&find("independent-modules-1").unwrap()));
        assert!(trigger_policy(&find("clark-cloud-1").unwrap()));
        assert!(!trigger_policy(&find("worker-crash-1").unwrap()));
        assert!(find("worker-crash-1").unwrap().expected_delegate);
    }
}
