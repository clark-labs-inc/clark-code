use std::path::Path;

use serde_json::json;

use super::*;

fn summary() -> SecurityScanReportSummary {
    SecurityScanReportSummary {
        id: "scan-123".into(),
        repository_id: "repository-456".into(),
        mode: "deep".into(),
        model: "security-model".into(),
        status: "completed".into(),
        created_at: "2026-08-04T12:00:00Z".into(),
        generated_at: "2026-08-04T12:05:00Z".into(),
    }
}

fn local_record() -> SecurityScanRecord {
    serde_json::from_value(json!({
        "path": ".agent/security-scans/local-123/scan.json",
        "modifiedAtMs": 1,
        "bundle": {
            "contractVersion": 2,
            "scanId": "local-123",
            "mode": "deep",
            "model": "security-model",
            "scope": ".",
            "inventoryId": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "phase": "reporting",
            "threatModel": {
                "assets": ["Tenant credentials", "Private repository source"],
                "trustBoundaries": ["Public API to repository worker"],
                "attackerInputs": ["Tenant-controlled destination URL"],
                "invariants": ["Tenant input cannot select internal services"]
            },
            "coverage": [
                {"path": "src/routes.rs", "status": "reviewed"},
                {"path": "vendor/generated.rs", "status": "excluded", "reason": "Generated vendor code"}
            ],
            "supportingCoverage": [
                {"path": "SECURITY.md", "status": "reviewed"}
            ],
            "candidates": [{
                "candidateId": "ssrf-1",
                "ruleId": "server-side-request-forgery.http-client",
                "identityAnchor": "fetch-route",
                "title": "Tenant URL reaches an unrestricted HTTP client",
                "summary": "A tenant-controlled destination crosses the service-network boundary without a canonical allowlist check.",
                "category": "server-side-request-forgery",
                "cwe": ["CWE-918"],
                "severity": "high",
                "confidence": "high",
                "source": {"path": "src/routes.rs", "line": 42, "description": "Tenant-controlled request destination"},
                "control": {"path": "src/policy.rs", "line": 18, "description": "Policy does not canonicalize the destination"},
                "sink": {"path": "src/client.rs", "line": 77, "description": "Server-side HTTP request"},
                "impact": "A tenant can probe services that should be unreachable from the public API.",
                "remediation": "Canonicalize destinations and enforce an explicit scheme, host, port, and resolved-address allowlist.",
                "validation": {
                    "disposition": "reportable",
                    "evidence": "A contained positive probe reached the test-only internal listener; the negative control was blocked.",
                    "counterevidence": ["Authentication is required", "The HTTP client has a request timeout"]
                },
                "poc": {
                    "goal": "Compare attacker-controlled and allowlisted destinations",
                    "outcome": "reproduced",
                    "positiveReceiptId": "receipt-positive",
                    "negativeReceiptId": "receipt-negative",
                    "limitations": ["Validation used the managed disposable lab only"]
                },
                "attackPath": {
                    "attacker": "Tenant user",
                    "entrypoint": "POST /fetch",
                    "preconditions": ["Authenticated tenant account"],
                    "path": ["request destination", "repository worker", "HTTP client", "internal service"],
                    "likelihood": "High when the worker can route to private services"
                }
            }]
        },
        "seal": {
            "contractVersion": 2,
            "scanId": "local-123",
            "model": "security-model",
            "scope": ".",
            "inventoryId": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "deepPasses": 3,
            "bundleDigest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "reviewedFiles": 1,
            "excludedFiles": 1,
            "supportingFiles": 1,
            "candidateCount": 1,
            "pocAttemptedCount": 2,
            "pocReproducedCount": 1,
            "findings": [{
                "findingId": "finding-1",
                "fingerprint": "fingerprint-1",
                "candidateId": "ssrf-1",
                "severity": "high",
                "sourcePath": "src/routes.rs",
                "impact": "Internal service access",
                "pocOutcome": "reproduced",
                "positiveReceiptId": "receipt-positive",
                "negativeReceiptId": "receipt-negative"
            }]
        },
        "pocReceipts": []
    }))
    .expect("valid local Security report fixture")
}

#[test]
fn cloud_only_report_is_explicit_about_missing_local_evidence() {
    let markdown = build_report(&summary(), None);
    assert!(markdown.contains("does not infer findings or coverage"));
    assert!(markdown.contains("| Scan ID | scan-123 |"));
}

#[test]
fn local_report_contains_findings_coverage_and_remediation() {
    let record = local_record();
    let markdown = build_report(&summary(), Some(&record));
    assert!(markdown.contains("## Validated findings"));
    assert!(markdown.contains("Tenant URL reaches an unrestricted HTTP client"));
    assert!(markdown.contains("## Coverage and integrity"));
    assert!(markdown.contains("Canonicalize destinations"));
}

#[test]
fn report_renders_as_a_tagged_pdf() {
    let record = local_record();
    let markdown = build_report(&summary(), Some(&record));
    let pdf = render_markdown_pdf(Path::new("security-report.md"), markdown.as_bytes())
        .expect("render security report");
    assert!(pdf.starts_with(b"%PDF-"));
    let body = String::from_utf8_lossy(&pdf);
    assert!(body.contains("/StructTreeRoot"));
    assert!(body.contains("/S /H1"));
    if let Ok(path) = std::env::var("AGENT_SECURITY_REPORT_TEST_OUTPUT") {
        std::fs::write(path, &pdf).expect("write requested visual QA artifact");
    }
}
