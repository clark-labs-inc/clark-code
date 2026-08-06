use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::SigningKey;
use scout_capsule_core::{
    normalize_json, CapsuleLimits, CAPSULE_ABI_VERSION, REQUEST_SCHEMA, RESPONSE_SCHEMA,
};
use scout_capsule_host::{
    dispatch, module_sha256, CapsuleHost, CapsuleHostLimits, CapsulePolicyBinding,
    CapsuleRegistryEntry, CapsuleRegistryPayload, CapsuleServiceRequest, CapsuleServiceResponse,
    InvokeCapsuleRequest, SignedCapsuleRegistry, CAPSULE_HOST_ABI_VERSION,
    CAPSULE_SERVICE_PROTOCOL_VERSION, SERVICE_NAME,
};
use serde_json::json;

fn main() -> Result<(), String> {
    let mut args = env::args_os().skip(1);
    let module_path = PathBuf::from(
        args.next()
            .ok_or("usage: qualify_guest <module.wasm> <receipt.json>")?,
    );
    let receipt_path = PathBuf::from(
        args.next()
            .ok_or("usage: qualify_guest <module.wasm> <receipt.json>")?,
    );
    if args.next().is_some() {
        return Err("usage: qualify_guest <module.wasm> <receipt.json>".into());
    }

    let module = fs::read(&module_path).map_err(|_| "could not read capsule module")?;
    let module_digest = module_sha256(&module);
    let input = serde_json::to_vec(&json!({
        "abi_version": CAPSULE_ABI_VERSION,
        "schema": REQUEST_SCHEMA,
        "adapter_id": "clark/qualification@1",
        "provider_namespace": "qualification",
        "records": [{
            "provider_type": "qualification.fixture",
            "identity_authority_scope": "offline",
            "native_id": "fixture-1",
            "semantic_kind": "test_fixture",
            "labels": ["portable", "portable", "bounded"],
            "fields": [{
                "name": "passed",
                "value": {"kind": "boolean", "value": true}
            }],
            "links": []
        }]
    }))
    .map_err(|_| "could not encode qualification input")?;
    let native = normalize_json(&input, CapsuleLimits::default())
        .map_err(|_| "native normalization rejected qualification input")?;
    let invocation = CapsuleHost::new([module_digest.clone()], CapsuleHostLimits::default())
        .map_err(|error| error.to_string())?
        .invoke(&module, &input)
        .map_err(|error| error.to_string())?;
    if invocation.output != native {
        return Err("guest output differs from the native normalization oracle".into());
    }
    let service_root = tempfile::tempdir().map_err(|_| "could not create service fixture")?;
    fs::create_dir(service_root.path().join("modules"))
        .map_err(|_| "could not create service module store")?;
    fs::write(
        service_root
            .path()
            .join("modules")
            .join(format!("{module_digest}.wasm")),
        &module,
    )
    .map_err(|_| "could not install service module fixture")?;
    let admin = SigningKey::from_bytes(&[0x5a; 32]);
    let target_identity_sha256 = "b".repeat(64);
    let registry = SignedCapsuleRegistry::sign(
        CapsuleRegistryPayload {
            schema: "scout-capsule-registry-v1".into(),
            generation: 7,
            not_before_ms: 1,
            expires_at_ms: u64::MAX,
            target_id: "qualification-target".into(),
            target_identity_sha256: target_identity_sha256.clone(),
            entries: BTreeMap::from([(
                "normalize-page".into(),
                CapsuleRegistryEntry {
                    module_sha256: module_digest.clone(),
                    abi_version: CAPSULE_HOST_ABI_VERSION,
                    tenant_ids: BTreeSet::from(["qualification-tenant".into()]),
                    enterprise_ids: BTreeSet::from(["qualification-enterprise".into()]),
                    input_schema: REQUEST_SCHEMA.into(),
                    output_schema: RESPONSE_SCHEMA.into(),
                    limits: CapsuleHostLimits::default(),
                },
            )]),
        },
        &admin,
    )?;
    fs::write(
        service_root.path().join("registry-v1.json"),
        serde_json::to_vec(&registry).map_err(|_| "could not encode service registry")?,
    )
    .map_err(|_| "could not install service registry fixture")?;
    let policy = CapsulePolicyBinding {
        protocol_version: CAPSULE_SERVICE_PROTOCOL_VERSION,
        authorized_tenant_id: "qualification-tenant".into(),
        trusted_admin_key_sha256: module_sha256(admin.verifying_key().as_bytes()),
        minimum_registry_generation: 7,
        target_id: "qualification-target".into(),
        target_identity_sha256,
    };
    let service_request = CapsuleServiceRequest::Invoke(InvokeCapsuleRequest {
        policy,
        capsule_id: "normalize-page".into(),
        enterprise_id: "qualification-enterprise".into(),
        input_schema: REQUEST_SCHEMA.into(),
        output_schema: RESPONSE_SCHEMA.into(),
        input_base64: STANDARD.encode(&input),
    });
    let canonical_service_root = fs::canonicalize(service_root.path())
        .map_err(|_| "could not canonicalize service fixture")?;
    let service_response = dispatch(
        SERVICE_NAME,
        &canonical_service_root,
        &serde_json::to_vec(&service_request)
            .map_err(|_| "could not encode service qualification request")?,
    )?;
    let service_response: CapsuleServiceResponse = serde_json::from_slice(&service_response)
        .map_err(|_| "could not decode service qualification response")?;
    let (
        registry_sha256,
        generation,
        service_output,
        service_isolation,
        deadline_is_hard_interrupt,
    ) = match service_response {
        CapsuleServiceResponse::Invoked {
            registry_sha256,
            generation,
            output_base64,
            isolation,
            deadline_is_hard_interrupt,
            ..
        } => (
            registry_sha256,
            generation,
            STANDARD
                .decode(output_base64)
                .map_err(|_| "service qualification output encoding is invalid")?,
            isolation,
            deadline_is_hard_interrupt,
        ),
        CapsuleServiceResponse::Failed { message, .. } => {
            return Err(format!("signed service qualification failed: {message}"))
        }
        CapsuleServiceResponse::Census { .. } => {
            return Err("signed service qualification returned a census".into())
        }
    };
    if service_output != native {
        return Err("signed service output differs from the native normalization oracle".into());
    }

    let receipt = json!({
        "schema": "scout-capsule-host-qualification-v1",
        "guest_output_matches_native": true,
        "isolation": invocation.receipt,
        "signed_service": {
            "registry_sha256": registry_sha256,
            "generation": generation,
            "output_matches_native": true,
            "isolation": service_isolation,
            "deadline_is_hard_interrupt": deadline_is_hard_interrupt
        }
    });
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|_| "could not encode qualification receipt")?;
    fs::write(receipt_path, bytes).map_err(|_| "could not write qualification receipt")?;
    Ok(())
}
