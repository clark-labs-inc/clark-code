use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::exec::Executor;

use super::identity::hex_digest;
use super::{display_relative, is_security_output, SECURITY_SCAN_CONTRACT_VERSION};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityDiffKind {
    WorkingTree,
    Range,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityDiffTarget {
    pub kind: SecurityDiffKind,
    pub base: String,
    #[serde(default)]
    pub head: Option<String>,
    pub target_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityDiffFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_path: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityDiffInventory {
    pub contract_version: u32,
    pub scope: String,
    pub target: SecurityDiffTarget,
    pub resolved_base: String,
    pub resolved_head: String,
    pub changed_files: Vec<SecurityDiffFile>,
}

pub async fn collect_security_diff_inventory(
    exec: &dyn Executor,
    root: &Path,
    scope: &Path,
    kind: SecurityDiffKind,
    base: &str,
    head: Option<&str>,
) -> Result<SecurityDiffInventory, String> {
    if !scope.starts_with(root) {
        return Err(format!(
            "security scope {} is outside project root {}",
            scope.display(),
            root.display()
        ));
    }
    if !crate::checkpoint::is_git_repo(exec, root).await {
        return Err("diff security scans require a Git working tree".into());
    }
    let base = nonempty_revision("base", base)?;
    let resolved_base = resolve_commit(exec, root, base).await?;
    let (head_label, resolved_head) = match kind {
        SecurityDiffKind::WorkingTree => {
            if head.is_some() {
                return Err("working_tree diff targets must not specify head".into());
            }
            (
                None,
                crate::checkpoint::working_tree(exec, root)
                    .await
                    .map_err(|error| format!("snapshotting working tree: {error}"))?,
            )
        }
        SecurityDiffKind::Range => {
            let head = nonempty_revision(
                "head",
                head.ok_or_else(|| "range diff targets require head".to_string())?,
            )?;
            (
                Some(head.to_string()),
                resolve_commit(exec, root, head).await?,
            )
        }
    };
    let scope_name = display_relative(root, scope);
    let mut args = vec![
        "diff".to_string(),
        "--find-renames".to_string(),
        "--raw".to_string(),
        "--full-index".to_string(),
        "-z".to_string(),
        resolved_base.clone(),
        resolved_head.clone(),
    ];
    if scope_name != "." {
        args.push("--".into());
        args.push(scope_name.clone());
    }
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    let raw = crate::git_metadata::required(exec, root, &args).await?;
    let mut records = parse_raw_diff(&raw)?;
    records.retain(|record| {
        !is_security_output(&record.file.path)
            && record
                .file
                .previous_path
                .as_deref()
                .is_none_or(|path| !is_security_output(path))
    });
    records.sort_by(|left, right| {
        left.file
            .path
            .cmp(&right.file.path)
            .then_with(|| left.file.previous_path.cmp(&right.file.previous_path))
    });
    records.dedup_by(|left, right| left.file == right.file);
    let target_id = target_digest(&scope_name, kind, &resolved_base, &resolved_head, &records);
    let changed_files = records
        .into_iter()
        .map(|record| record.file)
        .collect::<Vec<_>>();
    Ok(SecurityDiffInventory {
        contract_version: SECURITY_SCAN_CONTRACT_VERSION,
        scope: scope_name,
        target: SecurityDiffTarget {
            kind,
            base: base.to_string(),
            head: head_label,
            target_id,
        },
        resolved_base,
        resolved_head,
        changed_files,
    })
}

fn nonempty_revision<'a>(name: &str, value: &'a str) -> Result<&'a str, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("{name} revision must not be empty"))
    } else {
        Ok(value)
    }
}

async fn resolve_commit(
    exec: &dyn Executor,
    root: &Path,
    revision: &str,
) -> Result<String, String> {
    let commit = format!("{revision}^{{commit}}");
    let resolved =
        crate::git_metadata::required(exec, root, &["rev-parse", "--verify", &commit]).await?;
    let resolved = resolved.trim();
    if resolved.is_empty() {
        Err(format!("Git returned an empty object id for `{revision}`"))
    } else {
        Ok(resolved.to_string())
    }
}

#[derive(Clone, Debug)]
struct DiffRecord {
    file: SecurityDiffFile,
    object_transition: String,
}

fn parse_raw_diff(raw: &str) -> Result<Vec<DiffRecord>, String> {
    let mut fields = raw.split_terminator('\0');
    let mut records = Vec::new();
    while let Some(header) = fields.next() {
        let status = header
            .split_whitespace()
            .last()
            .ok_or_else(|| format!("Git returned an invalid raw diff header `{header}`"))?;
        let status_code = status.chars().next().ok_or_else(|| {
            "Git returned an empty status while inventorying the security diff".to_string()
        })?;
        if matches!(status_code, 'R' | 'C') {
            let previous_path = fields
                .next()
                .ok_or_else(|| format!("Git omitted the source path for status `{status}`"))?;
            let path = fields
                .next()
                .ok_or_else(|| format!("Git omitted the target path for status `{status}`"))?;
            let previous_path = previous_path.replace('\\', "/");
            let path = path.replace('\\', "/");
            records.push(DiffRecord {
                object_transition: format!("{header}\0{previous_path}\0{path}"),
                file: SecurityDiffFile {
                    path,
                    previous_path: Some(previous_path),
                    status: status_label(status_code).into(),
                },
            });
        } else {
            let path = fields
                .next()
                .ok_or_else(|| format!("Git omitted the path for status `{status}`"))?;
            let path = path.replace('\\', "/");
            records.push(DiffRecord {
                object_transition: format!("{header}\0{path}"),
                file: SecurityDiffFile {
                    path,
                    previous_path: None,
                    status: status_label(status_code).into(),
                },
            });
        }
    }
    Ok(records)
}

fn status_label(code: char) -> &'static str {
    match code {
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'T' => "type_changed",
        'U' => "unmerged",
        _ => "modified",
    }
}

fn target_digest(
    scope: &str,
    kind: SecurityDiffKind,
    base: &str,
    head: &str,
    records: &[DiffRecord],
) -> String {
    // Range scans bind the exact immutable head commit. Working-tree scans bind
    // the in-scope blob transitions instead of the whole throwaway tree: that
    // lets the scan write its own excluded `.clark/security-scans` bundle
    // without invalidating an otherwise unchanged target.
    let head_identity = match kind {
        SecurityDiffKind::WorkingTree => "",
        SecurityDiffKind::Range => head,
    };
    let mut input =
        format!("clark-security-diff-v1\0{scope}\0{kind:?}\0{base}\0{head_identity}").into_bytes();
    for record in records {
        input.push(0);
        input.extend_from_slice(record.object_transition.as_bytes());
    }
    hex_digest(&input)
}
