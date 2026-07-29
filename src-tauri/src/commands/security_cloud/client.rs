use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

use super::model::{
    ArtifactRecord, ArtifactSpec, ArtifactUploadGrant, PlatformScan, PlatformSealResult,
    PlatformTaskMutation, ScannerEnrollment,
};

const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;

pub(super) struct ClarkSecurityPlatformClient {
    base: String,
    api_key: String,
    http: reqwest::Client,
}

impl ClarkSecurityPlatformClient {
    pub(super) fn new(
        base: String,
        api_key: String,
        http: reqwest::Client,
    ) -> Result<Self, String> {
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            return Err("Clark Code API key is unavailable for Security evidence sync".into());
        }
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            api_key,
            http,
        })
    }

    pub(super) async fn enroll_scanner(
        &self,
        organization_id: &str,
        public_key: &str,
        kind: &str,
        display_name: &str,
    ) -> Result<ScannerEnrollment, String> {
        let enrollment = self
            .post_json(
                "/v1/security/scanners/enroll",
                &json!({
                    "organizationId": organization_id,
                    "publicKey": public_key,
                    "kind": kind,
                    "displayName": display_name,
                }),
                "Clark Security scanner enrollment",
            )
            .await?;
        let enrollment: ScannerEnrollment = serde_json::from_value(enrollment)
            .map_err(|error| format!("Clark Security scanner enrollment response: {error}"))?;
        if enrollment.organization_id != organization_id
            || enrollment.public_key != public_key
            || enrollment.kind != kind
            || enrollment.display_name != display_name
        {
            return Err(
                "Clark Security scanner enrollment did not match the device identity".into(),
            );
        }
        Ok(enrollment)
    }

    pub(super) async fn create_scan(&self, body: &Value) -> Result<PlatformScan, String> {
        self.post_typed(
            "/v1/security/scan-runs",
            body,
            "Clark Security scan creation",
        )
        .await
    }

    pub(super) async fn get_scan(
        &self,
        organization_id: &str,
        scan_id: &str,
    ) -> Result<PlatformScan, String> {
        let path = format!("/v1/security/scan-runs/{scan_id}?organizationId={organization_id}");
        self.get_typed(&path, "Clark Security scan status").await
    }

    pub(super) async fn claim_task(
        &self,
        organization_id: &str,
        repository_id: &str,
        scan_id: &str,
        scanner_id: &str,
        task_kind: &str,
    ) -> Result<Option<PlatformTaskMutation>, String> {
        let response = self
            .request(
                self.http
                    .post(format!("{}/v1/security/tasks/claim", self.base))
                    .bearer_auth(&self.api_key)
                    .json(&json!({
                        "organizationId": organization_id,
                        "scannerId": scanner_id,
                        "repositoryId": repository_id,
                        "scanId": scan_id,
                        "supportedKinds": [task_kind],
                        "leaseSeconds": 3_600,
                    })),
                "Clark Security task claim",
            )
            .await?;
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let mutation = decode_success(response, "Clark Security task claim").await?;
        let mutation: PlatformTaskMutation = serde_json::from_value(mutation)
            .map_err(|error| format!("Clark Security task claim response: {error}"))?;
        validate_task_binding(
            &mutation,
            organization_id,
            repository_id,
            scan_id,
            scanner_id,
            task_kind,
        )?;
        Ok(Some(mutation))
    }

    pub(super) async fn complete_task(
        &self,
        organization_id: &str,
        repository_id: &str,
        scan_id: &str,
        scanner_id: &str,
        task: &super::model::PlatformTask,
        result: Value,
    ) -> Result<PlatformTaskMutation, String> {
        let path = format!("/v1/security/tasks/{}/complete", task.id);
        let mutation: PlatformTaskMutation = self
            .post_typed(
                &path,
                &json!({
                    "organizationId": organization_id,
                    "repositoryId": repository_id,
                    "scanId": scan_id,
                    "taskId": task.id,
                    "scannerId": scanner_id,
                    "leaseFence": task.lease_fence,
                    "disposition": "succeeded",
                    "detail": null,
                    "result": result,
                }),
                "Clark Security task completion",
            )
            .await?;
        if mutation.task.id != task.id
            || mutation.task.lease_fence != task.lease_fence
            || mutation.task.scan_id != scan_id
            || mutation.scan.id != scan_id
        {
            return Err("Clark Security task completion returned a different task or scan".into());
        }
        Ok(mutation)
    }

    pub(super) async fn upload_artifact(
        &self,
        organization_id: &str,
        repository_id: &str,
        scan_id: &str,
        artifact: &ArtifactSpec,
    ) -> Result<ArtifactRecord, String> {
        if artifact.bytes.is_empty() {
            return Err(format!(
                "Clark Security {} artifact must not be empty",
                artifact.role
            ));
        }
        let sha256 = format!("sha256:{}", sha256_hex(&artifact.bytes));
        let identity = sha256_hex(
            format!("clark-security-artifact/v1\0{}\0{sha256}", artifact.role).as_bytes(),
        );
        let client_artifact_id = format!("artifact:{identity}");
        let upload_request_id = format!(
            "security-upload:{}",
            sha256_hex(
                format!("{organization_id}\0{repository_id}\0{scan_id}\0{identity}").as_bytes()
            )
        );
        let path = format!("/v1/security/scan-runs/{scan_id}/artifact-uploads");
        let grant: ArtifactUploadGrant = self
            .post_typed(
                &path,
                &json!({
                    "organizationId": organization_id,
                    "repositoryId": repository_id,
                    "scanId": scan_id,
                    "clientArtifactId": client_artifact_id,
                    "role": artifact.role,
                    "storageTier": artifact.storage_tier,
                    "classification": artifact.classification,
                    "uploadRequestId": upload_request_id,
                    "contentType": artifact.content_type,
                    "sizeBytes": artifact.bytes.len(),
                    "sha256": sha256,
                }),
                "Clark Security artifact authorization",
            )
            .await?;
        validate_artifact_authorization(
            &grant,
            organization_id,
            repository_id,
            scan_id,
            artifact,
            &client_artifact_id,
            &sha256,
        )?;
        if grant.authorization.status == "verified" {
            let object_version_id =
                grant
                    .authorization
                    .object_version_id
                    .clone()
                    .ok_or_else(|| {
                        "verified Clark Security artifact has no object version".to_string()
                    })?;
            return Ok(artifact_record_from_authorization(
                &grant.authorization,
                object_version_id,
            ));
        }
        if grant.authorization.status != "pending" {
            return Err("Clark Security artifact authorization is not uploadable".into());
        }
        upload_presigned(&self.http, &grant, &artifact.bytes).await?;

        let commit_request_id = format!(
            "security-commit:{}",
            sha256_hex(format!("{}\0{sha256}", grant.authorization.id).as_bytes())
        );
        let commit_path = format!("/v1/security/scan-runs/{scan_id}/artifact-commits");
        let record: ArtifactRecord = self
            .post_typed(
                &commit_path,
                &json!({
                    "organizationId": organization_id,
                    "repositoryId": repository_id,
                    "scanId": scan_id,
                    "artifactId": grant.authorization.id,
                    "commitRequestId": commit_request_id,
                }),
                "Clark Security artifact commit",
            )
            .await?;
        validate_artifact_record(&record, scan_id, artifact, &client_artifact_id, &sha256)?;
        Ok(record)
    }

    pub(super) async fn seal_scan(
        &self,
        scan_id: &str,
        body: &Value,
    ) -> Result<PlatformSealResult, String> {
        self.post_typed(
            &format!("/v1/security/scan-runs/{scan_id}/seal"),
            body,
            "Clark Security scan seal",
        )
        .await
    }

    async fn get_typed<T: DeserializeOwned>(&self, path: &str, what: &str) -> Result<T, String> {
        let response = self
            .request(
                self.http
                    .get(format!("{}{}", self.base, path))
                    .bearer_auth(&self.api_key),
                what,
            )
            .await?;
        let value = decode_success(response, what).await?;
        serde_json::from_value(value).map_err(|error| format!("{what} response: {error}"))
    }

    async fn post_typed<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
        what: &str,
    ) -> Result<T, String> {
        let value = self.post_json(path, body, what).await?;
        serde_json::from_value(value).map_err(|error| format!("{what} response: {error}"))
    }

    async fn post_json(&self, path: &str, body: &Value, what: &str) -> Result<Value, String> {
        let response = self
            .request(
                self.http
                    .post(format!("{}{}", self.base, path))
                    .bearer_auth(&self.api_key)
                    .json(body),
                what,
            )
            .await?;
        decode_success(response, what).await
    }

    async fn request(
        &self,
        request: reqwest::RequestBuilder,
        what: &str,
    ) -> Result<reqwest::Response, String> {
        request
            .send()
            .await
            .map_err(|error| format!("{what} request failed: {error}"))
    }
}

fn validate_task_binding(
    mutation: &PlatformTaskMutation,
    organization_id: &str,
    repository_id: &str,
    scan_id: &str,
    scanner_id: &str,
    task_kind: &str,
) -> Result<(), String> {
    let task = &mutation.task;
    if task.organization_id != organization_id
        || task.repository_id != repository_id
        || task.scan_id != scan_id
        || task.task_kind != task_kind
        || mutation.scan.id != scan_id
        || mutation.scan.organization_id != organization_id
        || mutation.scan.repository_id != repository_id
    {
        return Err("Clark Security task claim crossed its requested scan binding".into());
    }
    if task.lease_fence <= 0 {
        return Err("Clark Security task claim returned an invalid lease fence".into());
    }
    let _ = scanner_id;
    Ok(())
}

fn validate_artifact_authorization(
    grant: &ArtifactUploadGrant,
    organization_id: &str,
    repository_id: &str,
    scan_id: &str,
    artifact: &ArtifactSpec,
    client_artifact_id: &str,
    sha256: &str,
) -> Result<(), String> {
    let authorization = &grant.authorization;
    if authorization.organization_id != organization_id
        || authorization.repository_id != repository_id
        || authorization.scan_id != scan_id
        || authorization.client_artifact_id != client_artifact_id
        || authorization.role != artifact.role
        || authorization.storage_tier != artifact.storage_tier
        || authorization.classification != artifact.classification
        || authorization.content_type != artifact.content_type
        || authorization.size_bytes != artifact.bytes.len() as u64
        || authorization.sha256 != sha256
    {
        return Err("Clark Security artifact authorization changed the requested evidence".into());
    }
    match authorization.status.as_str() {
        "pending" if grant.upload_url.is_some() => Ok(()),
        "verified" if grant.upload_url.is_none() && grant.upload_headers.is_empty() => Ok(()),
        _ => Err("Clark Security artifact authorization has an invalid lifecycle".into()),
    }
}

fn artifact_record_from_authorization(
    authorization: &super::model::ArtifactAuthorization,
    object_version_id: String,
) -> ArtifactRecord {
    ArtifactRecord {
        id: authorization.id.clone(),
        scan_id: authorization.scan_id.clone(),
        client_artifact_id: authorization.client_artifact_id.clone(),
        role: authorization.role.clone(),
        storage_tier: authorization.storage_tier.clone(),
        classification: authorization.classification.clone(),
        object_version_id,
        size_bytes: authorization.size_bytes,
        sha256: authorization.sha256.clone(),
    }
}

fn validate_artifact_record(
    record: &ArtifactRecord,
    scan_id: &str,
    artifact: &ArtifactSpec,
    client_artifact_id: &str,
    sha256: &str,
) -> Result<(), String> {
    if record.scan_id != scan_id
        || record.client_artifact_id != client_artifact_id
        || record.role != artifact.role
        || record.storage_tier != artifact.storage_tier
        || record.classification != artifact.classification
        || record.size_bytes != artifact.bytes.len() as u64
        || record.sha256 != sha256
        || record.object_version_id.trim().is_empty()
    {
        return Err("Clark Security artifact commit did not match the uploaded evidence".into());
    }
    Ok(())
}

async fn upload_presigned(
    http: &reqwest::Client,
    grant: &ArtifactUploadGrant,
    bytes: &[u8],
) -> Result<(), String> {
    let raw_url = grant
        .upload_url
        .as_deref()
        .ok_or_else(|| "Clark Security artifact authorization has no upload URL".to_string())?;
    let url = reqwest::Url::parse(raw_url)
        .map_err(|_| "Clark Security returned an invalid artifact upload URL".to_string())?;
    validate_upload_url(&url)?;
    let mut headers = HeaderMap::new();
    for header in &grant.upload_headers {
        let name = HeaderName::from_bytes(header.name.as_bytes())
            .map_err(|_| "Clark Security returned an invalid upload header".to_string())?;
        if matches!(
            name.as_str(),
            "authorization" | "cookie" | "proxy-authorization" | "host"
        ) {
            return Err("Clark Security returned a forbidden upload header".into());
        }
        let value = HeaderValue::from_str(&header.value)
            .map_err(|_| "Clark Security returned an invalid upload header".to_string())?;
        headers.insert(name, value);
    }
    let response = http
        .put(url)
        .headers(headers)
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(|error| format!("Clark Security artifact upload failed: {error}"))?;
    if !response.status().is_success() {
        return Err(http_error(response, "Clark Security artifact upload").await);
    }
    Ok(())
}

fn validate_upload_url(url: &reqwest::Url) -> Result<(), String> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Clark Security artifact upload URL contains credentials".into());
    }
    let loopback = url
        .host_str()
        .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !(cfg!(any(debug_assertions, test)) && loopback) {
        return Err("Clark Security artifact uploads require HTTPS".into());
    }
    Ok(())
}

async fn decode_success(response: reqwest::Response, what: &str) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(http_error(response, what).await);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("{what} response failed: {error}"))?;
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("{what} returned invalid JSON: {error}"))
}

async fn http_error(response: reqwest::Response, what: &str) -> String {
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    let body = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_ERROR_BODY_BYTES)]);
    format!("{what} failed ({status}): {body}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
