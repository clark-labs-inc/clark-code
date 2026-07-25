use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{named_capability, DotenvFile, NamedCapability};
use crate::tools::ToolCtx;

const MAX_DOTENV_FILES: usize = 128;
const MAX_DOTENV_BYTES: u64 = 1_048_576;
const MAX_DIRECTORIES: usize = 4_096;
const MAX_KEYS_PER_DOTENV: usize = 512;

pub(super) async fn scan_dotenv(ctx: &ToolCtx, scope: &Path) -> (Vec<DotenvFile>, bool) {
    let (paths, mut truncated) = discover_dotenv_paths(ctx, scope).await;
    let mut reports = Vec::with_capacity(paths.len());
    for path in paths {
        if reports.len() >= MAX_DOTENV_FILES {
            truncated = true;
            break;
        }
        reports.push(inspect_dotenv(ctx, path).await);
    }
    reports.sort_by(|left, right| left.path.cmp(&right.path));
    (reports, truncated)
}

async fn discover_dotenv_paths(ctx: &ToolCtx, scope: &Path) -> (Vec<PathBuf>, bool) {
    let mut directories = VecDeque::from([scope.to_path_buf()]);
    let mut visited = 0usize;
    let mut paths = Vec::new();
    let mut truncated = false;

    while let Some(directory) = directories.pop_front() {
        if visited >= MAX_DIRECTORIES || paths.len() >= MAX_DOTENV_FILES {
            truncated = true;
            break;
        }
        visited += 1;
        let Ok(mut entries) = ctx.executor.read_dir(&directory).await else {
            continue;
        };
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        for entry in entries {
            if entry.is_symlink {
                continue;
            }
            let path = directory.join(&entry.name);
            if entry.is_dir {
                if !ignored_directory(&entry.name) {
                    directories.push_back(path);
                }
            } else if is_dotenv_path(&path) {
                paths.push(path);
            }
        }
    }
    (paths, truncated)
}

async fn inspect_dotenv(ctx: &ToolCtx, path: PathBuf) -> DotenvFile {
    let display = ctx.sandbox.display(&path);
    let template = is_template_path(&path);
    let length = match ctx.executor.metadata(&path).await {
        Ok(metadata) => metadata.len,
        Err(error) => return skipped(display, template, format!("metadata failed: {error}")),
    };
    if length > MAX_DOTENV_BYTES {
        return skipped(
            display,
            template,
            "file exceeds the 1 MiB census limit".into(),
        );
    }
    let bytes = match ctx.executor.read(&path).await {
        Ok(bytes) => bytes,
        Err(error) => return skipped(display, template, format!("read failed: {error}")),
    };
    let Some(text) = std::str::from_utf8(&bytes).ok() else {
        return skipped(display, template, "file is not UTF-8".into());
    };
    let mut keys = dotenv_keys(text);
    let schema_sha256 = safe_schema_hash(&keys);
    let keys_truncated = keys.len() > MAX_KEYS_PER_DOTENV;
    keys.truncate(MAX_KEYS_PER_DOTENV);
    DotenvFile {
        path: display,
        schema_sha256,
        keys,
        keys_truncated,
        template,
        skipped_reason: None,
    }
}

fn skipped(path: String, template: bool, reason: String) -> DotenvFile {
    DotenvFile {
        path,
        keys: Vec::new(),
        keys_truncated: false,
        schema_sha256: safe_schema_hash(&[]),
        template,
        skipped_reason: Some(reason),
    }
}

fn ignored_directory(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "target" | "dist" | ".next" | ".venv"
    )
}

fn is_dotenv_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".env" || name.starts_with(".env.") || name.ends_with(".env"))
}

fn is_template_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.contains("example") || name.contains("sample") || name.contains("template")
        })
}

pub(super) fn dotenv_keys(text: &str) -> Vec<NamedCapability> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (name, _) = line.split_once('=')?;
            let name = name.trim();
            valid_environment_name(name).then(|| named_capability(name.to_string()))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn safe_schema_hash(keys: &[NamedCapability]) -> String {
    let mut digest = Sha256::new();
    for key in keys {
        digest.update(key.name.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}
