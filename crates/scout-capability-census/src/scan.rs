use std::collections::{BTreeSet, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{
    named_capability, CensusError, CensusLimits, CensusTruncation, DotenvSchema, NamedCapability,
    ScanRootReceipt,
};

pub(super) struct ScanOutcome {
    pub roots: Vec<ScanRootReceipt>,
    pub dotenv_files: Vec<DotenvSchema>,
    pub directories_scanned: usize,
    pub bytes_read: u64,
    pub skipped_symlinks: usize,
    pub skipped_unreadable: usize,
    pub truncation: CensusTruncation,
}

struct PendingDirectory {
    root_index: usize,
    path: PathBuf,
    relative: PathBuf,
    depth: usize,
}

pub(super) fn scan_dotenv_roots(
    requested_roots: &[PathBuf],
    limits: &CensusLimits,
) -> Result<ScanOutcome, CensusError> {
    let mut roots = Vec::with_capacity(requested_roots.len());
    let mut queue = VecDeque::new();
    for (index, requested) in requested_roots.iter().enumerate() {
        let metadata =
            std::fs::symlink_metadata(requested).map_err(|error| CensusError::RootInspection {
                path: requested.display().to_string(),
                reason: error.to_string(),
            })?;
        if metadata.file_type().is_symlink() {
            return Err(CensusError::SymlinkRoot(requested.display().to_string()));
        }
        if !metadata.is_dir() {
            return Err(CensusError::NotDirectory(requested.display().to_string()));
        }
        let resolved =
            std::fs::canonicalize(requested).map_err(|error| CensusError::RootInspection {
                path: requested.display().to_string(),
                reason: error.to_string(),
            })?;
        let label = format!("root[{index}]");
        roots.push(ScanRootReceipt {
            label,
            requested_path: requested.display().to_string(),
            resolved_path: resolved.display().to_string(),
        });
        queue.push_back(PendingDirectory {
            root_index: index,
            path: resolved,
            relative: PathBuf::new(),
            depth: 0,
        });
    }

    let mut outcome = ScanOutcome {
        roots,
        dotenv_files: Vec::new(),
        directories_scanned: 0,
        bytes_read: 0,
        skipped_symlinks: 0,
        skipped_unreadable: 0,
        truncation: CensusTruncation::default(),
    };

    while let Some(pending) = queue.pop_front() {
        if outcome.directories_scanned >= limits.max_directories {
            outcome.truncation.directories = true;
            break;
        }
        outcome.directories_scanned += 1;
        let entries = match std::fs::read_dir(&pending.path) {
            Ok(entries) => entries,
            Err(_) => {
                outcome.skipped_unreadable += 1;
                continue;
            }
        };
        let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            let path = entry.path();
            let relative = pending.relative.join(&name);
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    outcome.skipped_unreadable += 1;
                    continue;
                }
            };
            if file_type.is_symlink() {
                outcome.skipped_symlinks += 1;
                continue;
            }
            if file_type.is_dir() {
                if ignored_directory(&name.to_string_lossy()) {
                    continue;
                }
                if pending.depth >= limits.max_depth {
                    outcome.truncation.depth = true;
                    continue;
                }
                queue.push_back(PendingDirectory {
                    root_index: pending.root_index,
                    path,
                    relative,
                    depth: pending.depth + 1,
                });
                continue;
            }
            if !file_type.is_file() || !is_dotenv_path(&path) {
                continue;
            }
            if outcome.dotenv_files.len() >= limits.max_dotenv_files {
                outcome.truncation.dotenv_files = true;
                queue.clear();
                break;
            }
            let label = format!(
                "root[{}]/{}",
                pending.root_index,
                portable_relative_path(&relative)
            );
            let file = inspect_dotenv(&path, label, limits, outcome.bytes_read);
            if file.skipped_reason.as_deref() == Some("total_byte_limit") {
                outcome.truncation.total_bytes = true;
            }
            if file.skipped_reason.as_deref() == Some("unreadable") {
                outcome.skipped_unreadable += 1;
            }
            outcome.bytes_read += file.bytes_read;
            outcome.dotenv_files.push(file);
        }
    }
    outcome
        .dotenv_files
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(outcome)
}

fn inspect_dotenv(
    path: &Path,
    label: String,
    limits: &CensusLimits,
    bytes_already_read: u64,
) -> DotenvSchema {
    let empty_hash = schema_hash(&[]);
    let template = is_template_path(path);
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        _ => return skipped(label, template, empty_hash, "unreadable"),
    };
    if metadata.len() > limits.max_file_bytes {
        return skipped(label, template, empty_hash, "file_byte_limit");
    }
    if metadata.len() > limits.max_total_bytes.saturating_sub(bytes_already_read) {
        return skipped(label, template, empty_hash, "total_byte_limit");
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return skipped(label, template, empty_hash, "unreadable"),
    };
    let Ok(open_metadata) = file.metadata() else {
        return skipped(label, template, empty_hash, "unreadable");
    };
    if !open_metadata.is_file() {
        return skipped(label, template, empty_hash, "unreadable");
    }
    let read_bound = limits
        .max_file_bytes
        .min(limits.max_total_bytes.saturating_sub(bytes_already_read));
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    if file
        .by_ref()
        .take(read_bound + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return skipped(label, template, empty_hash, "unreadable");
    }
    if bytes.len() as u64 > read_bound {
        return skipped(label, template, empty_hash, "file_changed_or_byte_limit");
    }
    let bytes_read = bytes.len() as u64;
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return DotenvSchema {
            path: label,
            key_names: Vec::new(),
            key_names_truncated: false,
            schema_sha256: empty_hash,
            template,
            bytes_read,
            skipped_reason: Some("non_utf8".into()),
        };
    };
    let mut keys = dotenv_keys(text);
    let schema_sha256 = schema_hash(&keys);
    let key_names_truncated = keys.len() > limits.max_keys_per_file;
    keys.truncate(limits.max_keys_per_file);
    DotenvSchema {
        path: label,
        key_names: keys,
        key_names_truncated,
        schema_sha256,
        template,
        bytes_read,
        skipped_reason: None,
    }
}

fn skipped(path: String, template: bool, schema_sha256: String, reason: &str) -> DotenvSchema {
    DotenvSchema {
        path,
        key_names: Vec::new(),
        key_names_truncated: false,
        schema_sha256,
        template,
        bytes_read: 0,
        skipped_reason: Some(reason.into()),
    }
}

fn dotenv_keys(text: &str) -> Vec<NamedCapability> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (name, _) = line.split_once('=')?;
            let name = name.trim();
            valid_environment_name(name).then(|| named_capability(name.into()))
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

fn schema_hash(keys: &[NamedCapability]) -> String {
    let mut digest = Sha256::new();
    for key in keys {
        digest.update(key.name.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

fn portable_relative_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn ignored_directory(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".next"
            | ".venv"
            | "venv"
            | "__pycache__"
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
            let lower = name.to_ascii_lowercase();
            lower.contains("example") || lower.contains("sample") || lower.contains("template")
        })
}
