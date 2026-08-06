use serde::Serialize;
use serde_json::{json, Value};

use crate::security::{SecurityConfidence, SecurityPocOutcome, SecurityScanMode, SecuritySeverity};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarkSecurityCloudIdentity {
    pub client_scan_id: String,
    pub idempotency_key: String,
    pub inventory_id: String,
    pub snapshot_digest: String,
    pub mode: SecurityScanMode,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarkSecurityCloudExport {
    pub identity: ClarkSecurityCloudIdentity,
    pub manifest: Value,
    pub findings: Value,
    pub coverage: Value,
    pub coverage_completeness: String,
    pub coverage_surfaces: Vec<ClarkSecurityCoverageSurfaceDraft>,
    pub occurrences: Vec<ClarkSecurityOccurrenceDraft>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarkSecurityCoverageSurfaceDraft {
    pub surface_id: String,
    pub label: String,
    pub disposition: String,
    pub risk_area: Option<String>,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarkSecurityLocationDraft {
    pub path: String,
    pub start_line: u32,
    pub end_line: Option<u32>,
    pub role: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarkSecurityFindingIdentityDraft {
    pub rule_id: String,
    pub anchor: String,
    pub instance: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarkSecurityOccurrenceDraft {
    pub candidate_id: String,
    pub identity: ClarkSecurityFindingIdentityDraft,
    pub disposition: String,
    pub severity: SecuritySeverity,
    pub confidence: SecurityConfidence,
    pub title: String,
    pub summary: String,
    pub category: String,
    pub cwe: Vec<String>,
    pub root_cause: Value,
    pub attack_path: Value,
    pub remediation: String,
    pub locations: Vec<ClarkSecurityLocationDraft>,
    pub provenance: Value,
    pub coverage_surface_ids: Vec<String>,
    pub poc_outcome: SecurityPocOutcome,
    #[serde(skip)]
    pub local_poc_receipt_ids: Vec<String>,
}

impl ClarkSecurityOccurrenceDraft {
    pub fn wire_value(&self, signed_poc_receipts: Vec<Value>) -> Value {
        json!({
            "candidateId": self.candidate_id,
            "identity": self.identity,
            "disposition": self.disposition,
            "severity": self.severity,
            "confidence": self.confidence,
            "title": self.title,
            "summary": self.summary,
            "category": self.category,
            "cwe": self.cwe,
            "rootCause": self.root_cause,
            "attackPath": self.attack_path,
            "remediation": self.remediation,
            "locations": self.locations,
            "provenance": self.provenance,
            "coverageSurfaceIds": self.coverage_surface_ids,
            "pocOutcome": self.poc_outcome,
            "pocReceipts": signed_poc_receipts,
        })
    }
}
