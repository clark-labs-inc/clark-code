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

const FLAG_FILES: &[(&str, &str, &str)] = &[
    ("repos/flags/src/schema.mjs", "export const scopes=['environment'];\n", "export const scopes=['environment','tenant']; export const key=(flag,tenant)=>`${flag}:tenant:${tenant}`;\n"),
    ("repos/flags/src/evaluate.mjs", "export const evaluate=(values,flag)=>values[flag];\n", "export const evaluate=(values,flag,tenant)=>values[`${flag}:tenant:${tenant}`]??values[`${flag}:environment`]??false;\n"),
    ("repos/admin/src/flagMigration.mjs", "export const migrate=(flag)=>flag;\n", "export const migrate=(flag,tenants,value)=>Object.fromEntries(tenants.map(t=>[`${flag}:tenant:${t}`,value]));\n"),
    ("repos/sdk/src/flags.mjs", "export const enabled=(client,flag)=>client.get(flag);\n", "export const enabled=(client,flag,tenant)=>client.get(flag,tenant,{fallbackScope:'environment'});\n"),
    ("config/flag-scope.json", "{\"scope\":\"environment\"}\n", "{\"writeScope\":\"tenant\",\"fallbackScope\":\"environment\",\"dualRead\":true}\n"),
    ("deploy/production/flag-scope.json", "{\"order\":[\"admin\",\"sdk\"]}\n", "{\"order\":[\"schema\",\"evaluator-dual-read\",\"sdk\",\"admin-tenant-write\"],\"rollback\":[\"admin-environment-write\",\"retain-dual-read\"]}\n"),
    ("repos/observability/src/flagMetrics.mjs", "export const metrics=['flag_eval_total'];\n", "export const metrics=['flag_eval_total','flag_tenant_fallback_total','flag_scope_miss_total'];\n"),
    ("docs/flag-scope.md", "Flags are scoped by environment.\n", "Read tenant first and environment second. Deploy evaluator dual-read before SDK and tenant writes. Roll back writes to environment while retaining dual-read.\n"),
];
const FLAG_CHECKS: &[(&str, &str)] = &[
    ("tenant_scope_schema", "const s=await load('repos/flags/src/schema.mjs'); assert.deepEqual(s.scopes,['environment','tenant']); assert.equal(s.key('new-ui','t1'),'new-ui:tenant:t1');"),
    ("ordered_scope_fallback", "const e=await load('repos/flags/src/evaluate.mjs'); assert.equal(e.evaluate({'f:tenant:t1':true,'f:environment':false},'f','t1'),true); assert.equal(e.evaluate({'f:environment':true},'f','t2'),true); assert.equal(e.evaluate({},'f','t2'),false);"),
    ("migration_and_sdk", "const a=await load('repos/admin/src/flagMigration.mjs'); const s=await load('repos/sdk/src/flags.mjs'); assert.deepEqual(a.migrate('f',['t1','t2'],true),{'f:tenant:t1':true,'f:tenant:t2':true}); const calls=[];s.enabled({get:(...x)=>calls.push(x)},'f','t1');assert.deepEqual(calls[0],['f','t1',{fallbackScope:'environment'}]);"),
    ("dual_read_configuration", "const fs=await import('node:fs/promises'); const c=JSON.parse(await fs.readFile(join(root,'config/flag-scope.json'),'utf8')); assert.equal(c.writeScope,'tenant');assert.equal(c.fallbackScope,'environment');assert.equal(c.dualRead,true);"),
    ("evaluator_first_rollout", "const fs=await import('node:fs/promises'); const d=JSON.parse(await fs.readFile(join(root,'deploy/production/flag-scope.json'),'utf8')); const doc=await fs.readFile(join(root,'docs/flag-scope.md'),'utf8'); const m=await load('repos/observability/src/flagMetrics.mjs');assert.equal(d.order[1],'evaluator-dual-read');assert.deepEqual(d.rollback,['admin-environment-write','retain-dual-read']);assert.ok(m.metrics.includes('flag_tenant_fallback_total'));assert.match(doc,/retaining dual-read/);"),
];
const FLAG_SPEC: FamilySpec = FamilySpec {
    name: "feature-flag-tenant-scope",
    files: FLAG_FILES,
    checks: FLAG_CHECKS,
};
family_functions!(seed_flag, apply_flag, verify_flag, FLAG_SPEC);
const FLAG_EVIDENCE: &[Evidence] = &[
    Evidence { id:"PROJECT-FLAG-FALLBACK", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"Tenant-scoped flags fall back to the environment value; the evaluator must dual-read before any tenant writes." },
    Evidence { id:"ORG-FLAG-ROLLBACK", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Rollback returns admin writes to environment scope while retaining tenant/environment dual-read." },
    Evidence { id:"SCOUT-FLAG-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Observed graph includes schema, evaluator, admin migration, SDK consumer, and fallback/miss metrics." },
    Evidence { id:"ORACLE-FLAG", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Implement tenant schema and fallback, migration and SDK, dual-read config, evaluator-first rollout, safe rollback, and metrics." },
    Evidence { id:"STALE-FLAG-TENANT", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Old draft reads tenant scope only and treats missing values as false." },
    Evidence { id:"CONFLICT-FLAG-DROP", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Unapproved migration deletes environment values after tenant seeding." },
];
const FLAG_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "tenant_scope_schema",
        required_all: &["schema.mjs", "environment", "tenant", "key"],
        required_any: &[],
        expectation: "plan defines stable tenant-scoped keys",
    },
    SemanticPlanCheck {
        id: "ordered_scope_fallback",
        required_all: &["evaluate.mjs", "tenant first", "environment", "false"],
        required_any: &[],
        expectation: "plan defines ordered fallback semantics",
    },
    SemanticPlanCheck {
        id: "migration_and_sdk",
        required_all: &["flagmigration.mjs", "sdk", "fallbackscope"],
        required_any: &[],
        expectation: "plan updates migration producer and SDK consumer",
    },
    SemanticPlanCheck {
        id: "dual_read_configuration",
        required_all: &["flag-scope.json", "writescope", "fallbackscope", "dualread"],
        required_any: &[],
        expectation: "plan updates executable scope configuration",
    },
    SemanticPlanCheck {
        id: "evaluator_first_rollout",
        required_all: &[
            "evaluator-first",
            "admin-environment-write",
            "retain-dual-read",
            "flag_tenant_fallback_total",
        ],
        required_any: &[],
        expectation: "plan encodes rollout rollback and observability",
    },
];

const SHARD_FILES: &[(&str, &str, &str)] = &[
    ("repos/database/src/router.mjs", "export const shardFor=()=>0;\n", "export const shardFor=(tenant,map)=>map[tenant]??0; export const previousShardFor=(tenant,previous)=>previous[tenant]??0;\n"),
    ("repos/database/src/writer.mjs", "export const targets=(tenant)=>[0];\n", "export const targets=(tenant,current,previous,dual)=>dual?[previous[tenant]??0,current[tenant]??0]:[current[tenant]??0];\n"),
    ("repos/api/src/tenantReader.mjs", "export const read=async(store,id)=>store[0].get(id);\n", "export const read=async(store,id,current,previous)=>await store[current].get(id)??await store[previous].get(id);\n"),
    ("repos/database/src/rebalance.mjs", "export const next=()=>0;\n", "export const next=(checkpoint,batch)=>({from:checkpoint,to:checkpoint+batch,checkpoint:checkpoint+batch});\n"),
    ("config/shard-map.json", "{\"version\":1,\"tenants\":{\"t1\":0}}\n", "{\"version\":2,\"previousVersion\":1,\"tenants\":{\"t1\":1},\"previous\":{\"t1\":0},\"dualWrite\":true,\"batchSize\":250}\n"),
    ("deploy/production/shard-rebalance.json", "{\"order\":[\"map\",\"copy\"]}\n", "{\"order\":[\"reader-fallback\",\"dual-write\",\"copy-checkpoints\",\"verify-counts\",\"map-v2\"],\"rollback\":[\"map-v1\",\"retain-dual-write\"]}\n"),
    ("repos/observability/src/shardMetrics.mjs", "export const metrics=['db_query_total'];\n", "export const metrics=['db_query_total','shard_dual_write_error_total','shard_rebalance_lag_rows'];\n"),
    ("docs/shard-rebalance.md", "Move tenant rows during maintenance.\n", "Deploy reader fallback and dual-write, copy in checkpoints of 250, verify counts, then activate map v2. Roll back to map v1 while retaining dual-write.\n"),
];
const SHARD_CHECKS: &[(&str, &str)] = &[
    ("versioned_shard_maps", "const r=await load('repos/database/src/router.mjs');assert.equal(r.shardFor('t1',{t1:1}),1);assert.equal(r.previousShardFor('t1',{t1:0}),0);"),
    ("dual_write_paths", "const w=await load('repos/database/src/writer.mjs');assert.deepEqual(w.targets('t1',{t1:1},{t1:0},true),[0,1]);assert.deepEqual(w.targets('t1',{t1:1},{t1:0},false),[1]);"),
    ("reader_fallback", "const r=await load('repos/api/src/tenantReader.mjs');let store={0:new Map([['x','old']]),1:new Map()};assert.equal(await r.read(store,'x',1,0),'old');store={0:new Map([['x','old']]),1:new Map([['x','new']])};assert.equal(await r.read(store,'x',1,0),'new');"),
    ("checkpointed_copy", "const b=await load('repos/database/src/rebalance.mjs');assert.deepEqual(b.next(250,250),{from:250,to:500,checkpoint:500});const fs=await import('node:fs/promises');const c=JSON.parse(await fs.readFile(join(root,'config/shard-map.json'),'utf8'));assert.equal(c.batchSize,250);assert.equal(c.dualWrite,true);"),
    ("verified_map_cutover", "const fs=await import('node:fs/promises');const d=JSON.parse(await fs.readFile(join(root,'deploy/production/shard-rebalance.json'),'utf8'));const doc=await fs.readFile(join(root,'docs/shard-rebalance.md'),'utf8');const m=await load('repos/observability/src/shardMetrics.mjs');assert.equal(d.order.at(-1),'map-v2');assert.deepEqual(d.rollback,['map-v1','retain-dual-write']);assert.ok(m.metrics.includes('shard_rebalance_lag_rows'));assert.match(doc,/verify counts/);"),
];
const SHARD_SPEC: FamilySpec = FamilySpec {
    name: "database-shard-rebalance",
    files: SHARD_FILES,
    checks: SHARD_CHECKS,
};
family_functions!(seed_shard, apply_shard, verify_shard, SHARD_SPEC);
const SHARD_EVIDENCE: &[Evidence] = &[
    Evidence { id:"PROJECT-SHARD-DUAL", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"Tenant rebalances preserve versioned old/new maps, reader fallback, dual-write, and resumable copy checkpoints of 250 rows." },
    Evidence { id:"ORG-SHARD-CUTOVER", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Activate map v2 only after count verification; rollback activates map v1 while retaining dual-write." },
    Evidence { id:"SCOUT-SHARD-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Observed graph includes API reader, database router/writer, rebalance worker, both shards, and lag/error telemetry." },
    Evidence { id:"ORACLE-SHARD", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Implement versioned maps, dual writer, reader fallback, checkpoint copy, verified cutover, rollback, and metrics." },
    Evidence { id:"STALE-SHARD-MAINT", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Old runbook stops all writes and copies the full shard at once." },
    Evidence { id:"CONFLICT-SHARD-NEW", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Unapproved shortcut writes only the new shard before copy verification." },
];
const SHARD_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "versioned_shard_maps",
        required_all: &["router.mjs", "version 2", "previous", "map"],
        required_any: &[],
        expectation: "plan retains versioned current and previous maps",
    },
    SemanticPlanCheck {
        id: "dual_write_paths",
        required_all: &["writer.mjs", "dual-write", "old", "new"],
        required_any: &[],
        expectation: "plan writes both shards during migration",
    },
    SemanticPlanCheck {
        id: "reader_fallback",
        required_all: &["tenantreader.mjs", "fallback", "current", "previous"],
        required_any: &[],
        expectation: "plan keeps old-shard read fallback",
    },
    SemanticPlanCheck {
        id: "checkpointed_copy",
        required_all: &["rebalance.mjs", "checkpoint", "250"],
        required_any: &[],
        expectation: "plan makes copy resumable and bounded",
    },
    SemanticPlanCheck {
        id: "verified_map_cutover",
        required_all: &[
            "verify counts",
            "map-v2",
            "map-v1",
            "retain-dual-write",
            "shard_rebalance_lag_rows",
        ],
        required_any: &[],
        expectation: "plan encodes verified cutover rollback and metrics",
    },
];

const TEMPLATE_FILES: &[(&str, &str, &str)] = &[
    ("repos/templates/src/registry.mjs", "export const active={receipt:1};\n", "export const active={receipt:2}; export const versions={receipt:[1,2]};\n"),
    ("repos/templates/src/render.mjs", "export const render=(templates,name,data)=>templates[name](data);\n", "export const render=(templates,name,version,data)=>{const chosen=templates[name]?.[version]??templates[name]?.[1];if(!chosen)throw new Error('template missing');return chosen(data);};\n"),
    ("repos/email/src/send.mjs", "export const templateVersion=1;\n", "export const templateVersion=2; export const fallbackVersion=1;\n"),
    ("repos/push/src/send.mjs", "export const templateVersion=1;\n", "export const templateVersion=2; export const fallbackVersion=1;\n"),
    ("config/template-rollout.json", "{\"receipt\":1}\n", "{\"receipt\":{\"write\":2,\"fallback\":1,\"dualRenderPercent\":10}}\n"),
    ("deploy/production/templates.json", "{\"order\":[\"email\",\"registry\"]}\n", "{\"order\":[\"publish-v2\",\"renderer-fallback\",\"email\",\"push\",\"activate-v2\"],\"rollback\":[\"activate-v1\",\"retain-v2-assets\"]}\n"),
    ("repos/observability/src/templateMetrics.mjs", "export const metrics=['template_render_total'];\n", "export const metrics=['template_render_total','template_fallback_total','template_render_mismatch_total'];\n"),
    ("docs/template-versioning.md", "Replace templates in place.\n", "Publish immutable v2 assets, retain v1 fallback, dual-render ten percent, then activate v2. Roll back by activating v1 without deleting v2 assets.\n"),
];
const TEMPLATE_CHECKS: &[(&str, &str)] = &[
    ("immutable_versions", "const r=await load('repos/templates/src/registry.mjs');assert.deepEqual(r.versions.receipt,[1,2]);assert.equal(r.active.receipt,2);"),
    ("renderer_fallback", "const r=await load('repos/templates/src/render.mjs');const t={receipt:{1:x=>`v1:${x.id}`,2:x=>`v2:${x.id}`}};assert.equal(r.render(t,'receipt',2,{id:7}),'v2:7');assert.equal(r.render({receipt:{1:t.receipt[1]}},'receipt',2,{id:7}),'v1:7');"),
    ("all_delivery_consumers", "const e=await load('repos/email/src/send.mjs');const p=await load('repos/push/src/send.mjs');for(const m of [e,p]){assert.equal(m.templateVersion,2);assert.equal(m.fallbackVersion,1)}"),
    ("bounded_dual_render", "const fs=await import('node:fs/promises');const c=JSON.parse(await fs.readFile(join(root,'config/template-rollout.json'),'utf8'));assert.equal(c.receipt.write,2);assert.equal(c.receipt.fallback,1);assert.equal(c.receipt.dualRenderPercent,10);"),
    ("asset_first_rollout", "const fs=await import('node:fs/promises');const d=JSON.parse(await fs.readFile(join(root,'deploy/production/templates.json'),'utf8'));const doc=await fs.readFile(join(root,'docs/template-versioning.md'),'utf8');const m=await load('repos/observability/src/templateMetrics.mjs');assert.equal(d.order[0],'publish-v2');assert.deepEqual(d.rollback,['activate-v1','retain-v2-assets']);assert.ok(m.metrics.includes('template_render_mismatch_total'));assert.match(doc,/without deleting v2 assets/);"),
];
const TEMPLATE_SPEC: FamilySpec = FamilySpec {
    name: "notification-template-versioning",
    files: TEMPLATE_FILES,
    checks: TEMPLATE_CHECKS,
};
family_functions!(
    seed_template,
    apply_template,
    verify_template,
    TEMPLATE_SPEC
);
const TEMPLATE_EVIDENCE: &[Evidence] = &[
    Evidence { id:"PROJECT-TEMPLATE-IMMUTABLE", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"Template assets are immutable by version; email and push retain v1 fallback while moving receipt writes to v2." },
    Evidence { id:"ORG-TEMPLATE-CANARY", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Dual-render ten percent before activation; rollback activates v1 but retains published v2 assets." },
    Evidence { id:"SCOUT-TEMPLATE-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Observed graph includes registry, renderer, email and push consumers, plus fallback and mismatch metrics." },
    Evidence { id:"ORACLE-TEMPLATE", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Implement immutable v1/v2 registry, renderer fallback, both consumers, bounded dual-render, asset-first rollout, rollback, and metrics." },
    Evidence { id:"STALE-TEMPLATE-REPLACE", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Old process overwrites template v1 assets in place." },
    Evidence { id:"CONFLICT-TEMPLATE-DELETE", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Unapproved cleanup deletes v1 immediately after publishing v2." },
];
const TEMPLATE_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "immutable_versions",
        required_all: &["registry.mjs", "immutable", "v1", "v2"],
        required_any: &[],
        expectation: "plan retains immutable v1 and v2 assets",
    },
    SemanticPlanCheck {
        id: "renderer_fallback",
        required_all: &["render.mjs", "fallback", "version 1"],
        required_any: &[],
        expectation: "plan makes renderer fallback explicit",
    },
    SemanticPlanCheck {
        id: "all_delivery_consumers",
        required_all: &["email", "push", "templateversion", "fallbackversion"],
        required_any: &[],
        expectation: "plan updates every observed delivery consumer",
    },
    SemanticPlanCheck {
        id: "bounded_dual_render",
        required_all: &["dual-render", "ten percent", "write 2", "fallback 1"],
        required_any: &[],
        expectation: "plan encodes a bounded canary",
    },
    SemanticPlanCheck {
        id: "asset_first_rollout",
        required_all: &[
            "publish-v2",
            "activate-v1",
            "retain-v2-assets",
            "template_render_mismatch_total",
        ],
        required_any: &[],
        expectation: "plan encodes asset-first rollout rollback and metrics",
    },
];

pub fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            id: "feature-flag-tenant-scope",
            task: "Migrate feature flags from environment-only to tenant scope without changing existing evaluations. Implement ordered fallback, update admin and SDK, dual-read before tenant writes, preserve rollback, add metrics, and run visible tests.",
            required_plan_terms: &["schema.mjs","evaluate.mjs","flagMigration.mjs","flags.mjs","flag-scope.json","flagMetrics.mjs","flag-scope.md","rollback"],
            semantic_plan_checks: FLAG_SEMANTICS,
            required_evidence: &["PROJECT-FLAG-FALLBACK","ORG-FLAG-ROLLBACK","SCOUT-FLAG-GRAPH"],
            forbidden_evidence: &["STALE-FLAG-TENANT","CONFLICT-FLAG-DROP"],
            oracle_plan: "Use PROJECT-FLAG-FALLBACK, ORG-FLAG-ROLLBACK, and SCOUT-FLAG-GRAPH. schema.mjs adds environment and tenant keys. evaluate.mjs checks tenant first, then environment, then false. flagMigration.mjs seeds tenant values and SDK flags.mjs sends tenant with fallbackScope environment. flag-scope.json sets writeScope tenant, fallbackScope environment, and dualRead. Use evaluator-first rollout before SDK and admin writes; rollback admin-environment-write and retain-dual-read. flagMetrics.mjs adds flag_tenant_fallback_total and flag-scope.md documents the order.",
            evidence: evidence(FLAG_EVIDENCE),
            seed: seed_flag,
            verify: verify_flag,
            reference_apply: apply_flag,
        },
        Scenario {
            id: "database-shard-rebalance",
            task: "Rebalance tenant t1 from shard 0 to shard 1 without downtime. Keep versioned maps, dual writes, read fallback, resumable copy checkpoints, verified cutover and rollback, add lag/error metrics, and run visible tests.",
            required_plan_terms: &["router.mjs","writer.mjs","tenantReader.mjs","rebalance.mjs","shard-map.json","shard-rebalance.json","shardMetrics.mjs","shard-rebalance.md"],
            semantic_plan_checks: SHARD_SEMANTICS,
            required_evidence: &["PROJECT-SHARD-DUAL","ORG-SHARD-CUTOVER","SCOUT-SHARD-GRAPH"],
            forbidden_evidence: &["STALE-SHARD-MAINT","CONFLICT-SHARD-NEW"],
            oracle_plan: "Use PROJECT-SHARD-DUAL, ORG-SHARD-CUTOVER, and SCOUT-SHARD-GRAPH. router.mjs retains previous map version 1 and current version 2. writer.mjs dual-writes old and new shards; tenantReader.mjs reads current with previous fallback. rebalance.mjs checkpoints every 250 rows and shard-map.json keeps dualWrite. Deploy reader fallback, dual-write, copy checkpoints, verify counts, then map-v2. Rollback map-v1 and retain-dual-write. shardMetrics.mjs adds shard_rebalance_lag_rows and the runbook documents cutover.",
            evidence: evidence(SHARD_EVIDENCE),
            seed: seed_shard,
            verify: verify_shard,
            reference_apply: apply_shard,
        },
        Scenario {
            id: "notification-template-versioning",
            task: "Roll receipt templates from v1 to v2 across registry, renderer, email, and push without breaking fallback. Use a bounded dual-render canary, asset-first rollout, safe rollback, mismatch metrics, and run visible tests.",
            required_plan_terms: &["registry.mjs","render.mjs","email","push","template-rollout.json","templates.json","templateMetrics.mjs","template-versioning.md"],
            semantic_plan_checks: TEMPLATE_SEMANTICS,
            required_evidence: &["PROJECT-TEMPLATE-IMMUTABLE","ORG-TEMPLATE-CANARY","SCOUT-TEMPLATE-GRAPH"],
            forbidden_evidence: &["STALE-TEMPLATE-REPLACE","CONFLICT-TEMPLATE-DELETE"],
            oracle_plan: "Use PROJECT-TEMPLATE-IMMUTABLE, ORG-TEMPLATE-CANARY, and SCOUT-TEMPLATE-GRAPH. registry.mjs retains immutable v1 and v2. render.mjs renders requested version with fallback version 1. Email and push use templateVersion 2 and fallbackVersion 1. template-rollout.json uses write 2, fallback 1, and dual-render ten percent. Use asset-first publish-v2 then renderer and consumers; rollback activate-v1 and retain-v2-assets. templateMetrics.mjs adds template_render_mismatch_total and the runbook preserves assets.",
            evidence: evidence(TEMPLATE_EVIDENCE),
            seed: seed_template,
            verify: verify_template,
            reference_apply: apply_template,
        },
    ]
}
