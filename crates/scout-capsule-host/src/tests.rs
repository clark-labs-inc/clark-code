use std::collections::{BTreeMap, BTreeSet};

use crate::{
    dispatch, module_sha256, CapsuleHost, CapsuleHostError, CapsuleHostLimits,
    CapsulePolicyBinding, CapsuleRegistryEntry, CapsuleRegistryPayload, CapsuleServiceRequest,
    CapsuleServiceResponse, CensusCapsuleRequest, InvokeCapsuleRequest, SignedCapsuleRegistry,
    CAPSULE_HOST_ABI_VERSION, CAPSULE_HOST_RUNTIME, CAPSULE_ISOLATION_RECEIPT_SCHEMA,
    CAPSULE_SERVICE_PROTOCOL_VERSION, SERVICE_NAME,
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::SigningKey;

fn compile(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).unwrap()
}

fn echo_module() -> Vec<u8> {
    compile(
        r#"
        (module
          (memory (export "memory") 1 16)
          (global $next (mut i32) (i32.const 1024))
          (func (export "scout_alloc") (param $length i32) (result i32)
            (local $pointer i32)
            (local.set $pointer (global.get $next))
            (global.set $next (i32.add (global.get $next) (local.get $length)))
            (local.get $pointer))
          (func (export "scout_run") (param $pointer i32) (param $length i32) (result i64)
            (i64.or
              (i64.extend_i32_u (local.get $pointer))
              (i64.shl (i64.extend_i32_u (local.get $length)) (i64.const 32)))))
        "#,
    )
}

fn host(module: &[u8], limits: CapsuleHostLimits) -> CapsuleHost {
    CapsuleHost::new([module_sha256(module)], limits).unwrap()
}

#[test]
fn approved_zero_import_module_runs_in_fresh_deterministic_instances() {
    let module = echo_module();
    let host = host(&module, CapsuleHostLimits::default());
    let input = br#"{"fixture":"bounded"}"#;

    let first = host.invoke(&module, input).unwrap();
    let second = host.invoke(&module, input).unwrap();

    assert_eq!(first.output, input);
    assert_eq!(second.output, input);
    assert_eq!(first.receipt.output_sha256, second.receipt.output_sha256);
    assert_eq!(first.receipt.schema, CAPSULE_ISOLATION_RECEIPT_SCHEMA);
    assert_eq!(first.receipt.abi_version, CAPSULE_HOST_ABI_VERSION);
    assert_eq!(first.receipt.runtime, CAPSULE_HOST_RUNTIME);
    assert_eq!(first.receipt.module_sha256, module_sha256(&module));
    assert!(first.receipt.import_set.is_empty());
    assert!(first.receipt.fresh_instance);
    assert!(!first.receipt.wasi_enabled);
    assert!(first.receipt.fuel_consumed > 0);
}

#[test]
fn approval_is_host_owned_and_digest_exact() {
    let module = echo_module();
    let other = compile("(module)");
    let host = host(&module, CapsuleHostLimits::default());

    assert_eq!(
        host.invoke(&other, b"private-input").unwrap_err(),
        CapsuleHostError::ModuleNotApproved
    );
    assert!(matches!(
        CapsuleHost::new(BTreeSet::new(), CapsuleHostLimits::default()),
        Err(CapsuleHostError::EmptyApprovalPolicy)
    ));
    assert!(matches!(
        CapsuleHost::new(["A".repeat(64)], CapsuleHostLimits::default()),
        Err(CapsuleHostError::InvalidApprovedDigest)
    ));
}

#[test]
fn any_import_is_rejected_before_linking() {
    let module = compile(
        r#"
        (module
          (import "wasi_snapshot_preview1" "clock_time_get"
            (func $clock (param i32 i64 i32) (result i32)))
          (memory (export "memory") 1)
          (func (export "scout_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scout_run") (param i32 i32) (result i64) (i64.const 0)))
        "#,
    );
    let host = host(&module, CapsuleHostLimits::default());

    assert_eq!(
        host.invoke(&module, b"input").unwrap_err(),
        CapsuleHostError::ImportedCapability
    );
}

#[test]
fn deterministic_fuel_stops_an_infinite_guest() {
    let module = compile(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "scout_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scout_run") (param i32 i32) (result i64)
            (loop $forever (br $forever))
            (i64.const 0)))
        "#,
    );
    let limits = CapsuleHostLimits {
        max_fuel: 10_000,
        ..CapsuleHostLimits::default()
    };
    let host = host(&module, limits);

    assert_eq!(
        host.invoke(&module, b"input").unwrap_err(),
        CapsuleHostError::FuelExhausted
    );
}

#[test]
fn wall_clock_deadline_returns_and_retains_the_concurrency_slot_until_fuel_stops() {
    let module = compile(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "scout_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scout_run") (param i32 i32) (result i64)
            (loop $forever (br $forever))
            (i64.const 0)))
        "#,
    );
    let limits = CapsuleHostLimits {
        max_fuel: 20_000_000,
        deadline_ms: 1,
        max_concurrent_instances: 1,
        ..CapsuleHostLimits::default()
    };
    let host = host(&module, limits);

    assert_eq!(
        host.invoke(&module, b"input").unwrap_err(),
        CapsuleHostError::DeadlineExceeded
    );
    assert_eq!(
        host.invoke(&module, b"input").unwrap_err(),
        CapsuleHostError::ConcurrencyLimit
    );
}

#[test]
fn linear_memory_and_output_limits_fail_closed() {
    let grower = compile(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "scout_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scout_run") (param i32 i32) (result i64)
            (drop (memory.grow (i32.const 1)))
            (i64.const 0)))
        "#,
    );
    let limits = CapsuleHostLimits {
        max_linear_memory_bytes: 64 * 1024,
        ..CapsuleHostLimits::default()
    };
    assert_eq!(
        host(&grower, limits).invoke(&grower, b"").unwrap_err(),
        CapsuleHostError::GuestTrap
    );

    let oversized_output = compile(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "scout_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scout_run") (param i32 i32) (result i64)
            (i64.const 8589934592)))
        "#,
    );
    let limits = CapsuleHostLimits {
        max_output_bytes: 1,
        ..CapsuleHostLimits::default()
    };
    assert_eq!(
        host(&oversized_output, limits)
            .invoke(&oversized_output, b"")
            .unwrap_err(),
        CapsuleHostError::OutputTooLarge
    );
}

#[test]
fn malformed_abi_and_untrusted_payloads_never_enter_errors() {
    let module = compile("(module (memory (export \"memory\") 1))");
    let host = host(&module, CapsuleHostLimits::default());
    let private_marker = b"private-customer-marker";

    let error = host.invoke(&module, private_marker).unwrap_err();
    assert_eq!(error, CapsuleHostError::InvalidAbi);
    assert!(!error.to_string().contains("private-customer-marker"));
}

#[test]
fn module_and_input_limits_are_checked_before_execution() {
    let module = echo_module();
    let limits = CapsuleHostLimits {
        max_module_bytes: module.len() - 1,
        ..CapsuleHostLimits::default()
    };
    let bounded_host = CapsuleHost::new([module_sha256(&module)], limits).unwrap();
    assert_eq!(
        bounded_host.invoke(&module, b"").unwrap_err(),
        CapsuleHostError::ModuleTooLarge
    );

    let limits = CapsuleHostLimits {
        max_input_bytes: 1,
        ..CapsuleHostLimits::default()
    };
    assert_eq!(
        host(&module, limits).invoke(&module, b"12").unwrap_err(),
        CapsuleHostError::InputTooLarge
    );
}

struct ServiceFixture {
    root: tempfile::TempDir,
    policy: CapsulePolicyBinding,
    module_path: std::path::PathBuf,
}

fn service_fixture() -> ServiceFixture {
    service_fixture_with_validity(echo_module(), CapsuleHostLimits::default(), 1, u64::MAX)
}

fn service_fixture_with(module: Vec<u8>, limits: CapsuleHostLimits) -> ServiceFixture {
    service_fixture_with_validity(module, limits, 1, u64::MAX)
}

fn service_fixture_with_validity(
    module: Vec<u8>,
    limits: CapsuleHostLimits,
    not_before_ms: u64,
    expires_at_ms: u64,
) -> ServiceFixture {
    let root = tempfile::tempdir().unwrap();
    let modules = root.path().join("modules");
    std::fs::create_dir(&modules).unwrap();
    let module_digest = module_sha256(&module);
    let module_path = modules.join(format!("{module_digest}.wasm"));
    std::fs::write(&module_path, module).unwrap();
    let key = SigningKey::from_bytes(&[7_u8; 32]);
    let target_identity_sha256 = "b".repeat(64);
    let payload = CapsuleRegistryPayload {
        schema: "scout-capsule-registry-v1".into(),
        generation: 4,
        not_before_ms,
        expires_at_ms,
        target_id: "target-a".into(),
        target_identity_sha256: target_identity_sha256.clone(),
        entries: BTreeMap::from([(
            "normalize-page".into(),
            CapsuleRegistryEntry {
                module_sha256: module_digest,
                abi_version: CAPSULE_HOST_ABI_VERSION,
                tenant_ids: BTreeSet::from(["tenant-a".into()]),
                enterprise_ids: BTreeSet::from(["enterprise-a".into()]),
                input_schema: "typed-page-v1".into(),
                output_schema: "typed-page-v1".into(),
                limits,
            },
        )]),
    };
    let registry = SignedCapsuleRegistry::sign(payload, &key).unwrap();
    std::fs::write(
        root.path().join("registry-v1.json"),
        serde_json::to_vec(&registry).unwrap(),
    )
    .unwrap();
    ServiceFixture {
        root,
        policy: CapsulePolicyBinding {
            protocol_version: CAPSULE_SERVICE_PROTOCOL_VERSION,
            authorized_tenant_id: "tenant-a".into(),
            trusted_admin_key_sha256: module_sha256(key.verifying_key().as_bytes()),
            minimum_registry_generation: 4,
            target_id: "target-a".into(),
            target_identity_sha256,
        },
        module_path,
    }
}

fn service_call(root: &std::path::Path, request: CapsuleServiceRequest) -> CapsuleServiceResponse {
    let encoded = serde_json::to_vec(&request).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let response = dispatch(SERVICE_NAME, &root, &encoded).unwrap();
    serde_json::from_slice(&response).unwrap()
}

#[test]
fn signed_service_census_and_invoke_are_exactly_bound() {
    let fixture = service_fixture();
    let census = service_call(
        fixture.root.path(),
        CapsuleServiceRequest::Census(CensusCapsuleRequest {
            policy: fixture.policy.clone(),
            enterprise_id: "enterprise-a".into(),
        }),
    );
    let CapsuleServiceResponse::Census {
        generation,
        capsules,
        ..
    } = census
    else {
        panic!("expected census");
    };
    assert_eq!(generation, 4);
    assert_eq!(capsules.len(), 1);
    assert_eq!(capsules[0].capsule_id, "normalize-page");
    let other_enterprise = service_call(
        fixture.root.path(),
        CapsuleServiceRequest::Census(CensusCapsuleRequest {
            policy: fixture.policy.clone(),
            enterprise_id: "enterprise-b".into(),
        }),
    );
    assert!(matches!(
        other_enterprise,
        CapsuleServiceResponse::Census { capsules, .. } if capsules.is_empty()
    ));

    let input = br#"{"schema":"typed-page-v1","records":[]}"#;
    let response = service_call(
        fixture.root.path(),
        CapsuleServiceRequest::Invoke(InvokeCapsuleRequest {
            policy: fixture.policy,
            capsule_id: "normalize-page".into(),
            enterprise_id: "enterprise-a".into(),
            input_schema: "typed-page-v1".into(),
            output_schema: "typed-page-v1".into(),
            input_base64: STANDARD.encode(input),
        }),
    );
    let CapsuleServiceResponse::Invoked {
        output_base64,
        deadline_is_hard_interrupt,
        ..
    } = response
    else {
        panic!("expected invocation");
    };
    assert_eq!(STANDARD.decode(output_base64).unwrap(), input);
    assert!(!deadline_is_hard_interrupt);
}

#[test]
fn signed_service_rejects_wrong_authority_target_schema_and_tampering() {
    let fixture = service_fixture();
    for policy in [
        {
            let mut policy = fixture.policy.clone();
            policy.trusted_admin_key_sha256 = "c".repeat(64);
            policy
        },
        {
            let mut policy = fixture.policy.clone();
            policy.target_id = "target-b".into();
            policy
        },
        {
            let mut policy = fixture.policy.clone();
            policy.authorized_tenant_id = "tenant-b".into();
            policy
        },
    ] {
        let response = service_call(
            fixture.root.path(),
            CapsuleServiceRequest::Invoke(InvokeCapsuleRequest {
                policy,
                capsule_id: "normalize-page".into(),
                enterprise_id: "enterprise-a".into(),
                input_schema: "typed-page-v1".into(),
                output_schema: "typed-page-v1".into(),
                input_base64: STANDARD.encode(br#"{"schema":"typed-page-v1"}"#),
            }),
        );
        assert!(matches!(response, CapsuleServiceResponse::Failed { .. }));
    }

    let wrong_enterprise = service_call(
        fixture.root.path(),
        CapsuleServiceRequest::Invoke(InvokeCapsuleRequest {
            policy: fixture.policy.clone(),
            capsule_id: "normalize-page".into(),
            enterprise_id: "enterprise-b".into(),
            input_schema: "typed-page-v1".into(),
            output_schema: "typed-page-v1".into(),
            input_base64: STANDARD.encode(br#"{"schema":"typed-page-v1"}"#),
        }),
    );
    assert!(matches!(
        wrong_enterprise,
        CapsuleServiceResponse::Failed { .. }
    ));

    std::fs::write(
        &fixture.module_path,
        echo_module().into_iter().chain([0]).collect::<Vec<_>>(),
    )
    .unwrap();
    let tampered = service_call(
        fixture.root.path(),
        CapsuleServiceRequest::Invoke(InvokeCapsuleRequest {
            policy: fixture.policy,
            capsule_id: "normalize-page".into(),
            enterprise_id: "enterprise-a".into(),
            input_schema: "typed-page-v1".into(),
            output_schema: "typed-page-v1".into(),
            input_base64: STANDARD.encode(br#"{"schema":"typed-page-v1"}"#),
        }),
    );
    assert!(matches!(tampered, CapsuleServiceResponse::Failed { .. }));
}

#[test]
fn signed_registry_validity_uses_the_host_clock() {
    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let future = service_fixture_with_validity(
        echo_module(),
        CapsuleHostLimits::default(),
        now_ms + 60_000,
        now_ms + 120_000,
    );
    let not_yet_valid = service_call(
        future.root.path(),
        CapsuleServiceRequest::Census(CensusCapsuleRequest {
            policy: future.policy,
            enterprise_id: "enterprise-a".into(),
        }),
    );
    assert!(matches!(
        not_yet_valid,
        CapsuleServiceResponse::Failed { ref message, .. } if message.contains("not yet valid")
    ));

    let expired =
        service_fixture_with_validity(echo_module(), CapsuleHostLimits::default(), 1, now_ms);
    let expired_response = service_call(
        expired.root.path(),
        CapsuleServiceRequest::Census(CensusCapsuleRequest {
            policy: expired.policy,
            enterprise_id: "enterprise-a".into(),
        }),
    );
    assert!(matches!(
        expired_response,
        CapsuleServiceResponse::Failed { ref message, .. } if message.contains("expired")
    ));
}

#[test]
fn service_refuses_non_regular_registry_module_and_module_store() {
    let registry_fixture = service_fixture();
    let registry = registry_fixture.root.path().join("registry-v1.json");
    std::fs::remove_file(&registry).unwrap();
    std::fs::create_dir(&registry).unwrap();
    let response = service_call(
        registry_fixture.root.path(),
        CapsuleServiceRequest::Census(CensusCapsuleRequest {
            policy: registry_fixture.policy,
            enterprise_id: "enterprise-a".into(),
        }),
    );
    assert!(matches!(
        response,
        CapsuleServiceResponse::Failed { ref message, .. } if message.contains("regular file")
    ));

    let module_fixture = service_fixture();
    std::fs::remove_file(&module_fixture.module_path).unwrap();
    std::fs::create_dir(&module_fixture.module_path).unwrap();
    let response = invoke_fixture(&module_fixture);
    assert!(matches!(
        response,
        CapsuleServiceResponse::Failed { ref message, .. } if message.contains("regular file")
    ));

    let store_fixture = service_fixture();
    std::fs::remove_file(&store_fixture.module_path).unwrap();
    let modules = store_fixture.root.path().join("modules");
    std::fs::remove_dir(&modules).unwrap();
    std::fs::write(&modules, b"not a directory").unwrap();
    let response = invoke_fixture(&store_fixture);
    assert!(matches!(
        response,
        CapsuleServiceResponse::Failed { ref message, .. } if message.contains("not a directory")
    ));
}

fn invoke_fixture(fixture: &ServiceFixture) -> CapsuleServiceResponse {
    service_call(
        fixture.root.path(),
        CapsuleServiceRequest::Invoke(InvokeCapsuleRequest {
            policy: fixture.policy.clone(),
            capsule_id: "normalize-page".into(),
            enterprise_id: "enterprise-a".into(),
            input_schema: "typed-page-v1".into(),
            output_schema: "typed-page-v1".into(),
            input_base64: STANDARD.encode(br#"{"schema":"typed-page-v1"}"#),
        }),
    )
}

#[cfg(unix)]
#[test]
fn signed_service_refuses_symlinked_registry_and_module_store() {
    use std::os::unix::fs::symlink;

    let fixture = service_fixture();
    let registry = fixture.root.path().join("registry-v1.json");
    let moved_registry = fixture.root.path().join("moved-registry.json");
    std::fs::rename(&registry, &moved_registry).unwrap();
    symlink(&moved_registry, &registry).unwrap();
    let response = service_call(
        fixture.root.path(),
        CapsuleServiceRequest::Census(CensusCapsuleRequest {
            policy: fixture.policy.clone(),
            enterprise_id: "enterprise-a".into(),
        }),
    );
    assert!(matches!(response, CapsuleServiceResponse::Failed { .. }));

    std::fs::remove_file(&registry).unwrap();
    std::fs::rename(&moved_registry, &registry).unwrap();
    let modules = fixture.root.path().join("modules");
    let moved_modules = fixture.root.path().join("moved-modules");
    std::fs::rename(&modules, &moved_modules).unwrap();
    symlink(&moved_modules, &modules).unwrap();
    let response = service_call(
        fixture.root.path(),
        CapsuleServiceRequest::Invoke(InvokeCapsuleRequest {
            policy: fixture.policy,
            capsule_id: "normalize-page".into(),
            enterprise_id: "enterprise-a".into(),
            input_schema: "typed-page-v1".into(),
            output_schema: "typed-page-v1".into(),
            input_base64: STANDARD.encode(br#"{"schema":"typed-page-v1"}"#),
        }),
    );
    assert!(matches!(response, CapsuleServiceResponse::Failed { .. }));
}

#[test]
fn service_cache_preserves_concurrency_after_an_observation_timeout() {
    let module = compile(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "scout_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scout_run") (param i32 i32) (result i64)
            (loop $forever (br $forever))
            (i64.const 0)))
        "#,
    );
    let fixture = service_fixture_with(
        module,
        CapsuleHostLimits {
            max_fuel: 20_000_000,
            deadline_ms: 1,
            max_concurrent_instances: 1,
            ..CapsuleHostLimits::default()
        },
    );
    let invoke = || {
        CapsuleServiceRequest::Invoke(InvokeCapsuleRequest {
            policy: fixture.policy.clone(),
            capsule_id: "normalize-page".into(),
            enterprise_id: "enterprise-a".into(),
            input_schema: "typed-page-v1".into(),
            output_schema: "typed-page-v1".into(),
            input_base64: STANDARD.encode(br#"{"schema":"typed-page-v1"}"#),
        })
    };

    let first = service_call(fixture.root.path(), invoke());
    let second = service_call(fixture.root.path(), invoke());

    assert!(matches!(
        first,
        CapsuleServiceResponse::Failed { ref message, .. }
            if message.contains("observation deadline")
    ));
    assert!(matches!(
        second,
        CapsuleServiceResponse::Failed { ref message, .. }
            if message.contains("concurrency")
    ));
}
