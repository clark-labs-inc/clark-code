use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use super::model::{
    CandidateControl, CandidateRequest, FileFixture, LaneSpec, PublicTaskManifest,
    RepositoryManifest, Scenario,
};

pub type DynError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug)]
pub struct SeededRepo {
    pub root: PathBuf,
    pub baseline_sha: String,
    pub before: BTreeMap<String, Vec<u8>>,
    pub dirty_before: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug)]
pub struct SeededWorkspace {
    pub root: PathBuf,
    pub repositories: BTreeMap<String, SeededRepo>,
}

impl SeededWorkspace {
    pub fn seed(run_root: &Path, scenario: &Scenario) -> Result<Self, DynError> {
        let root = run_root.join("workspace");
        fs::create_dir_all(root.join("repos"))?;
        let mut repositories = BTreeMap::new();

        for spec in &scenario.repositories {
            let repo_root = root.join("repos").join(&spec.id);
            fs::create_dir_all(&repo_root)?;
            run_git(&repo_root, &["init", "--quiet"])?;
            run_git(&repo_root, &["config", "user.name", "Agent Benchmark"])?;
            run_git(
                &repo_root,
                &["config", "user.email", "benchmark@invalid.local"],
            )?;
            write_files(&repo_root, &spec.initial_files)?;
            fs::write(
                repo_root.join(".agent-benchmark-repository-id"),
                format!("{}\n", spec.id),
            )?;
            run_git(&repo_root, &["add", "--all"])?;
            run_git(&repo_root, &["commit", "--quiet", "-m", "fixture baseline"])?;
            let baseline_sha = git_stdout(&repo_root, &["rev-parse", "HEAD"])?;
            write_files(&repo_root, &spec.dirty_user_files)?;
            let before = snapshot(&repo_root)?;
            let dirty_before = spec
                .dirty_user_files
                .iter()
                .map(|file| Ok((file.path.clone(), fs::read(repo_root.join(&file.path))?)))
                .collect::<Result<BTreeMap<_, _>, std::io::Error>>()?;
            repositories.insert(
                spec.id.clone(),
                SeededRepo {
                    root: repo_root,
                    baseline_sha,
                    before,
                    dirty_before,
                },
            );
        }

        Ok(Self { root, repositories })
    }

    pub fn public_manifest(&self, scenario: &Scenario, lane: &LaneSpec) -> PublicTaskManifest {
        let repositories = scenario
            .repositories
            .iter()
            .map(|spec| {
                let seeded = &self.repositories[&spec.id];
                RepositoryManifest {
                    id: spec.id.clone(),
                    path: seeded.root.display().to_string(),
                    baseline_sha: seeded.baseline_sha.clone(),
                    allowed_changed_paths: spec.allowed_changed_paths.clone(),
                    public_checks: spec.public_checks.clone(),
                    cloud_eligible: spec.cloud_eligible,
                }
            })
            .collect();
        PublicTaskManifest {
            schema_version: 1,
            scenario_id: scenario.id.clone(),
            title: scenario.title.clone(),
            prompt: scenario.prompt.clone(),
            repositories,
            contracts: scenario.edges.clone(),
            lane: lane.clone(),
        }
    }

    pub fn request(
        &self,
        scenario: &Scenario,
        lane: &LaneSpec,
        manifest_path: &Path,
        result_path: &Path,
    ) -> CandidateRequest {
        CandidateRequest {
            schema_version: 1,
            workspace_path: self.root.display().to_string(),
            manifest_path: manifest_path.display().to_string(),
            result_path: result_path.display().to_string(),
            task: self.public_manifest(scenario, lane),
            control: CandidateControl {
                injected_fault: scenario.fault,
            },
        }
    }
}

pub fn write_files(root: &Path, files: &[FileFixture]) -> Result<(), DynError> {
    for file in files {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, file.content.as_bytes())?;
    }
    Ok(())
}

pub fn clone_repository(source: &Path, destination: &Path) -> Result<(), DynError> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    command_ok(
        Command::new("git")
            .args(["clone", "--quiet"])
            .arg(source)
            .arg(destination),
        "clone fixture repository",
    )?;
    Ok(())
}

pub fn git_patch(root: &Path) -> Result<Vec<u8>, DynError> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["diff", "--binary", "--no-ext-diff", "HEAD", "--"])
        .output()?;
    ensure_output(output, "create git patch").map(|output| output.stdout)
}

pub fn apply_patch(root: &Path, patch: &[u8]) -> Result<(), DynError> {
    let mut child = Command::new("git")
        .current_dir(root)
        .args(["apply", "--whitespace=nowarn", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    use std::io::Write;
    child
        .stdin
        .take()
        .ok_or("git apply stdin was unavailable")?
        .write_all(patch)?;
    ensure_output(child.wait_with_output()?, "apply git patch")?;
    Ok(())
}

pub fn solution_changed_paths(root: &Path) -> Result<BTreeSet<String>, DynError> {
    let output = git_stdout(root, &["diff", "--name-only", "HEAD", "--"])?;
    Ok(output.lines().map(str::to_string).collect())
}

pub fn head_sha(root: &Path) -> Result<String, DynError> {
    git_stdout(root, &["rev-parse", "HEAD"])
}

pub fn result_tree_sha256(
    root: &Path,
    baseline_sha: &str,
    patch_sha256: &str,
    changed_paths: &BTreeSet<String>,
) -> Result<String, DynError> {
    let mut hasher = Sha256::new();
    hasher.update(baseline_sha.as_bytes());
    hasher.update([0]);
    hasher.update(patch_sha256.as_bytes());
    for path in changed_paths {
        hasher.update([0]);
        hasher.update(path.as_bytes());
        let absolute = root.join(path);
        match fs::symlink_metadata(&absolute) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!("result package contains a symlink: {path}").into());
            }
            Ok(metadata) if metadata.is_dir() => {
                return Err(format!("result package path is a directory: {path}").into());
            }
            Ok(_) => hasher.update(fs::read(absolute)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(b"<deleted>")
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn snapshot(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, DynError> {
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(root)?;
        if relative
            .components()
            .next()
            .is_some_and(|part| part.as_os_str() == ".git")
        {
            continue;
        }
        files.insert(
            relative.to_string_lossy().replace('\\', "/"),
            fs::read(entry.path())?,
        );
    }
    Ok(files)
}

fn run_git(root: &Path, args: &[&str]) -> Result<(), DynError> {
    command_ok(Command::new("git").current_dir(root).args(args), "run git")?;
    Ok(())
}

fn git_stdout(root: &Path, args: &[&str]) -> Result<String, DynError> {
    let output = ensure_output(
        Command::new("git").current_dir(root).args(args).output()?,
        "run git",
    )?;
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn command_ok(command: &mut Command, context: &str) -> Result<Output, DynError> {
    ensure_output(command.output()?, context)
}

fn ensure_output(output: Output, context: &str) -> Result<Output, DynError> {
    if output.status.success() {
        return Ok(output);
    }
    Err(format!(
        "{context} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )
    .into())
}
