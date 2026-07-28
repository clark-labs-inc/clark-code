use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::Value;

use crate::{
    module_sha256, CapsuleDescriptor, CapsuleHost, CapsuleServiceRequest, CapsuleServiceResponse,
    InvokeCapsuleRequest, SignedCapsuleRegistry, CAPSULE_SERVICE_PROTOCOL_VERSION, SERVICE_NAME,
};

const MAX_SERVICE_REQUEST_BYTES: usize = 12 * 1024 * 1024;
const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;
const MAX_CACHED_HOSTS: usize = 64;

static HOSTS: OnceLock<Mutex<BTreeMap<String, CapsuleHost>>> = OnceLock::new();

pub fn dispatch(service: &str, root: &Path, request: &[u8]) -> Result<Vec<u8>, String> {
    if service != SERVICE_NAME {
        return Err(format!("unsupported Scout capsule service: {service}"));
    }
    if request.len() > MAX_SERVICE_REQUEST_BYTES {
        return Err("Scout capsule service request exceeds its byte limit".into());
    }
    let request: CapsuleServiceRequest =
        serde_json::from_slice(request).map_err(|_| "invalid Scout capsule request".to_string())?;
    let response = match handle(root, request) {
        Ok(response) => response,
        Err(message) => CapsuleServiceResponse::Failed {
            code: "policy_or_invocation_rejected".into(),
            message,
        },
    };
    serde_json::to_vec(&response).map_err(|_| "Scout capsule response encoding failed".into())
}

fn handle(root: &Path, request: CapsuleServiceRequest) -> Result<CapsuleServiceResponse, String> {
    let policy = match &request {
        CapsuleServiceRequest::Census(request) => request.policy.clone(),
        CapsuleServiceRequest::Invoke(request) => request.policy.clone(),
    };
    if policy.protocol_version != CAPSULE_SERVICE_PROTOCOL_VERSION {
        return Err("capsule service protocol version is unsupported".into());
    }
    let (registry, registry_sha256, canonical_root) = load_registry(root, &policy)?;
    if registry.payload.target_id != policy.target_id
        || registry.payload.target_identity_sha256 != policy.target_identity_sha256
    {
        return Err("capsule registry is bound to another execution target".into());
    }
    match request {
        CapsuleServiceRequest::Census(request) => Ok(CapsuleServiceResponse::Census {
            registry_sha256,
            generation: registry.payload.generation,
            capsules: registry
                .payload
                .entries
                .into_iter()
                .filter(|(_, entry)| {
                    entry.tenant_ids.contains(&policy.authorized_tenant_id)
                        && entry.enterprise_ids.contains(&request.enterprise_id)
                })
                .map(|(capsule_id, entry)| CapsuleDescriptor {
                    capsule_id,
                    abi_version: entry.abi_version,
                    input_schema: entry.input_schema,
                    output_schema: entry.output_schema,
                })
                .collect(),
        }),
        CapsuleServiceRequest::Invoke(request) => {
            invoke(canonical_root.as_path(), registry, registry_sha256, request)
        }
    }
}

fn load_registry(
    root: &Path,
    policy: &crate::CapsulePolicyBinding,
) -> Result<(SignedCapsuleRegistry, String, std::path::PathBuf), String> {
    reject_symlink_components(root)?;
    require_directory(root, "capsule registry root")?;
    let canonical_root =
        fs::canonicalize(root).map_err(|_| "capsule registry root is unavailable".to_string())?;
    let path = canonical_root.join("registry-v1.json");
    reject_symlink(&path)?;
    require_regular_file(&path, "capsule registry")?;
    let metadata =
        fs::metadata(&path).map_err(|_| "capsule registry is unavailable".to_string())?;
    if metadata.len() > MAX_REGISTRY_BYTES {
        return Err("capsule registry exceeds its byte limit".into());
    }
    let bytes = fs::read(path).map_err(|_| "capsule registry is unavailable".to_string())?;
    let registry: SignedCapsuleRegistry =
        serde_json::from_slice(&bytes).map_err(|_| "capsule registry is invalid".to_string())?;
    let registry_sha256 = registry.verify(
        &policy.trusted_admin_key_sha256,
        policy.minimum_registry_generation,
    )?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "host clock is before the Unix epoch".to_string())?
        .as_millis();
    let now_ms = u64::try_from(now_ms).map_err(|_| "host clock is out of range".to_string())?;
    if now_ms < registry.payload.not_before_ms {
        return Err("capsule registry is not yet valid".into());
    }
    if now_ms >= registry.payload.expires_at_ms {
        return Err("capsule registry has expired".into());
    }
    Ok((registry, registry_sha256, canonical_root))
}

fn invoke(
    root: &Path,
    registry: SignedCapsuleRegistry,
    registry_sha256: String,
    request: InvokeCapsuleRequest,
) -> Result<CapsuleServiceResponse, String> {
    let entry = registry
        .payload
        .entries
        .get(&request.capsule_id)
        .ok_or_else(|| "capsule is not registered".to_string())?;
    if !entry
        .tenant_ids
        .contains(&request.policy.authorized_tenant_id)
        || !entry.enterprise_ids.contains(&request.enterprise_id)
        || entry.input_schema != request.input_schema
        || entry.output_schema != request.output_schema
    {
        return Err("capsule invocation is outside its signed authority binding".into());
    }
    let modules_path = root.join("modules");
    reject_symlink(&modules_path)?;
    require_directory(&modules_path, "capsule module store")?;
    let module_path = modules_path.join(format!("{}.wasm", entry.module_sha256));
    reject_symlink(&module_path)?;
    require_regular_file(&module_path, "registered capsule module")?;
    let canonical_module = fs::canonicalize(&module_path)
        .map_err(|_| "registered capsule module is unavailable".to_string())?;
    let canonical_modules = fs::canonicalize(root.join("modules"))
        .map_err(|_| "capsule module directory is unavailable".to_string())?;
    if !canonical_module.starts_with(&canonical_modules) {
        return Err("registered capsule module escapes its content store".into());
    }
    let module = fs::read(canonical_module)
        .map_err(|_| "registered capsule module is unavailable".to_string())?;
    if module_sha256(&module) != entry.module_sha256 {
        return Err("registered capsule module digest does not match".into());
    }
    let input = STANDARD
        .decode(&request.input_base64)
        .map_err(|_| "capsule input encoding is invalid")?;
    validate_schema(&input, &request.input_schema, "input")?;
    let cache_key = format!(
        "{}:{}:{}",
        root.display(),
        registry_sha256,
        request.capsule_id
    );
    let host = cached_host(cache_key, &entry.module_sha256, entry.limits)?;
    let invocation = host.invoke(&module, &input).map_err(|error| match error {
        crate::CapsuleHostError::DeadlineExceeded => {
            "capsule observation deadline elapsed; the worker retains its slot until finite fuel termination".to_string()
        }
        other => other.to_string(),
    })?;
    validate_schema(&invocation.output, &request.output_schema, "output")?;
    Ok(CapsuleServiceResponse::Invoked {
        registry_sha256,
        generation: registry.payload.generation,
        capsule_id: request.capsule_id,
        enterprise_id: request.enterprise_id,
        target_id: request.policy.target_id,
        output_base64: STANDARD.encode(invocation.output),
        isolation: Box::new(invocation.receipt),
        deadline_is_hard_interrupt: false,
    })
}

fn cached_host(
    key: String,
    digest: &str,
    limits: crate::CapsuleHostLimits,
) -> Result<CapsuleHost, String> {
    let hosts = HOSTS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut hosts = hosts
        .lock()
        .map_err(|_| "capsule host cache failed".to_string())?;
    if let Some(host) = hosts.get(&key) {
        return Ok(host.clone());
    }
    if hosts.len() >= MAX_CACHED_HOSTS {
        hosts.clear();
    }
    let host = CapsuleHost::new([digest.to_owned()], limits).map_err(|error| error.to_string())?;
    hosts.insert(key, host.clone());
    Ok(host)
}

fn validate_schema(bytes: &[u8], expected_schema: &str, kind: &str) -> Result<(), String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| format!("capsule {kind} is not valid JSON"))?;
    if value.get("schema").and_then(Value::as_str) != Some(expected_schema) {
        return Err(format!(
            "capsule {kind} schema does not match its signed registry entry"
        ));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "capsule file is unavailable".to_string())?;
    if metadata.file_type().is_symlink() {
        Err("capsule service refuses symbolic links".into())
    } else {
        Ok(())
    }
}

fn reject_symlink_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err("capsule service refuses symbolic-link path components".into());
        }
    }
    Ok(())
}

fn require_regular_file(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| format!("{label} is unavailable"))?;
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(format!("{label} is not a regular file"))
    }
}

fn require_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| format!("{label} is unavailable"))?;
    if metadata.file_type().is_dir() {
        Ok(())
    } else {
        Err(format!("{label} is not a directory"))
    }
}
