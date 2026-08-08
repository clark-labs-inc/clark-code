use std::collections::{BTreeMap, BTreeSet};

use scout_adapter_protocol::{
    AdapterId, AdapterQuery, AuthContextHandle, AuthContextId, CoverageBinding, CursorHandle,
    SafeFieldValue, TargetId,
};

use crate::{
    CompletionDisposition, ExpansionRule, PageCompletion, QuotaPolicy, RouteKind, ScheduleManifest,
    Scheduler, TaskOrigin, TaskSpec, TaskStatus, TerminalDisposition,
};

const ENTERPRISE: &str = "enterprise-acme";
const CHARTER: &str = "charter-acme";
const AUTHORITY: &str = "organization/acme";

fn digest(character: char) -> String {
    std::iter::repeat_n(character, 64).collect()
}

fn target(character: char) -> TargetId {
    TargetId::new(format!("target:{}", digest(character))).unwrap()
}

fn auth_context(character: char) -> AuthContextId {
    AuthContextId::new(format!("authctx:{}", digest(character))).unwrap()
}

fn auth_handle(suffix: u32) -> AuthContextHandle {
    AuthContextHandle::new(format!("auth:00000000-0000-4000-8000-{suffix:012}")).unwrap()
}

fn cursor(suffix: u32) -> CursorHandle {
    CursorHandle::new(format!("cursor:00000000-0000-4000-8000-{suffix:012}")).unwrap()
}

fn adapter(name: &str) -> AdapterId {
    AdapterId::new(format!("clark/{name}@1")).unwrap()
}

fn query(operation: &str, provider_resource_type: &str) -> AdapterQuery {
    AdapterQuery {
        operation: operation.into(),
        authority_scope: AUTHORITY.into(),
        provider_resource_type: provider_resource_type.into(),
        filters: BTreeMap::<String, SafeFieldValue>::new(),
        projection: BTreeSet::from(["native_id".into()]),
        page_size: 100,
    }
}

#[allow(clippy::too_many_arguments)]
fn task(
    target_id: TargetId,
    adapter_id: AdapterId,
    auth_context_id: AuthContextId,
    handle: AuthContextHandle,
    query: AdapterQuery,
    page_ordinal: u32,
    cursor_handle: Option<CursorHandle>,
    origin: TaskOrigin,
) -> TaskSpec {
    let coverage = CoverageBinding {
        enterprise_id: ENTERPRISE.into(),
        charter_id: CHARTER.into(),
        discovery_epoch: 1,
        sequence: 1,
        adapter_id: adapter_id.clone(),
        auth_context_id: auth_context_id.clone(),
        tenant: AUTHORITY.into(),
        region_or_project: "global".into(),
        resource_kind: query.provider_resource_type.clone(),
    };
    TaskSpec::new(
        ENTERPRISE,
        CHARTER,
        1,
        target_id,
        adapter_id,
        auth_context_id,
        handle,
        coverage,
        query,
        page_ordinal,
        cursor_handle,
        origin,
        100,
    )
    .unwrap()
}

fn policy(max_in_flight: u16, max_attempts: u16) -> QuotaPolicy {
    QuotaPolicy {
        max_in_flight,
        min_start_interval_ms: 0,
        lease_duration_ms: 1_000,
        base_backoff_ms: 100,
        max_backoff_ms: 10_000,
        max_attempts,
    }
}

fn scheduler(
    roots: Vec<TaskSpec>,
    policies: Vec<(crate::QuotaKey, QuotaPolicy)>,
    expansion_rules: Vec<ExpansionRule>,
) -> Scheduler {
    let root_task_ids = roots
        .iter()
        .map(|task| task.task_id.clone())
        .collect::<BTreeSet<_>>();
    let manifest = ScheduleManifest::new(
        ENTERPRISE,
        CHARTER,
        1,
        root_task_ids,
        policies.into_iter().collect(),
        expansion_rules
            .into_iter()
            .map(|rule| (rule.rule_id.clone(), rule))
            .collect(),
    )
    .unwrap();
    Scheduler::new(manifest, roots, 10).unwrap()
}

fn success(
    claim: &crate::LeaseClaim,
    completed_at_ms: u64,
    final_page: bool,
    continuation: Option<TaskSpec>,
    expansions: Vec<TaskSpec>,
) -> PageCompletion {
    PageCompletion {
        task_id: claim.task.task_id.clone(),
        machine_id: claim.machine_id.clone(),
        fence: claim.fence,
        completed_at_ms,
        disposition: CompletionDisposition::Success { final_page },
        receipt_id: Some(format!("receipt:{}", digest('c'))),
        evidence_sha256: Some(digest('d')),
        continuation,
        expansions,
    }
}

#[test]
fn quota_and_target_affinity_prevent_double_claims() {
    let target_id = target('a');
    let adapter_id = adapter("aws-resource-explorer");
    let auth_context_id = auth_context('b');
    let handle = auth_handle(1);
    let first = task(
        target_id.clone(),
        adapter_id.clone(),
        auth_context_id.clone(),
        handle.clone(),
        query("search", "aws_resource"),
        0,
        None,
        TaskOrigin::Root,
    );
    let mut second = task(
        target_id.clone(),
        adapter_id,
        auth_context_id,
        handle,
        query("search", "aws_resource"),
        0,
        None,
        TaskOrigin::Root,
    );
    second.coverage.sequence = 2;
    second = TaskSpec::new(
        second.enterprise_id,
        second.charter_id,
        second.discovery_epoch,
        second.target_id,
        second.adapter_id,
        second.auth_context_id,
        second.auth_context_handle,
        second.coverage,
        second.query,
        second.page_ordinal,
        second.cursor_handle,
        second.origin,
        second.priority,
    )
    .unwrap();
    let quota_key = first.quota_key();
    let mut scheduler = scheduler(vec![first, second], vec![(quota_key, policy(1, 3))], vec![]);
    let wrong_target = BTreeSet::from([target('f')]);
    assert!(scheduler
        .claim("machine-wrong", &wrong_target, 20, 2)
        .unwrap()
        .is_empty());

    let eligible = BTreeSet::from([target_id]);
    let first_claim = scheduler.claim("machine-a", &eligible, 20, 2).unwrap();
    assert_eq!(first_claim.len(), 1);
    assert!(scheduler
        .claim("machine-b", &eligible, 20, 2)
        .unwrap()
        .is_empty());

    let mut stale = success(&first_claim[0], 30, true, None, vec![]);
    stale.machine_id = "machine-b".into();
    assert!(scheduler.complete(stale).is_err());
    scheduler
        .complete(success(&first_claim[0], 30, true, None, vec![]))
        .unwrap();
    assert_eq!(
        scheduler
            .claim("machine-b", &eligible, 40, 2)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn continuation_can_move_workers_but_not_targets_or_routes() {
    let target_id = target('a');
    let adapter_id = adapter("github");
    let auth_context_id = auth_context('b');
    let handle = auth_handle(1);
    let root = task(
        target_id.clone(),
        adapter_id.clone(),
        auth_context_id.clone(),
        handle.clone(),
        query("list_repositories", "repository"),
        0,
        None,
        TaskOrigin::Root,
    );
    let mut scheduler = scheduler(
        vec![root.clone()],
        vec![(root.quota_key(), policy(1, 3))],
        vec![],
    );
    let eligible = BTreeSet::from([target_id.clone()]);
    let claim = scheduler
        .claim("machine-a", &eligible, 20, 1)
        .unwrap()
        .remove(0);
    let continuation = task(
        target_id,
        adapter_id,
        auth_context_id,
        handle,
        query("list_repositories", "repository"),
        1,
        Some(cursor(1)),
        TaskOrigin::Continuation {
            parent_task_id: root.task_id.clone(),
        },
    );
    scheduler
        .complete(success(
            &claim,
            30,
            false,
            Some(continuation.clone()),
            vec![],
        ))
        .unwrap();
    let partial = scheduler.receipt().unwrap();
    assert_eq!(partial.tasks, 2);
    assert!(!partial.sealed);

    let claim = scheduler
        .claim("machine-b", &eligible, 40, 1)
        .unwrap()
        .remove(0);
    assert_eq!(claim.task.task_id, continuation.task_id);
    scheduler
        .complete(success(&claim, 50, true, None, vec![]))
        .unwrap();
    let complete = scheduler.receipt().unwrap();
    assert!(complete.sealed);
    assert!(complete.complete);
}

#[test]
fn expired_leases_back_off_and_eventually_become_explicit_gaps() {
    let root = task(
        target('a'),
        adapter("gcp"),
        auth_context('b'),
        auth_handle(1),
        query("search_all_resources", "gcp_resource"),
        0,
        None,
        TaskOrigin::Root,
    );
    let eligible = BTreeSet::from([root.target_id.clone()]);
    let mut scheduler = scheduler(
        vec![root.clone()],
        vec![(root.quota_key(), policy(1, 2))],
        vec![],
    );
    let stale_claim = scheduler
        .claim("machine-a", &eligible, 20, 1)
        .unwrap()
        .remove(0);
    assert_eq!(scheduler.reap_expired(1_021).unwrap(), 1);
    assert!(scheduler
        .claim("machine-b", &eligible, 1_050, 1)
        .unwrap()
        .is_empty());
    let retry = scheduler
        .claim("machine-b", &eligible, 1_121, 1)
        .unwrap()
        .remove(0);
    assert_ne!(retry.fence, stale_claim.fence);
    assert!(scheduler
        .complete(success(&stale_claim, 900, true, None, vec![]))
        .is_err());
    assert_eq!(scheduler.reap_expired(2_122).unwrap(), 1);
    let receipt = scheduler.receipt().unwrap();
    assert!(receipt.sealed);
    assert!(!receipt.complete);
    assert_eq!(receipt.gap_terminal, 1);
    assert!(matches!(
        scheduler.task_status(&root.task_id),
        Some(TaskStatus::Terminal {
            disposition: TerminalDisposition::RetryExhausted,
            ..
        })
    ));
}

#[test]
fn expansion_requires_a_manifest_rule_and_is_atomic() {
    let target_id = target('a');
    let parent = task(
        target_id.clone(),
        adapter("aws-organizations"),
        auth_context('b'),
        auth_handle(1),
        query("list_accounts", "aws_account"),
        0,
        None,
        TaskOrigin::Root,
    );
    let child_adapter = adapter("aws-resource-explorer");
    let child_auth_context = auth_context('e');
    let child_handle = auth_handle(2);
    let child_query = query("search", "aws_resource");
    let rule = ExpansionRule {
        rule_id: "account-to-resources".into(),
        parent: parent.route_kind(),
        child: RouteKind::new(
            child_adapter.clone(),
            child_query.operation.clone(),
            child_query.provider_resource_type.clone(),
        )
        .unwrap(),
        same_target: true,
        max_children_per_parent: 2,
    };
    let child = task(
        target_id.clone(),
        child_adapter,
        child_auth_context,
        child_handle,
        child_query,
        0,
        None,
        TaskOrigin::Expansion {
            parent_task_id: parent.task_id.clone(),
            rule_id: rule.rule_id.clone(),
            source_evidence_sha256: digest('9'),
        },
    );
    let mut scheduler = scheduler(
        vec![parent.clone()],
        vec![
            (parent.quota_key(), policy(1, 3)),
            (child.quota_key(), policy(2, 3)),
        ],
        vec![rule],
    );
    let eligible = BTreeSet::from([target_id]);
    let claim = scheduler
        .claim("machine-a", &eligible, 20, 1)
        .unwrap()
        .remove(0);

    let mut invalid_child = child.clone();
    invalid_child.origin = TaskOrigin::Expansion {
        parent_task_id: parent.task_id.clone(),
        rule_id: "undeclared".into(),
        source_evidence_sha256: digest('9'),
    };
    invalid_child = TaskSpec::new(
        invalid_child.enterprise_id,
        invalid_child.charter_id,
        invalid_child.discovery_epoch,
        invalid_child.target_id,
        invalid_child.adapter_id,
        invalid_child.auth_context_id,
        invalid_child.auth_context_handle,
        invalid_child.coverage,
        invalid_child.query,
        invalid_child.page_ordinal,
        invalid_child.cursor_handle,
        invalid_child.origin,
        invalid_child.priority,
    )
    .unwrap();
    assert!(scheduler
        .complete(success(&claim, 30, true, None, vec![invalid_child],))
        .is_err());
    assert!(matches!(
        scheduler.task_status(&parent.task_id),
        Some(TaskStatus::Leased { .. })
    ));

    scheduler
        .complete(success(&claim, 30, true, None, vec![child.clone()]))
        .unwrap();
    assert_eq!(scheduler.task_count(), 2);
    assert!(matches!(
        scheduler.task_status(&child.task_id),
        Some(TaskStatus::Pending { .. })
    ));
}

#[test]
fn encoded_state_revalidates_and_preserves_receipt() {
    let root = task(
        target('a'),
        adapter("github"),
        auth_context('b'),
        auth_handle(1),
        query("list_repositories", "repository"),
        0,
        None,
        TaskOrigin::Root,
    );
    let scheduler = scheduler(
        vec![root.clone()],
        vec![(root.quota_key(), policy(1, 3))],
        vec![],
    );
    let bytes = scheduler.encode().unwrap();
    let decoded = Scheduler::decode(&bytes).unwrap();
    assert_eq!(decoded, scheduler);
    assert_eq!(
        decoded.receipt().unwrap().state_sha256,
        scheduler.receipt().unwrap().state_sha256
    );
}
