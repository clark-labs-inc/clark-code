use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::model::{
    CandidateControl, CandidateRequest, FileFixture, LaneSpec, ProjectManifest, PublicTaskManifest,
    Scenario,
};

pub type DynError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Debug)]
pub struct ProjectWorkspace {
    pub root: PathBuf,
    pub baseline_sha: String,
    pub original_dirty_files: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct SeededWorkspace {
    pub root: PathBuf,
    pub projects: BTreeMap<String, ProjectWorkspace>,
}

#[derive(Clone, Debug)]
pub struct BehaviorCheck {
    pub id: String,
    pub passed: bool,
    pub detail: String,
}

impl SeededWorkspace {
    pub fn seed(run_root: &Path, scenario: &Scenario) -> Result<Self, DynError> {
        let root = run_root.join("workspace");
        let repos_root = root.join("repos");
        fs::create_dir_all(&repos_root)?;
        let mut projects = BTreeMap::new();
        for project in &scenario.projects {
            let project_root = repos_root.join(&project.id);
            fs::create_dir_all(&project_root)?;
            write_files(&project_root, &project.initial_files)?;
            git(&project_root, &["init", "--quiet"])?;
            git(
                &project_root,
                &["config", "user.email", "benchmark@example.test"],
            )?;
            git(&project_root, &["config", "user.name", "Agent Benchmark"])?;
            git(&project_root, &["add", "."])?;
            git(
                &project_root,
                &["commit", "--quiet", "-m", "synthetic baseline"],
            )?;
            let baseline_sha = git_output(&project_root, &["rev-parse", "HEAD"])?;
            write_files(&project_root, &project.dirty_user_files)?;
            let original_dirty_files = project
                .dirty_user_files
                .iter()
                .map(|file| Ok((file.path.clone(), fs::read(project_root.join(&file.path))?)))
                .collect::<Result<_, std::io::Error>>()?;
            projects.insert(
                project.id.clone(),
                ProjectWorkspace {
                    root: project_root,
                    baseline_sha,
                    original_dirty_files,
                },
            );
        }
        Ok(Self { root, projects })
    }

    pub fn apply_solution(&self, scenario: &Scenario) -> Result<(), DynError> {
        for project in &scenario.projects {
            write_files(&self.projects[&project.id].root, &project.solution_files)?;
        }
        Ok(())
    }

    pub fn public_manifest(&self, scenario: &Scenario, lane: &LaneSpec) -> PublicTaskManifest {
        PublicTaskManifest {
            schema_version: 1,
            scenario_id: scenario.id.clone(),
            title: scenario.title.clone(),
            prompt: scenario.prompt.clone(),
            projects: scenario
                .projects
                .iter()
                .map(|project| {
                    let workspace = &self.projects[&project.id];
                    ProjectManifest {
                        id: project.id.clone(),
                        path: workspace.root.display().to_string(),
                        baseline_sha: workspace.baseline_sha.clone(),
                        allowed_changed_paths: project.allowed_changed_paths.clone(),
                        cloud_eligible: project.cloud_eligible,
                    }
                })
                .collect(),
            lane: lane.clone(),
        }
    }

    pub fn request(
        &self,
        scenario: &Scenario,
        lane: &LaneSpec,
        result_path: &Path,
    ) -> CandidateRequest {
        CandidateRequest {
            schema_version: 1,
            workspace_path: self.root.display().to_string(),
            result_path: result_path.display().to_string(),
            task: self.public_manifest(scenario, lane),
            control: CandidateControl {
                fault: scenario.fault,
            },
        }
    }

    pub fn behavior_checks(&self, scenario: &Scenario) -> Result<Vec<BehaviorCheck>, DynError> {
        let mut checks = Vec::new();
        for project in &scenario.projects {
            let workspace = &self.projects[&project.id];
            for expected in &project.solution_files {
                let actual = fs::read(workspace.root.join(&expected.path)).unwrap_or_default();
                checks.push(BehaviorCheck {
                    id: format!("solution:{}:{}", project.id, expected.path),
                    passed: actual == expected.content.as_bytes(),
                    detail: format!(
                        "{} must contain the hidden expected result",
                        workspace.root.join(&expected.path).display()
                    ),
                });
            }
            for (path, expected) in &workspace.original_dirty_files {
                let actual = fs::read(workspace.root.join(path)).unwrap_or_default();
                checks.push(BehaviorCheck {
                    id: format!("preserve-user-file:{}:{path}", project.id),
                    passed: &actual == expected,
                    detail: format!("pre-existing user file {path} must remain byte-identical"),
                });
            }
            let head = git_output(&workspace.root, &["rev-parse", "HEAD"])?;
            checks.push(BehaviorCheck {
                id: format!("baseline-head:{}", project.id),
                passed: head == workspace.baseline_sha,
                detail: "candidate must not move the synthetic repository HEAD".into(),
            });
            let changed = git_output(&workspace.root, &["diff", "--name-only"])?;
            let changed_paths = changed
                .lines()
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            let allowed = changed_paths
                .iter()
                .all(|path| project.allowed_changed_paths.contains(*path));
            checks.push(BehaviorCheck {
                id: format!("write-scope:{}", project.id),
                passed: allowed,
                detail: format!("changed tracked paths: {changed_paths:?}"),
            });
        }
        Ok(checks)
    }

    pub fn baselines(&self) -> BTreeMap<String, String> {
        self.projects
            .iter()
            .map(|(id, project)| (id.clone(), project.baseline_sha.clone()))
            .collect()
    }
}

fn write_files(root: &Path, files: &[FileFixture]) -> Result<(), DynError> {
    for file in files {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, file.content.as_bytes())?;
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<(), DynError> {
    let status = Command::new("git").current_dir(root).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {args:?} failed in {}", root.display()).into())
    }
}

fn git_output(root: &Path, args: &[&str]) -> Result<String, DynError> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    if !output.status.success() {
        return Err(format!("git {args:?} failed in {}", root.display()).into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}
