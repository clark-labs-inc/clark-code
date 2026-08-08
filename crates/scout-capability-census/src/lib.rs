//! Portable, read-only Scout capability census.
//!
//! The census inspects only the ambient executable `PATH`, names of ambient
//! environment variables, known credential locations (existence only), and
//! explicitly supplied workspace roots for dotenv schemas. It never executes
//! discovered programs or serializes environment/dotenv values.

mod registry;
mod scan;
mod system;

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use registry::{curated_executables, rust_fallback_gaps};
pub use registry::{CuratedExecutable, RustFallbackGap};
use scan::{scan_dotenv_roots, ScanOutcome};
pub use system::{collect_system_capabilities, SystemCapabilityCensus};

const MAX_SCAN_ROOTS: usize = 64;

/// Explicit bounds for dotenv discovery.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CensusLimits {
    pub max_depth: usize,
    pub max_directories: usize,
    pub max_dotenv_files: usize,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
    pub max_keys_per_file: usize,
}

impl Default for CensusLimits {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_directories: 4_096,
            max_dotenv_files: 128,
            max_total_bytes: 8 * 1_048_576,
            max_file_bytes: 1_048_576,
            max_keys_per_file: 512,
        }
    }
}

/// A portable census request. At least one explicit scan root is required.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CensusConfig {
    pub scan_roots: Vec<PathBuf>,
    #[serde(default)]
    pub limits: CensusLimits,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct NamedCapability {
    pub name: String,
    pub credential_candidate: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanRootReceipt {
    pub label: String,
    pub requested_path: String,
    pub resolved_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DotenvSchema {
    pub path: String,
    pub key_names: Vec<NamedCapability>,
    pub key_names_truncated: bool,
    pub schema_sha256: String,
    pub template: bool,
    pub bytes_read: u64,
    pub skipped_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CensusTruncation {
    pub path_executables: bool,
    pub environment_names: bool,
    pub directories: bool,
    pub depth: bool,
    pub dotenv_files: bool,
    pub total_bytes: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverageCounts {
    pub curated_executable_total: usize,
    pub curated_executable_present: usize,
    pub curated_executable_missing: usize,
    pub relevant_environment_names: usize,
    pub credential_surfaces: usize,
    pub roots_scanned: usize,
    pub directories_scanned: usize,
    pub dotenv_files_discovered: usize,
    pub dotenv_files_inspected: usize,
    pub dotenv_keys_discovered: usize,
    pub bytes_read: u64,
    pub skipped_symlinks: usize,
    pub skipped_unreadable: usize,
    pub rust_fallback_available: usize,
    pub rust_fallback_partial: usize,
    pub rust_fallback_missing: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactionReceipt {
    pub values_emitted: bool,
    pub discovered_executables_executed: bool,
    pub dotenv_values_read_but_not_retained: bool,
    pub emitted_data_classes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathEvidence {
    pub executable_name_count: usize,
    pub executable_names_sha256: String,
}

/// Deterministic, secret-safe receipt produced by [`run_census`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CensusReceipt {
    pub schema_version: String,
    pub platform: String,
    pub architecture: String,
    pub roots: Vec<ScanRootReceipt>,
    pub limits: CensusLimits,
    pub path_evidence: PathEvidence,
    pub curated_executables: Vec<CuratedExecutable>,
    pub environment: Vec<NamedCapability>,
    pub credential_surfaces: Vec<String>,
    pub dotenv_files: Vec<DotenvSchema>,
    pub rust_fallback_gaps: Vec<RustFallbackGap>,
    pub coverage: CoverageCounts,
    pub truncation: CensusTruncation,
    pub redaction: RedactionReceipt,
    pub semantic_digest_sha256: String,
}

#[derive(Debug, Error)]
pub enum CensusError {
    #[error("at least one explicit --root is required")]
    NoScanRoots,
    #[error("at most {MAX_SCAN_ROOTS} scan roots are allowed")]
    TooManyScanRoots,
    #[error("invalid census limit: {0}")]
    InvalidLimit(&'static str),
    #[error("scan root is a symlink and was refused: {0}")]
    SymlinkRoot(String),
    #[error("scan root is not a directory: {0}")]
    NotDirectory(String),
    #[error("scan root could not be inspected: {path}: {reason}")]
    RootInspection { path: String, reason: String },
}

#[derive(Clone)]
struct SystemSnapshot {
    platform: String,
    architecture: String,
    executable_names: Vec<String>,
    environment_names: Vec<String>,
    credential_surfaces: Vec<String>,
    executables_truncated: bool,
    environment_names_truncated: bool,
}

/// Run a bounded census against the current host without executing any
/// discovered program.
pub fn run_census(config: CensusConfig) -> Result<CensusReceipt, CensusError> {
    let system = collect_system_capabilities(None);
    run_with_snapshot(
        config,
        SystemSnapshot {
            platform: system.platform,
            architecture: system.architecture,
            executable_names: system.executable_names,
            environment_names: system.environment_variable_names,
            credential_surfaces: system.credential_surfaces,
            executables_truncated: system.executables_truncated,
            environment_names_truncated: system.environment_names_truncated,
        },
    )
}

fn run_with_snapshot(
    config: CensusConfig,
    mut system: SystemSnapshot,
) -> Result<CensusReceipt, CensusError> {
    validate_config(&config)?;
    system.executable_names.sort();
    system.executable_names.dedup();
    system.environment_names.sort();
    system.environment_names.dedup();
    system.credential_surfaces.sort();
    system.credential_surfaces.dedup();

    let executable_set = system
        .executable_names
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let curated_executables = curated_executables(&executable_set);
    let environment = system
        .environment_names
        .iter()
        .filter(|name| relevant_environment_name(name))
        .map(|name| named_capability(name.clone()))
        .collect::<Vec<_>>();
    let ScanOutcome {
        roots,
        dotenv_files,
        directories_scanned,
        bytes_read,
        skipped_symlinks,
        skipped_unreadable,
        mut truncation,
    } = scan_dotenv_roots(&config.scan_roots, &config.limits)?;
    truncation.path_executables = system.executables_truncated;
    truncation.environment_names = system.environment_names_truncated;

    let rust_fallback_gaps = rust_fallback_gaps(&curated_executables);
    let curated_executable_present = curated_executables
        .iter()
        .filter(|entry| entry.state == "present")
        .count();
    let dotenv_files_inspected = dotenv_files
        .iter()
        .filter(|file| file.skipped_reason.is_none())
        .count();
    let dotenv_keys_discovered = dotenv_files.iter().map(|file| file.key_names.len()).sum();
    let mut coverage = CoverageCounts {
        curated_executable_total: curated_executables.len(),
        curated_executable_present,
        curated_executable_missing: curated_executables.len() - curated_executable_present,
        relevant_environment_names: environment.len(),
        credential_surfaces: system.credential_surfaces.len(),
        roots_scanned: roots.len(),
        directories_scanned,
        dotenv_files_discovered: dotenv_files.len(),
        dotenv_files_inspected,
        dotenv_keys_discovered,
        bytes_read,
        skipped_symlinks,
        skipped_unreadable,
        rust_fallback_available: 0,
        rust_fallback_partial: 0,
        rust_fallback_missing: 0,
    };
    for fallback in &rust_fallback_gaps {
        match fallback.state.as_str() {
            "available" | "available_after_authorization" => coverage.rust_fallback_available += 1,
            "partial" => coverage.rust_fallback_partial += 1,
            _ => coverage.rust_fallback_missing += 1,
        }
    }

    let path_evidence = PathEvidence {
        executable_name_count: system.executable_names.len(),
        executable_names_sha256: names_hash(&system.executable_names),
    };
    let mut receipt = CensusReceipt {
        schema_version: "scout-portable-capability-census-v1".into(),
        platform: system.platform,
        architecture: system.architecture,
        roots,
        limits: config.limits,
        path_evidence,
        curated_executables,
        environment,
        credential_surfaces: system.credential_surfaces,
        dotenv_files,
        rust_fallback_gaps,
        coverage,
        truncation,
        redaction: RedactionReceipt {
            values_emitted: false,
            discovered_executables_executed: false,
            dotenv_values_read_but_not_retained: true,
            emitted_data_classes: vec![
                "executable_names_from_curated_registry".into(),
                "environment_variable_names".into(),
                "credential_surface_names".into(),
                "dotenv_paths_and_key_names".into(),
            ],
        },
        semantic_digest_sha256: String::new(),
    };
    receipt.semantic_digest_sha256 = semantic_digest(&receipt);
    Ok(receipt)
}

fn validate_config(config: &CensusConfig) -> Result<(), CensusError> {
    if config.scan_roots.is_empty() {
        return Err(CensusError::NoScanRoots);
    }
    if config.scan_roots.len() > MAX_SCAN_ROOTS {
        return Err(CensusError::TooManyScanRoots);
    }
    let limits = &config.limits;
    if limits.max_depth > 64 {
        return Err(CensusError::InvalidLimit("max_depth must be <= 64"));
    }
    if limits.max_directories == 0 || limits.max_directories > 1_000_000 {
        return Err(CensusError::InvalidLimit(
            "max_directories must be in 1..=1000000",
        ));
    }
    if limits.max_dotenv_files == 0 || limits.max_dotenv_files > 65_536 {
        return Err(CensusError::InvalidLimit(
            "max_dotenv_files must be in 1..=65536",
        ));
    }
    if limits.max_total_bytes == 0 || limits.max_total_bytes > 1_073_741_824 {
        return Err(CensusError::InvalidLimit(
            "max_total_bytes must be in 1..=1073741824",
        ));
    }
    if limits.max_file_bytes == 0 || limits.max_file_bytes > 67_108_864 {
        return Err(CensusError::InvalidLimit(
            "max_file_bytes must be in 1..=67108864",
        ));
    }
    if limits.max_file_bytes > limits.max_total_bytes {
        return Err(CensusError::InvalidLimit(
            "max_file_bytes must not exceed max_total_bytes",
        ));
    }
    if limits.max_keys_per_file == 0 || limits.max_keys_per_file > 65_536 {
        return Err(CensusError::InvalidLimit(
            "max_keys_per_file must be in 1..=65536",
        ));
    }
    Ok(())
}

fn named_capability(name: String) -> NamedCapability {
    NamedCapability {
        credential_candidate: credential_candidate(&name),
        name,
    }
}

fn relevant_environment_name(name: &str) -> bool {
    registry::relevant_environment_name(name)
}

fn credential_candidate(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "PRIVATE_KEY",
        "ACCESS_KEY",
        "API_KEY",
        "AUTH",
        "COOKIE",
        "SESSION",
    ]
    .iter()
    .any(|marker| upper.contains(marker))
}

fn names_hash(names: &[String]) -> String {
    let mut digest = Sha256::new();
    for name in names {
        digest.update(name.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

fn semantic_digest(receipt: &CensusReceipt) -> String {
    #[derive(Serialize)]
    struct SemanticDotenv<'a> {
        path: &'a str,
        key_names: &'a [NamedCapability],
        key_names_truncated: bool,
        schema_sha256: &'a str,
        template: bool,
        skipped_reason: &'a Option<String>,
    }
    #[derive(Serialize)]
    struct SemanticReceipt<'a> {
        schema_version: &'a str,
        curated_executables: &'a [CuratedExecutable],
        environment: &'a [NamedCapability],
        credential_surfaces: &'a [String],
        dotenv_files: Vec<SemanticDotenv<'a>>,
        rust_fallback_gaps: &'a [RustFallbackGap],
        truncation: &'a CensusTruncation,
        redaction: &'a RedactionReceipt,
    }
    let semantic = SemanticReceipt {
        schema_version: &receipt.schema_version,
        curated_executables: &receipt.curated_executables,
        environment: &receipt.environment,
        credential_surfaces: &receipt.credential_surfaces,
        dotenv_files: receipt
            .dotenv_files
            .iter()
            .map(|file| SemanticDotenv {
                path: &file.path,
                key_names: &file.key_names,
                key_names_truncated: file.key_names_truncated,
                schema_sha256: &file.schema_sha256,
                template: file.template,
                skipped_reason: &file.skipped_reason,
            })
            .collect(),
        rust_fallback_gaps: &receipt.rust_fallback_gaps,
        truncation: &receipt.truncation,
        redaction: &receipt.redaction,
    };
    let encoded = serde_json::to_vec(&semantic).expect("semantic census receipt serializes");
    format!("{:x}", Sha256::digest(encoded))
}

#[cfg(test)]
mod tests;
