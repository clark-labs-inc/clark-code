use crate::model::{
    Evidence, EvidenceRole, EvidenceSource, Scenario, SemanticPlanCheck, Verification,
};
use crate::scenario_families::{apply_family, evidence, seed_family, verify_family, FamilySpec};
use std::path::Path;

macro_rules! family_functions {
    ($seed:ident, $apply:ident, $verify:ident, $spec:ident) => {
        fn $seed(root: &Path) -> Result<(), String> {
            seed_family(root, &$spec)
        }
        fn $apply(root: &Path) -> Result<(), String> {
            apply_family(root, &$spec)
        }
        fn $verify(root: &Path) -> Verification {
            verify_family(root, &$spec)
        }
    };
}

const CACHE_FILES: &[(&str, &str, &str)] = &[
    ("repos/policy/src/cache.mjs", "export const key=(tenant)=>tenant;\n", "export const key=(tenant,version)=>`${tenant}:${version}`; export const accept=(cached,current)=>cached.version===current;\n"),
    ("repos/policy/src/publisher.mjs", "export const publish=(tenant)=>({tenant});\n", "export const publish=(tenant,version)=>({type:'policy.invalidated',tenant,version});\n"),
    ("repos/gateway/src/policySubscriber.mjs", "export const apply=()=>false;\n", "export const apply=(cache,event)=>cache.delete(`${event.tenant}:${event.version-1}`);\n"),
    ("repos/worker/src/policySubscriber.mjs", "export const apply=()=>false;\n", "export const apply=(cache,event)=>cache.delete(`${event.tenant}:${event.version-1}`);\n"),
    ("config/policy-cache.json", "{\"ttlSeconds\":3600}\n", "{\"ttlSeconds\":300,\"versionedKeys\":true,\"event\":\"policy.invalidated\"}\n"),
    ("deploy/production/policy-cache.json", "{\"order\":[\"publisher\",\"consumers\"]}\n", "{\"order\":[\"gateway-subscriber\",\"worker-subscriber\",\"versioned-writes\",\"publisher\"],\"rollback\":[\"disable-publisher\",\"retain-versioned-read\"]}\n"),
    ("repos/observability/src/policyMetrics.mjs", "export const metrics=['policy_cache_hit_total'];\n", "export const metrics=['policy_cache_hit_total','policy_cache_stale_reject_total','policy_invalidation_lag_ms'];\n"),
    ("docs/policy-cache.md", "Policy cache uses a one hour TTL.\n", "Version keys by tenant and policy version, reject stale entries, deploy both subscribers before publishing invalidations, and roll back by disabling the publisher while retaining versioned reads.\n"),
];
const CACHE_CHECKS: &[(&str, &str)] = &[
    ("versioned_cache_keys", "const c=await load('repos/policy/src/cache.mjs'); assert.equal(c.key('t1',7),'t1:7'); assert.equal(c.accept({version:6},7),false);"),
    ("versioned_invalidation_event", "const p=await load('repos/policy/src/publisher.mjs'); assert.deepEqual(p.publish('t1',7),{type:'policy.invalidated',tenant:'t1',version:7});"),
    ("all_cache_consumers", "for(const path of ['repos/gateway/src/policySubscriber.mjs','repos/worker/src/policySubscriber.mjs']){const m=await load(path);const cache=new Map([['t1:6',{}]]);m.apply(cache,{tenant:'t1',version:7});assert.equal(cache.has('t1:6'),false)}"),
    ("bounded_staleness", "const fs=await import('node:fs/promises'); const c=JSON.parse(await fs.readFile(join(root,'config/policy-cache.json'),'utf8')); assert.equal(c.ttlSeconds,300); assert.equal(c.versionedKeys,true);"),
    ("subscriber_first_rollout", "const fs=await import('node:fs/promises'); const d=JSON.parse(await fs.readFile(join(root,'deploy/production/policy-cache.json'),'utf8')); const doc=await fs.readFile(join(root,'docs/policy-cache.md'),'utf8'); const m=await load('repos/observability/src/policyMetrics.mjs'); assert.deepEqual(d.order.slice(0,2),['gateway-subscriber','worker-subscriber']); assert.equal(d.rollback[0],'disable-publisher'); assert.ok(m.metrics.includes('policy_cache_stale_reject_total')); assert.match(doc,/retaining versioned reads/);"),
];
const CACHE_SPEC: FamilySpec = FamilySpec {
    name: "tenant-policy-cache-invalidation",
    files: CACHE_FILES,
    checks: CACHE_CHECKS,
};
family_functions!(seed_cache, apply_cache, verify_cache, CACHE_SPEC);
const CACHE_EVIDENCE: &[Evidence] = &[
    Evidence { id:"PROJECT-CACHE-VERSION", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"Policy cache keys include tenant and monotonic policy version; readers reject cached versions older than the authoritative version." },
    Evidence { id:"ORG-CACHE-TTL", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Policy staleness is bounded at 300 seconds; invalidation subscribers deploy before publishing events." },
    Evidence { id:"SCOUT-CACHE-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Observed policy.invalidated consumers are gateway and background worker; both export invalidation lag telemetry." },
    Evidence { id:"ORACLE-CACHE", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Implement versioned keys/events, both subscribers, bounded TTL, subscriber-first rollout, safe rollback, and stale-reject metrics." },
    Evidence { id:"STALE-CACHE-TTL", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Old design relies only on a one-hour TTL." },
    Evidence { id:"CONFLICT-CACHE-FLUSH", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Unapproved shortcut flushes every tenant cache on any policy change." },
];
const CACHE_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "versioned_cache_keys",
        required_all: &["cache.mjs", "tenant", "version", "reject stale"],
        required_any: &[],
        expectation: "plan uses versioned tenant keys and stale rejection",
    },
    SemanticPlanCheck {
        id: "versioned_invalidation_event",
        required_all: &["publisher.mjs", "policy.invalidated", "tenant", "version"],
        required_any: &[],
        expectation: "plan defines the versioned invalidation event",
    },
    SemanticPlanCheck {
        id: "all_cache_consumers",
        required_all: &["gateway", "worker", "subscriber"],
        required_any: &[],
        expectation: "plan updates every observed cache consumer",
    },
    SemanticPlanCheck {
        id: "bounded_staleness",
        required_all: &["300", "ttl", "versionedkeys"],
        required_any: &[],
        expectation: "plan keeps an explicit staleness bound",
    },
    SemanticPlanCheck {
        id: "subscriber_first_rollout",
        required_all: &[
            "subscriber-first",
            "disable-publisher",
            "retain-versioned-read",
            "policy_cache_stale_reject_total",
        ],
        required_any: &[],
        expectation: "plan encodes rollout rollback and metrics",
    },
];

const RETENTION_FILES: &[(&str, &str, &str)] = &[
    ("repos/artifacts/src/classifier.mjs", "export const classify=()=>({days:30});\n", "export const classify=(kind)=>kind==='audit'?{days:2555}:{days:30};\n"),
    ("repos/storage/src/artifact.mjs", "export const metadata=(id)=>({id});\n", "export const metadata=(id,retentionClass,hold=false)=>({id,retentionClass,legalHold:hold});\n"),
    ("repos/worker/src/deleteArtifact.mjs", "export const canDelete=()=>true;\n", "export const canDelete=(artifact,now)=>!artifact.legalHold&&now>=artifact.deleteAfter;\n"),
    ("repos/api/src/legalHold.mjs", "export const setHold=(artifact)=>artifact;\n", "export const setHold=(artifact,enabled,caseId)=>({...artifact,legalHold:enabled,holdCaseId:enabled?caseId:null});\n"),
    ("config/artifact-retention.json", "{\"defaultDays\":30}\n", "{\"defaultDays\":30,\"classes\":{\"audit\":2555},\"holdOverridesDeletion\":true}\n"),
    ("deploy/production/artifact-retention.json", "{\"order\":[\"worker\",\"api\"]}\n", "{\"order\":[\"metadata-schema\",\"legal-hold-api\",\"delete-guard\",\"sweeper\"],\"rollback\":[\"pause-sweeper\",\"retain-metadata\"]}\n"),
    ("repos/observability/src/retentionMetrics.mjs", "export const metrics=['artifact_delete_total'];\n", "export const metrics=['artifact_delete_total','artifact_hold_skip_total','artifact_retention_violation_total'];\n"),
    ("docs/artifact-retention.md", "Artifacts expire after thirty days.\n", "Audit artifacts retain for 2555 days. Legal hold overrides deletion. Roll back by pausing the sweeper and retaining hold metadata; never clear holds during rollback.\n"),
];
const RETENTION_CHECKS: &[(&str, &str)] = &[
    ("retention_classes", "const c=await load('repos/artifacts/src/classifier.mjs'); assert.equal(c.classify('audit').days,2555); assert.equal(c.classify('temp').days,30);"),
    ("durable_hold_metadata", "const s=await load('repos/storage/src/artifact.mjs'); const a=await load('repos/api/src/legalHold.mjs'); assert.deepEqual(a.setHold(s.metadata('x','audit'),true,'case-7'),{id:'x',retentionClass:'audit',legalHold:true,holdCaseId:'case-7'});"),
    ("delete_guard", "const w=await load('repos/worker/src/deleteArtifact.mjs'); assert.equal(w.canDelete({legalHold:true,deleteAfter:1},100),false); assert.equal(w.canDelete({legalHold:false,deleteAfter:50},100),true);"),
    ("policy_configuration", "const fs=await import('node:fs/promises'); const c=JSON.parse(await fs.readFile(join(root,'config/artifact-retention.json'),'utf8')); assert.equal(c.classes.audit,2555); assert.equal(c.holdOverridesDeletion,true);"),
    ("safe_sweeper_rollout", "const fs=await import('node:fs/promises'); const d=JSON.parse(await fs.readFile(join(root,'deploy/production/artifact-retention.json'),'utf8')); const m=await load('repos/observability/src/retentionMetrics.mjs'); const doc=await fs.readFile(join(root,'docs/artifact-retention.md'),'utf8'); assert.equal(d.order[0],'metadata-schema'); assert.deepEqual(d.rollback,['pause-sweeper','retain-metadata']); assert.ok(m.metrics.includes('artifact_hold_skip_total')); assert.match(doc,/never clear holds/);"),
];
const RETENTION_SPEC: FamilySpec = FamilySpec {
    name: "artifact-retention-legal-hold",
    files: RETENTION_FILES,
    checks: RETENTION_CHECKS,
};
family_functions!(
    seed_retention,
    apply_retention,
    verify_retention,
    RETENTION_SPEC
);
const RETENTION_EVIDENCE: &[Evidence] = &[
    Evidence { id:"PROJECT-RETENTION-CLASS", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"Audit artifacts use the seven-year 2555-day class; ordinary temporary artifacts remain thirty days." },
    Evidence { id:"ORG-LEGAL-HOLD", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Legal hold is durable metadata and always overrides scheduled deletion; rollback may never clear a hold." },
    Evidence { id:"SCOUT-RETENTION-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Observed path is metadata classifier -> hold API -> delete guard -> sweeper, with hold-skip and violation metrics." },
    Evidence { id:"ORACLE-RETENTION", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Implement retention classes, hold metadata/API, delete guard, policy config, schema-first rollout, pause-sweeper rollback, and metrics." },
    Evidence { id:"STALE-RETENTION-30", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Superseded policy expires every artifact after thirty days." },
    Evidence { id:"CONFLICT-RETENTION-CLEAR", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Unapproved cleanup clears holds when the sweeper rolls back." },
];
const RETENTION_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "retention_classes",
        required_all: &["classifier.mjs", "audit", "2555", "30"],
        required_any: &[],
        expectation: "plan distinguishes audit and default retention classes",
    },
    SemanticPlanCheck {
        id: "durable_hold_metadata",
        required_all: &["artifact.mjs", "legalhold.mjs", "holdcaseid", "durable"],
        required_any: &[],
        expectation: "plan persists hold state and case identity",
    },
    SemanticPlanCheck {
        id: "delete_guard",
        required_all: &["deleteartifact.mjs", "legalhold", "deleteafter"],
        required_any: &[],
        expectation: "plan guards deletion with both time and hold",
    },
    SemanticPlanCheck {
        id: "policy_configuration",
        required_all: &["artifact-retention.json", "holdoverridesdeletion", "2555"],
        required_any: &[],
        expectation: "plan updates executable policy configuration",
    },
    SemanticPlanCheck {
        id: "safe_sweeper_rollout",
        required_all: &[
            "metadata-schema",
            "pause-sweeper",
            "retain-metadata",
            "artifact_hold_skip_total",
            "never clear",
        ],
        required_any: &[],
        expectation: "plan encodes safe rollout rollback and observability",
    },
];

const SYNC_FILES: &[(&str, &str, &str)] = &[
    ("repos/sync/src/protocol.mjs", "export const version=2; export const encode=(op)=>op;\n", "export const versions=[2,3]; export const encode=(op)=>({...op,protocolVersion:3,operationId:op.operationId});\n"),
    ("repos/mobile/src/offlineQueue.mjs", "export const enqueue=(queue,op)=>queue.push(op);\n", "export const enqueue=(queue,op)=>queue.some(x=>x.operationId===op.operationId)?queue.length:queue.push(op); export const pending=(queue)=>queue.filter(x=>!x.acked);\n"),
    ("repos/api/src/syncMerge.mjs", "export const merge=(server,client)=>client;\n", "export const merge=(server,client)=>client.updatedAt>=server.updatedAt?{...server,...client}:{...client,...server};\n"),
    ("repos/sync/src/conflicts.mjs", "export const resolve=(a,b)=>b;\n", "export const resolve=(a,b)=>a.updatedAt===b.updatedAt?(a.deviceId<b.deviceId?a:b):(a.updatedAt>b.updatedAt?a:b);\n"),
    ("config/mobile-sync.json", "{\"protocol\":2}\n", "{\"protocol\":3,\"dualRead\":[2,3],\"batchSize\":100,\"ackByOperationId\":true}\n"),
    ("deploy/production/mobile-sync.json", "{\"order\":[\"mobile\",\"server\"]}\n", "{\"order\":[\"server-dual-read\",\"conflict-resolver\",\"mobile-v3-write\"],\"rollback\":[\"mobile-v2-write\",\"retain-server-dual-read\"]}\n"),
    ("repos/observability/src/syncMetrics.mjs", "export const metrics=['sync_total'];\n", "export const metrics=['sync_total','sync_duplicate_operation_total','sync_conflict_total','sync_unacked_operations'];\n"),
    ("docs/mobile-sync.md", "Clients send pending changes when online.\n", "Server dual-reads protocol 2 and 3 before mobile writes v3. Dedupe and acknowledge by operationId, resolve equal timestamps by deviceId, and roll back mobile writes while retaining server dual-read.\n"),
];
const SYNC_CHECKS: &[(&str, &str)] = &[
    ("versioned_protocol", "const p=await load('repos/sync/src/protocol.mjs'); assert.deepEqual(p.versions,[2,3]); assert.deepEqual(p.encode({operationId:'o1',x:1}),{operationId:'o1',x:1,protocolVersion:3});"),
    ("offline_dedup_ack", "const q=await load('repos/mobile/src/offlineQueue.mjs'); const queue=[];q.enqueue(queue,{operationId:'o1'});q.enqueue(queue,{operationId:'o1'});assert.equal(queue.length,1);assert.equal(q.pending(queue).length,1);"),
    ("deterministic_merge", "const m=await load('repos/api/src/syncMerge.mjs'); const c=await load('repos/sync/src/conflicts.mjs'); assert.equal(m.merge({updatedAt:2,value:'s'},{updatedAt:1,value:'c'}).value,'s'); assert.equal(c.resolve({updatedAt:2,deviceId:'a'},{updatedAt:2,deviceId:'b'}).deviceId,'a');"),
    ("bounded_batch_compat", "const fs=await import('node:fs/promises'); const c=JSON.parse(await fs.readFile(join(root,'config/mobile-sync.json'),'utf8')); assert.deepEqual(c.dualRead,[2,3]);assert.equal(c.batchSize,100);assert.equal(c.ackByOperationId,true);"),
    ("server_first_rollout", "const fs=await import('node:fs/promises'); const d=JSON.parse(await fs.readFile(join(root,'deploy/production/mobile-sync.json'),'utf8')); const doc=await fs.readFile(join(root,'docs/mobile-sync.md'),'utf8'); const m=await load('repos/observability/src/syncMetrics.mjs'); assert.equal(d.order[0],'server-dual-read'); assert.deepEqual(d.rollback,['mobile-v2-write','retain-server-dual-read']); assert.ok(m.metrics.includes('sync_duplicate_operation_total')); assert.match(doc,/retaining server dual-read/);"),
];
const SYNC_SPEC: FamilySpec = FamilySpec {
    name: "mobile-offline-sync-v3",
    files: SYNC_FILES,
    checks: SYNC_CHECKS,
};
family_functions!(seed_sync, apply_sync, verify_sync, SYNC_SPEC);
const SYNC_EVIDENCE: &[Evidence] = &[
    Evidence { id:"PROJECT-SYNC-ID", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"Offline operations have stable operationId values used for queue dedupe and acknowledgements; batches contain at most 100 operations." },
    Evidence { id:"ORG-SYNC-COMPAT", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"The server dual-reads protocols 2 and 3 before mobile v3 writes; rollback returns mobile to v2 while retaining server dual-read." },
    Evidence { id:"SCOUT-SYNC-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Observed graph includes mobile offline queue, API merge, conflict resolver, and sync telemetry; equal timestamps resolve by stable deviceId." },
    Evidence { id:"ORACLE-SYNC", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Implement v2/v3 protocol, operation dedupe/ack, deterministic merge, bounded compatibility config, server-first rollout, rollback, and metrics." },
    Evidence { id:"STALE-SYNC-LWW", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Old prototype accepted whichever operation arrived last without a stable tie break." },
    Evidence { id:"CONFLICT-SYNC-DROP", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Unapproved migration disables protocol 2 reads as soon as one v3 client ships." },
];
const SYNC_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "versioned_protocol",
        required_all: &["protocol.mjs", "versions 2 and 3", "operationid"],
        required_any: &[],
        expectation: "plan retains v2 and adds operation-identified v3",
    },
    SemanticPlanCheck {
        id: "offline_dedup_ack",
        required_all: &["offlinequeue.mjs", "dedupe", "acknowledge", "operationid"],
        required_any: &[],
        expectation: "plan deduplicates and acknowledges offline operations",
    },
    SemanticPlanCheck {
        id: "deterministic_merge",
        required_all: &["syncmerge.mjs", "conflicts.mjs", "updatedat", "deviceid"],
        required_any: &[],
        expectation: "plan defines deterministic conflict resolution",
    },
    SemanticPlanCheck {
        id: "bounded_batch_compat",
        required_all: &["dual-read", "2 and 3", "100", "ackbyoperationid"],
        required_any: &[],
        expectation: "plan bounds batches and preserves protocol compatibility",
    },
    SemanticPlanCheck {
        id: "server_first_rollout",
        required_all: &[
            "server-first",
            "mobile-v2-write",
            "retain-server-dual-read",
            "sync_duplicate_operation_total",
        ],
        required_any: &[],
        expectation: "plan encodes safe rollout rollback and metrics",
    },
];

pub fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            id: "tenant-policy-cache-invalidation",
            task: "Make tenant policy cache invalidation version-aware across publisher, gateway, and worker. Bound stale reads, deploy subscribers before publishing, preserve rollback safety, add stale/lag metrics, and run visible tests.",
            required_plan_terms: &["cache.mjs","publisher.mjs","policySubscriber.mjs","policy-cache.json","policy-cache.json","policyMetrics.mjs","policy-cache.md","rollback"],
            semantic_plan_checks: CACHE_SEMANTICS,
            required_evidence: &["PROJECT-CACHE-VERSION","ORG-CACHE-TTL","SCOUT-CACHE-GRAPH"],
            forbidden_evidence: &["STALE-CACHE-TTL","CONFLICT-CACHE-FLUSH"],
            oracle_plan: "Use PROJECT-CACHE-VERSION, ORG-CACHE-TTL, and SCOUT-CACHE-GRAPH. cache.mjs keys by tenant and version and must reject stale entries. publisher.mjs emits policy.invalidated with tenant and version. Update gateway and worker policySubscriber.mjs consumers before publisher activation. policy-cache.json sets ttl 300 and versionedKeys. Use a subscriber-first rollout; rollback disable-publisher while retain-versioned-read. policyMetrics.mjs adds policy_cache_stale_reject_total and invalidation lag, and policy-cache.md documents the boundary.",
            evidence: evidence(CACHE_EVIDENCE),
            seed: seed_cache,
            verify: verify_cache,
            reference_apply: apply_cache,
        },
        Scenario {
            id: "artifact-retention-legal-hold",
            task: "Implement versioned artifact-retention classes and legal hold across classifier, storage metadata, API, delete worker, production rollout, and observability. Holds must override deletion and survive rollback. Run visible tests.",
            required_plan_terms: &["classifier.mjs","artifact.mjs","deleteArtifact.mjs","legalHold.mjs","artifact-retention.json","retentionMetrics.mjs","artifact-retention.md","rollback"],
            semantic_plan_checks: RETENTION_SEMANTICS,
            required_evidence: &["PROJECT-RETENTION-CLASS","ORG-LEGAL-HOLD","SCOUT-RETENTION-GRAPH"],
            forbidden_evidence: &["STALE-RETENTION-30","CONFLICT-RETENTION-CLEAR"],
            oracle_plan: "Use PROJECT-RETENTION-CLASS, ORG-LEGAL-HOLD, and SCOUT-RETENTION-GRAPH. classifier.mjs assigns audit 2555 days and default 30. artifact.mjs stores durable legalHold and retention class; legalHold.mjs persists holdCaseId. deleteArtifact.mjs requires deleteAfter and no legalHold. artifact-retention.json sets holdOverridesDeletion and the audit class. Deploy metadata-schema, legal-hold-api, delete-guard, sweeper. Rollback pause-sweeper and retain-metadata; never clear holds. retentionMetrics.mjs adds artifact_hold_skip_total and artifact-retention.md documents the invariant.",
            evidence: evidence(RETENTION_EVIDENCE),
            seed: seed_retention,
            verify: verify_retention,
            reference_apply: apply_retention,
        },
        Scenario {
            id: "mobile-offline-sync-v3",
            task: "Implement protocol v3 for offline mobile synchronization without dropping v2 clients. Dedupe and acknowledge operations, define deterministic conflict resolution, bound batches, encode server-first rollout and safe rollback, add metrics, and run visible tests.",
            required_plan_terms: &["protocol.mjs","offlineQueue.mjs","syncMerge.mjs","conflicts.mjs","mobile-sync.json","syncMetrics.mjs","mobile-sync.md","rollback"],
            semantic_plan_checks: SYNC_SEMANTICS,
            required_evidence: &["PROJECT-SYNC-ID","ORG-SYNC-COMPAT","SCOUT-SYNC-GRAPH"],
            forbidden_evidence: &["STALE-SYNC-LWW","CONFLICT-SYNC-DROP"],
            oracle_plan: "Use PROJECT-SYNC-ID, ORG-SYNC-COMPAT, and SCOUT-SYNC-GRAPH. protocol.mjs retains versions 2 and 3 and encodes operationId. offlineQueue.mjs must dedupe and acknowledge by operationId. syncMerge.mjs chooses updatedAt while conflicts.mjs breaks equal timestamps by stable deviceId. mobile-sync.json dual-reads 2 and 3 with batchSize 100 and ackByOperationId. Use server-first rollout, then mobile v3 writes; rollback mobile-v2-write and retain-server-dual-read. syncMetrics.mjs adds sync_duplicate_operation_total and mobile-sync.md documents compatibility.",
            evidence: evidence(SYNC_EVIDENCE),
            seed: seed_sync,
            verify: verify_sync,
            reference_apply: apply_sync,
        },
    ]
}
