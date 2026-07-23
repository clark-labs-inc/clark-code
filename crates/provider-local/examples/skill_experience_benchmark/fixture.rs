use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::model::{error, require, DynError};

pub struct HomeGuard {
    previous: Option<OsString>,
}

impl HomeGuard {
    pub fn enter(home: &Path) -> Result<Self, DynError> {
        require(
            home.is_dir(),
            format!("fake home {} is missing", home.display()),
        )?;
        let previous = std::env::var_os("HOME");
        // This benchmark uses a current-thread Tokio runtime and changes HOME
        // before constructing any provider task. The process is dedicated to
        // this benchmark, so no concurrent environment readers exist.
        unsafe { std::env::set_var("HOME", home) };
        Ok(Self { previous })
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            unsafe { std::env::set_var("HOME", previous) };
        } else {
            unsafe { std::env::remove_var("HOME") };
        }
    }
}

pub fn copy_tree(source: &Path, destination: &Path) -> Result<(), DynError> {
    require(
        source.is_dir(),
        format!("{} is not a directory", source.display()),
    )?;
    let canonical_source = source.canonicalize()?;
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(source)?;
        if relative
            .components()
            .any(|component| component.as_os_str() == ".git")
        {
            continue;
        }
        let target = destination.join(relative);
        if entry.file_type().is_symlink() {
            let resolved = entry.path().canonicalize()?;
            require(
                resolved.starts_with(&canonical_source),
                format!(
                    "benchmark symlink {} escapes source {}",
                    entry.path().display(),
                    source.display()
                ),
            )?;
            require(
                resolved.is_file(),
                format!(
                    "benchmark symlink {} must resolve to a regular file",
                    entry.path().display()
                ),
            )?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(resolved, target)?;
            continue;
        }
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

pub fn tree_digest(root: &Path) -> Result<String, DynError> {
    let mut files = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            !entry
                .path()
                .components()
                .any(|component| component.as_os_str() == ".git")
        })
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    files.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"clark-skill-experience-source-v1\0");
    for file in files {
        hasher.update(file.strip_prefix(root)?.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        hasher.update(std::fs::read(file)?);
        hasher.update(b"\0");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn init_repository(root: &Path) -> Result<(), DynError> {
    std::fs::create_dir_all(root)?;
    let output = Command::new("git")
        .args(["init", "-q", "--initial-branch=main"])
        .current_dir(root)
        .output()?;
    require(
        output.status.success(),
        format!(
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

pub fn seed_instructions(repository: &Path, selected: &Path, home: &Path) -> Result<(), DynError> {
    std::fs::create_dir_all(home.join(".clark"))?;
    std::fs::write(
        home.join(".clark/AGENTS.md"),
        "READ_BENCH_PERSONAL_INSTRUCTION\n",
    )?;
    std::fs::write(
        repository.join("AGENTS.md"),
        "READ_BENCH_PROJECT_INSTRUCTION\n",
    )?;
    std::fs::create_dir_all(selected)?;
    std::fs::write(
        selected.join("CLAUDE.md"),
        "READ_BENCH_NESTED_INSTRUCTION\n",
    )?;
    Ok(())
}

pub fn seed_collision(selected: &Path) -> Result<PathBuf, DynError> {
    let skill = selected.join(".agents/skills/brainstorming/SKILL.md");
    std::fs::create_dir_all(skill.parent().ok_or_else(|| error("skill has no parent"))?)?;
    std::fs::write(
        &skill,
        "---\nname: brainstorming\ndescription: Project-local collision fixture\n---\n\nPROJECT_COLLISION_BODY\n",
    )?;
    Ok(skill)
}

pub fn append_update_marker(source: &Path, marker: &str) -> Result<(), DynError> {
    let skill = source.join("skills/brainstorming/SKILL.md");
    let mut body = std::fs::read_to_string(&skill)?;
    body.push_str(&format!("\n\n<!-- {marker} -->\n"));
    std::fs::write(skill, body)?;
    Ok(())
}

#[cfg(unix)]
pub fn create_legacy_link(home: &Path, source: &Path) -> Result<&'static str, DynError> {
    use std::os::unix::fs::symlink;

    let root = home.join(".codex/skills");
    std::fs::create_dir_all(&root)?;
    symlink(
        source.join("skills").canonicalize()?,
        root.join("read-superpowers"),
    )?;
    Ok("directory_symlink")
}

#[cfg(not(unix))]
pub fn create_legacy_link(home: &Path, source: &Path) -> Result<&'static str, DynError> {
    copy_tree(
        &source.join("skills"),
        &home.join(".codex/skills/read-superpowers"),
    )?;
    Ok("directory_copy_fallback")
}

pub fn remove_legacy_link(home: &Path) -> Result<(), DynError> {
    let path = home.join(".codex/skills/read-superpowers");
    let metadata = std::fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(path)?;
    } else {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

pub fn create_synthetic_superpowers(root: &Path) -> Result<(), DynError> {
    for index in 0..12 {
        let name = if index == 0 {
            "brainstorming".to_string()
        } else {
            format!("fixture-skill-{index}")
        };
        let directory = root.join("skills").join(&name);
        std::fs::create_dir_all(&directory)?;
        std::fs::write(
            directory.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: Synthetic benchmark skill {index}\n---\n\n# {}\n\nSYNTHETIC_SKILL_BODY_{index}\n",
                if index == 0 { "Brainstorming Ideas Into Designs" } else { "Fixture" }
            ),
        )?;
        if index == 0 {
            std::fs::write(
                directory.join("visual-companion.md"),
                "VISUAL_COMPANION_FIXTURE\n",
            )?;
        }
    }
    Ok(())
}
