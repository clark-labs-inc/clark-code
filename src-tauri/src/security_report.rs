use std::path::{Path, PathBuf};

use provider_local::SecurityScanRecord;
use serde::Deserialize;

use crate::markdown_export::render_markdown_pdf;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScanReportSummary {
    id: String,
    repository_id: String,
    mode: String,
    model: String,
    status: String,
    created_at: String,
    generated_at: String,
}

/// Export one Security scan through the foundation's bundled pure-Rust PDF
/// renderer. The cloud row is always represented; when its originating local
/// record is available, the report also includes the sealed evidence bundle.
#[tauri::command]
pub async fn export_security_scan_pdf(
    path: String,
    scan: SecurityScanReportSummary,
    local_record: Option<SecurityScanRecord>,
) -> Result<(), String> {
    let destination = PathBuf::from(path);
    let report = build_report(&scan, local_record.as_ref());

    tokio::task::spawn_blocking(move || {
        let pdf = render_markdown_pdf(Path::new("security-report.md"), report.as_bytes())?;
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("create dir: {error}"))?;
        }
        std::fs::write(&destination, pdf).map_err(|error| format!("write failed: {error}"))
    })
    .await
    .map_err(|error| format!("Security PDF export failed: {error}"))?
}

fn build_report(
    scan: &SecurityScanReportSummary,
    local_record: Option<&SecurityScanRecord>,
) -> String {
    let mut out = String::new();
    out.push_str("# Security Report\n\n");
    out.push_str("*Evidence-backed repository security assessment*\n\n");
    out.push_str("---\n\n");
    out.push_str("## Executive summary\n\n");

    if let Some(record) = local_record {
        let reportable = record
            .bundle
            .candidates
            .iter()
            .filter(|candidate| is_reportable(candidate.validation.disposition))
            .count();
        let deferred = record
            .bundle
            .candidates
            .iter()
            .filter(|candidate| is_deferred(candidate.validation.disposition))
            .count();
        let (critical, high, medium, low) = severity_counts(record);
        let integrity = if record.seal.is_some() {
            "Sealed and integrity-verified"
        } else {
            "In progress - evidence is not sealed"
        };
        out.push_str(&format!(
            "The security scan identified **{reportable} reportable finding{}**. The report contains {critical} critical, {high} high, {medium} medium, and {low} low severity findings. {deferred} candidate{} remain deferred. **Evidence status:** {integrity}.\n\n",
            plural(reportable),
            plural(deferred),
        ));
    } else {
        out.push_str(
            "This historical cloud scan is not backed by a local evidence bundle in the active checkout. "
        );
        out.push_str(
            "The PDF therefore reports only authoritative scan metadata and does not infer findings or coverage.\n\n",
        );
    }

    out.push_str("| Scan detail | Value |\n|---|---|\n");
    table_row(&mut out, "Scan ID", &scan.id);
    table_row(&mut out, "Repository", &scan.repository_id);
    table_row(&mut out, "Mode", &scan.mode);
    table_row(&mut out, "Status", &scan.status);
    table_row(&mut out, "Model", &scan.model);
    table_row(&mut out, "Created", &scan.created_at);
    table_row(&mut out, "Report generated", &scan.generated_at);

    let Some(record) = local_record else {
        out.push_str("\n## Evidence availability\n\n");
        out.push_str(
            "Open the repository that produced this scan to export its findings, coverage, and validation evidence.\n",
        );
        return out;
    };

    out.push_str("\n## Risk overview\n\n");
    let (critical, high, medium, low) = severity_counts(record);
    out.push_str("| Severity | Findings |\n|---|---:|\n");
    table_row(&mut out, "Critical", &critical.to_string());
    table_row(&mut out, "High", &high.to_string());
    table_row(&mut out, "Medium", &medium.to_string());
    table_row(&mut out, "Low", &low.to_string());

    out.push_str("\n## Threat model\n\n");
    out.push_str("| Threat surface | Recorded assumptions |\n|---|---|\n");
    table_list(&mut out, "Assets", &record.bundle.threat_model.assets);
    table_list(
        &mut out,
        "Trust boundaries",
        &record.bundle.threat_model.trust_boundaries,
    );
    table_list(
        &mut out,
        "Attacker inputs",
        &record.bundle.threat_model.attacker_inputs,
    );
    table_list(
        &mut out,
        "Security invariants",
        &record.bundle.threat_model.invariants,
    );

    out.push_str("\n## Validated findings\n\n");
    let mut findings = record
        .bundle
        .candidates
        .iter()
        .filter(|candidate| is_reportable(candidate.validation.disposition))
        .collect::<Vec<_>>();
    findings.sort_by_key(|candidate| std::cmp::Reverse(severity_rank(candidate.severity)));
    if findings.is_empty() {
        out.push_str("No reportable findings were sealed for this scan.\n");
    }
    for (index, candidate) in findings.iter().enumerate() {
        out.push_str(&format!(
            "\n### {}. {}\n\n",
            index + 1,
            markdown_text(&candidate.title)
        ));
        out.push_str("| Attribute | Value |\n|---|---|\n");
        table_row(
            &mut out,
            "Severity / confidence",
            &format!(
                "{} / {}",
                enum_label(candidate.severity),
                enum_label(candidate.confidence)
            ),
        );
        table_row(&mut out, "Category", &candidate.category);
        table_row(&mut out, "Rule", &candidate.rule_id);
        table_row(&mut out, "CWE", &candidate.cwe.join(", "));
        table_row(&mut out, "PoC outcome", &enum_label(candidate.poc.outcome));
        out.push('\n');
        paragraph(&mut out, "Summary", &candidate.summary);
        paragraph(&mut out, "Impact", &candidate.impact);
        paragraph(
            &mut out,
            "Validation evidence",
            &candidate.validation.evidence,
        );
        location(&mut out, "Source", &candidate.source);
        location(&mut out, "Control", &candidate.control);
        location(&mut out, "Sink", &candidate.sink);
        if let Some(path) = &candidate.attack_path {
            out.push_str("\n**Attack path**\n\n");
            out.push_str(&format!(
                "{} -> {} -> {}\n\n",
                markdown_text(&path.attacker),
                markdown_text(&path.entrypoint),
                path.path
                    .iter()
                    .map(|step| markdown_text(step))
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ));
            paragraph(&mut out, "Likelihood", &path.likelihood);
            bullet_section(&mut out, "Preconditions", &path.preconditions);
        }
        paragraph(&mut out, "Recommended remediation", &candidate.remediation);
        bullet_section(
            &mut out,
            "Counterevidence considered",
            &candidate.validation.counterevidence,
        );
        bullet_section(&mut out, "PoC limitations", &candidate.poc.limitations);
    }

    out.push_str("\n## Coverage and integrity\n\n");
    let reviewed = record
        .bundle
        .coverage
        .iter()
        .filter(|coverage| {
            matches!(
                coverage.status,
                provider_local::security::SecurityCoverageStatus::Reviewed
            )
        })
        .count();
    let excluded = record.bundle.coverage.len().saturating_sub(reviewed);
    out.push_str("| Evidence measure | Value |\n|---|---:|\n");
    table_row(&mut out, "Reviewed files", &reviewed.to_string());
    table_row(&mut out, "Explicitly excluded files", &excluded.to_string());
    table_row(
        &mut out,
        "Supporting files",
        &record.bundle.supporting_coverage.len().to_string(),
    );
    table_row(
        &mut out,
        "Candidates evaluated",
        &record.bundle.candidates.len().to_string(),
    );
    if let Some(seal) = &record.seal {
        table_row(
            &mut out,
            "Bundle digest",
            &wrapped_digest(&seal.bundle_digest),
        );
        table_row(
            &mut out,
            "PoCs attempted",
            &seal.poc_attempted_count.to_string(),
        );
        table_row(
            &mut out,
            "PoCs reproduced",
            &seal.poc_reproduced_count.to_string(),
        );
    }

    let exclusions = record
        .bundle
        .coverage
        .iter()
        .filter_map(|coverage| {
            coverage
                .reason
                .as_ref()
                .map(|reason| format!("{} - {}", coverage.path, reason))
        })
        .collect::<Vec<_>>();
    bullet_section(&mut out, "Explicit exclusions", &exclusions);

    let deferred = record
        .bundle
        .candidates
        .iter()
        .filter(|candidate| is_deferred(candidate.validation.disposition))
        .map(|candidate| {
            let reason = if candidate.poc.limitations.is_empty() {
                "validation incomplete".to_string()
            } else {
                candidate.poc.limitations.join("; ")
            };
            format!("{} - {}", candidate.title, reason)
        })
        .collect::<Vec<_>>();
    bullet_section(&mut out, "Deferred work", &deferred);

    out.push_str("\n---\n\n");
    out.push_str(
        "This report preserves the scan's recorded evidence and dispositions. It is not a guarantee that unreviewed or changed code is vulnerability-free.\n",
    );
    out
}

fn severity_counts(record: &SecurityScanRecord) -> (usize, usize, usize, usize) {
    let mut counts = (0, 0, 0, 0);
    for candidate in record
        .bundle
        .candidates
        .iter()
        .filter(|candidate| is_reportable(candidate.validation.disposition))
    {
        match enum_label(candidate.severity).as_str() {
            "critical" => counts.0 += 1,
            "high" => counts.1 += 1,
            "medium" => counts.2 += 1,
            _ => counts.3 += 1,
        }
    }
    counts
}

fn severity_rank(severity: provider_local::security::SecuritySeverity) -> u8 {
    match severity {
        provider_local::security::SecuritySeverity::Critical => 4,
        provider_local::security::SecuritySeverity::High => 3,
        provider_local::security::SecuritySeverity::Medium => 2,
        provider_local::security::SecuritySeverity::Low => 1,
    }
}

fn is_reportable(disposition: provider_local::security::SecurityDisposition) -> bool {
    matches!(
        disposition,
        provider_local::security::SecurityDisposition::Reportable
    )
}

fn is_deferred(disposition: provider_local::security::SecurityDisposition) -> bool {
    matches!(
        disposition,
        provider_local::security::SecurityDisposition::Deferred
    )
}

fn enum_label(value: impl std::fmt::Debug) -> String {
    format!("{value:?}").to_lowercase()
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn table_row(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!(
        "| {} | {} |\n",
        table_text(label),
        table_text(value)
    ));
}

fn table_list(out: &mut String, label: &str, values: &[String]) {
    table_row(out, label, &values.join("; "));
}

fn wrapped_digest(digest: &str) -> String {
    let chunks = digest
        .as_bytes()
        .chunks(16)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default());
    format!("sha256: {}", chunks.collect::<Vec<_>>().join(" "))
}

fn paragraph(out: &mut String, label: &str, value: &str) {
    if !value.trim().is_empty() {
        out.push_str(&format!("**{}:** {}\n\n", label, markdown_text(value)));
    }
}

fn location(out: &mut String, label: &str, value: &provider_local::security::SecurityLocation) {
    let line = value
        .line
        .map(|line| format!(":{line}"))
        .unwrap_or_default();
    out.push_str(&format!(
        "**{label}:** `{}`{} - {}\n\n",
        value.path.replace('`', "'"),
        line,
        markdown_text(&value.description)
    ));
}

fn bullet_section(out: &mut String, label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    out.push_str(&format!("\n**{label}**\n\n"));
    for value in values {
        out.push_str(&format!("- {}\n", markdown_text(value)));
    }
    out.push('\n');
}

fn markdown_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('#', "\\#")
        .replace('\n', " ")
        .trim()
        .to_string()
}

fn table_text(value: &str) -> String {
    markdown_text(value).replace('|', "\\|")
}

#[cfg(test)]
#[path = "security_report_tests.rs"]
mod tests;
