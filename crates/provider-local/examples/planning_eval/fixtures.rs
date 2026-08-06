use crate::fixture_support::{push_node, seed_repository_history, seed_scaffold, write};
use crate::model::{
    Evidence, EvidenceRole, EvidenceSource, Scenario, SemanticPlanCheck, Verification,
};
use std::path::Path;
use std::process::Command;

fn seed_audit(root: &Path) -> Result<(), String> {
    seed_scaffold(root, "regional-audit-export")?;
    write(root, "repos/api/src/auditExports.mjs", "export const route='/v1/audit-export'; export function createAuditExport(region){ return {route,job:{region,format:'csv'}}; }\n")?;
    write(
        root,
        "repos/worker/src/exportJob.mjs",
        "export async function run(job,storage){ return storage.us.put('audit.csv'); }\n",
    )?;
    write(root, "repos/web/src/auditDownload.mjs", "export const endpoint='/v1/audit-export'; export const statusUrl=(id)=>`${endpoint}/${id}`;\n")?;
    write(
        root,
        "repos/infra/residency.json",
        "{\"buckets\":{\"us\":\"audit-us\"}}\n",
    )?;
    write(
        root,
        "repos/api/src/routes.mjs",
        "export const routes=['/v1/audit-export'];\n",
    )?;
    write(
        root,
        "repos/worker/src/metrics.mjs",
        "export const metric='audit_export_completed_total';\n",
    )?;
    write(
        root,
        "repos/api/src/auditValidation.mjs",
        "export const validateAuditJob=(job)=>job.format==='csv'&&job.region==='us';\n",
    )?;
    write(
        root,
        "deploy/production/audit-export.json",
        "{\"order\":[\"api\",\"worker\"],\"rollback\":[\"worker\",\"api\"]}\n",
    )?;
    write(root, "tests/audit-visible.test.mjs", "import test from 'node:test'; import assert from 'node:assert/strict'; import {createAuditExport} from '../repos/api/src/auditExports.mjs'; test('legacy export remains available',()=>assert.equal(createAuditExport('us').job.region,'us'));\n")?;
    seed_repository_history(root, "Preserve the legacy audit export boundary")
}

fn reference_audit(root: &Path) -> Result<(), String> {
    write(root, "repos/api/src/auditExports.mjs", "export const route='/v2/audit-exports'; export function createAuditExport(region){ return {route,job:{region,format:'ndjson',schemaVersion:2}}; }\n")?;
    write(root, "repos/worker/src/exportJob.mjs", "export async function run(job,storage){ const target=job.region==='eu'?storage.eu:storage.us; return target.put(`audit.${job.format}`); }\n")?;
    write(root, "repos/web/src/auditDownload.mjs", "export const endpoint='/v2/audit-exports'; export const statusUrl=(id)=>`${endpoint}/${id}`; export async function pollUntilReady(id,fetchStatus){ for(let n=0;n<5;n++){const value=await fetchStatus(statusUrl(id));if(value.status==='ready')return value;} throw new Error('export not ready'); }\n")?;
    write(root, "repos/infra/residency.json", "{\"buckets\":{\"us\":\"audit-us\",\"eu\":\"audit-eu-west-1\"},\"coverage\":{\"production-eu\":\"verified\",\"staging-eu\":\"unreachable\"}}\n")?;
    write(
        root,
        "repos/api/src/routes.mjs",
        "export const routes=['/v1/audit-export','/v2/audit-exports'];\n",
    )?;
    write(root, "repos/worker/src/metrics.mjs", "export const metrics=['audit_export_completed_total','audit_export_residency_violation_total'];\n")?;
    write(root, "repos/api/src/auditValidation.mjs", "export const validateAuditJob=(job)=>['us','eu'].includes(job.region)&&job.format==='ndjson'&&job.schemaVersion===2;\n")?;
    write(root, "deploy/production/audit-export.json", "{\"order\":[\"storage\",\"worker\",\"api\",\"web\"],\"rollback\":[\"api-v2-route\",\"worker\",\"storage\"]}\n")
}

fn verify_audit(root: &Path) -> Verification {
    let mut out = Verification::default();
    push_node(&mut out, root, "api_v2_job", "const m=await load('repos/api/src/auditExports.mjs'); const validation=await load('repos/api/src/auditValidation.mjs'); const create=m.createAuditExportV2??m.createAuditExport; const value=create('eu'); const job=value.job??value; assert.equal(value.route??m.routeV2??m.v2Route,'/v2/audit-exports'); assert.equal(job.region,'eu'); assert.equal(job.format,'ndjson'); assert.equal(job.schemaVersion,2); assert.equal(validation.validateAuditJob(job),true); assert.equal(validation.validateAuditJob({...job,format:'csv'}),false);");
    push_node(&mut out, root, "worker_residency", "const m=await load('repos/worker/src/exportJob.mjs'); const calls=[]; const storage={us:{put:x=>calls.push(['us',x])},eu:{put:x=>calls.push(['eu',x])}}; await m.run({schemaVersion:2,region:'eu',format:'ndjson'},storage); await m.run({schemaVersion:2,region:'us',format:'ndjson'},storage); assert.deepEqual(calls,[['eu','audit.ndjson'],['us','audit.ndjson']]);");
    push_node(&mut out, root, "web_polling", "const m=await load('repos/web/src/auditDownload.mjs'); let n=0; const value=await m.pollUntilReady('42',async url=>{assert.equal(url,'/v2/audit-exports/42');return {status:++n===2?'ready':'pending'};}); assert.equal(value?.status??value,'ready');");
    push_node(&mut out, root, "infra_and_coverage", "const fs=await import('node:fs/promises'); const cfg=JSON.parse(await fs.readFile(join(root,'repos/infra/residency.json'),'utf8')); const deploy=JSON.parse(await fs.readFile(join(root,'deploy/production/audit-export.json'),'utf8')); assert.equal(cfg.buckets.eu,'audit-eu-west-1'); const gap=cfg.coverage?.['staging-eu']==='unreachable'||cfg.coverageGaps?.some?.(value=>value==='staging-eu'||(value?.scope==='staging-eu'&&(value.reachable===false||value.status==='unreachable'))); assert.equal(gap,true); assert.deepEqual(deploy.order,['storage','worker','api','web']); assert.equal(deploy.rollback[0],'api-v2-route');");
    push_node(&mut out, root, "compat_and_metrics", "const routes=await load('repos/api/src/routes.mjs'); const metrics=await load('repos/worker/src/metrics.mjs'); assert.ok(routes.routes.includes('/v1/audit-export')); assert.ok(routes.routes.includes('/v2/audit-exports')); assert.ok(Object.values(metrics).flat().includes('audit_export_residency_violation_total'));");
    out
}

fn seed_event(root: &Path) -> Result<(), String> {
    seed_scaffold(root, "event-envelope-v2")?;
    write(
        root,
        "repos/events/src/envelope.mjs",
        "export const publish=(id,payload)=>({id,payload});\n",
    )?;
    write(
        root,
        "repos/billing/src/consumer.mjs",
        "export const consume=(event)=>event.payload.amount;\n",
    )?;
    write(
        root,
        "repos/analytics/src/consumer.py",
        "def consume(event):\n    return {'payload': event['payload']}\n",
    )?;
    write(
        root,
        "repos/notifications/src/consumer.mjs",
        "export const subject=(event)=>event.payload.subject;\n",
    )?;
    write(
        root,
        "config/event-rollout.json",
        "{\"producerV2\":false,\"order\":[\"producer\",\"consumers\"]}\n",
    )?;
    write(
        root,
        "docs/event-contract.md",
        "Unversioned payload only.\n",
    )?;
    write(
        root,
        "repos/events/src/schema.mjs",
        "export const versions=[1]; export const fields={1:['id','payload']};\n",
    )?;
    write(
        root,
        "deploy/production/event-migration.json",
        "{\"phase\":\"single-cutover\",\"rollback\":[]}\n",
    )?;
    write(root, "tests/event-visible.test.mjs", "import test from 'node:test'; import assert from 'node:assert/strict'; import {consume} from '../repos/billing/src/consumer.mjs'; test('v1 remains readable',()=>assert.equal(consume({payload:{amount:3}}),3));\n")?;
    seed_repository_history(root, "Keep event producers backward compatible")
}

fn reference_event(root: &Path) -> Result<(), String> {
    write(root, "repos/events/src/envelope.mjs", "export const publishV1=(id,payload)=>({id,payload}); export const publishV2=(id,payload,occurredAt)=>({id,payload,schemaVersion:2,occurredAt});\n")?;
    write(root, "repos/billing/src/consumer.mjs", "export const consume=(event)=>({amount:event.payload.amount,version:event.schemaVersion??1});\n")?;
    write(root, "repos/analytics/src/consumer.py", "def consume(event):\n    return {'payload': event['payload'], 'schema_version': event.get('schemaVersion', 1), 'occurred_at': event.get('occurredAt')}\n")?;
    write(root, "repos/notifications/src/consumer.mjs", "export const subject=(event)=>({subject:event.payload.subject,version:event.schemaVersion??1});\n")?;
    write(root, "config/event-rollout.json", "{\"producerV2\":false,\"order\":[\"billing-consumer\",\"analytics-consumer\",\"notifications-consumer\",\"producer\"],\"rollback\":[\"producer\",\"consumers\"]}\n")?;
    write(root, "docs/event-contract.md", "EventV2 is additive: schemaVersion=2 and occurredAt. Deploy dual-read consumers first, then enable the producer. Roll back the producer before consumers.\n")?;
    write(root, "repos/events/src/schema.mjs", "export const versions=[1,2]; export const fields={1:['id','payload'],2:['id','payload','schemaVersion','occurredAt']};\n")?;
    write(root, "deploy/production/event-migration.json", "{\"order\":[\"billing-consumer\",\"analytics-consumer\",\"notifications-consumer\",\"producer\"],\"rollback\":[\"producer\",\"consumers\"],\"compatibility\":\"dual-read\"}\n")
}

fn verify_event(root: &Path) -> Verification {
    let mut out = Verification::default();
    push_node(&mut out, root, "additive_envelopes", "const m=await load('repos/events/src/envelope.mjs'); const schema=await load('repos/events/src/schema.mjs'); assert.deepEqual(m.publishV1('1',{x:1}),{id:'1',payload:{x:1}}); assert.deepEqual(m.publishV2('2',{x:2},'2026-01-01'),{id:'2',payload:{x:2},schemaVersion:2,occurredAt:'2026-01-01'}); assert.deepEqual(schema.versions,[1,2]); assert.ok(schema.fields[2].includes('occurredAt'));");
    push_node(&mut out, root, "billing_dual_read", "const m=await load('repos/billing/src/consumer.mjs'); assert.equal(m.consume({payload:{amount:3}}).version,1); assert.equal(m.consume({payload:{amount:4},schemaVersion:2}).version,2);");
    let py = Command::new("python3").arg("-c").arg("import importlib.util,os; p=os.path.join(os.environ['EVAL_ROOT'],'repos/analytics/src/consumer.py'); s=importlib.util.spec_from_file_location('c',p);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);assert m.consume({'payload':{},'schemaVersion':2,'occurredAt':'t'})=={'payload':{},'schema_version':2,'occurred_at':'t'};assert m.consume({'payload':{}})['schema_version']==1").env("EVAL_ROOT", root).output();
    out.push(
        "analytics_dual_read",
        py.as_ref().is_ok_and(|x| x.status.success()),
        py.ok()
            .map(|x| String::from_utf8_lossy(&x.stderr).to_string())
            .unwrap_or_else(|| "python unavailable".into()),
    );
    push_node(&mut out, root, "third_consumer", "const m=await load('repos/notifications/src/consumer.mjs'); assert.equal(m.subject({payload:{subject:'x'}}).version,1); assert.equal(m.subject({payload:{subject:'x'},schemaVersion:2}).version,2);");
    push_node(&mut out, root, "rollout_contract", "const fs=await import('node:fs/promises'); const cfg=JSON.parse(await fs.readFile(join(root,'config/event-rollout.json'),'utf8')); const deploy=JSON.parse(await fs.readFile(join(root,'deploy/production/event-migration.json'),'utf8')); assert.deepEqual(cfg.order,['billing-consumer','analytics-consumer','notifications-consumer','producer']); assert.deepEqual(cfg.rollback,['producer','consumers']); assert.deepEqual(deploy.order,cfg.order); assert.equal(deploy.compatibility,'dual-read'); const doc=await fs.readFile(join(root,'docs/event-contract.md'),'utf8'); assert.match(doc,/dual-read consumers first/);");
    out
}

fn seed_preferences(root: &Path) -> Result<(), String> {
    seed_scaffold(root, "collaboration-preferences")?;
    write(root, "repos/core/src/preferences.mjs", "export const permissionModes=['ask','auto','full','plan']; export const key='permission-mode';\n")?;
    write(
        root,
        "repos/cloud/src/preferences.mjs",
        "export const save=(store,value)=>store.set('permission-mode',value);\n",
    )?;
    write(
        root,
        "repos/desktop/src/migrate.mjs",
        "export const migrate=(legacy)=>({permissionMode:legacy});\n",
    )?;
    write(
        root,
        "repos/mobile/src/settings.mjs",
        "export const modes=['ask','auto','full','plan'];\n",
    )?;
    write(
        root,
        "repos/sync/src/merge.mjs",
        "export const merge=(local,remote)=>({...remote});\n",
    )?;
    write(
        root,
        "repos/core/src/preferenceSchema.mjs",
        "export const fields=['permissionMode']; export const version=1;\n",
    )?;
    write(
        root,
        "repos/cloud/src/read.mjs",
        "export const read=(store)=>({permissionMode:store.get('permission-mode')});\n",
    )?;
    write(
        root,
        "config/preference-rollout.json",
        "{\"version\":1,\"write\":\"permission-mode\"}\n",
    )?;
    write(root, "tests/preferences-visible.test.mjs", "import test from 'node:test'; import assert from 'node:assert/strict'; import {migrate} from '../repos/desktop/src/migrate.mjs'; test('legacy ask survives',()=>assert.equal(migrate('ask').permissionMode,'ask'));\n")?;
    seed_repository_history(root, "Retain user choices across preference migrations")
}

fn reference_preferences(root: &Path) -> Result<(), String> {
    write(root, "repos/core/src/preferences.mjs", "export const permissionModes=['ask','auto','full']; export const collaborationModes=['default','plan']; export const permissionKey='permission-mode-v2'; export const collaborationKey='collaboration-mode';\n")?;
    write(root, "repos/cloud/src/preferences.mjs", "export const save=(store,value)=>{store.set('permission-mode-v2',value.permissionMode);store.set('collaboration-mode',value.collaborationMode);};\n")?;
    write(root, "repos/desktop/src/migrate.mjs", "export const migrate=(legacy)=>legacy==='plan'?{permissionMode:'ask',collaborationMode:'plan'}:{permissionMode:legacy??'ask',collaborationMode:'default'};\n")?;
    write(root, "repos/mobile/src/settings.mjs", "export const permissionModes=['ask','auto','full']; export const collaborationModes=['default','plan'];\n")?;
    write(root, "repos/sync/src/merge.mjs", "export const merge=(local,remote)=>({permissionMode:remote.permissionMode??local.permissionMode,collaborationMode:remote.collaborationMode??local.collaborationMode});\n")?;
    write(
        root,
        "repos/core/src/preferenceSchema.mjs",
        "export const fields=['permissionMode','collaborationMode']; export const version=2;\n",
    )?;
    write(root, "repos/cloud/src/read.mjs", "export const read=(store)=>({permissionMode:store.get('permission-mode-v2')??'ask',collaborationMode:store.get('collaboration-mode')??'default'});\n")?;
    write(root, "config/preference-rollout.json", "{\"version\":2,\"dualRead\":[\"permission-mode\",\"permission-mode-v2\"],\"write\":[\"permission-mode-v2\",\"collaboration-mode\"],\"rollback\":\"retain-v1-read\"}\n")
}

fn verify_preferences(root: &Path) -> Verification {
    let mut out = Verification::default();
    push_node(&mut out, root, "orthogonal_types", "const m=await load('repos/core/src/preferences.mjs'); const schema=await load('repos/core/src/preferenceSchema.mjs'); assert.deepEqual(m.permissionModes,['ask','auto','full']); assert.deepEqual(m.collaborationModes,['default','plan']); assert.equal(m.permissionKey,'permission-mode-v2'); assert.equal(schema.version,2); assert.deepEqual(schema.fields,['permissionMode','collaborationMode']);");
    push_node(&mut out, root, "legacy_migration", "const m=await load('repos/desktop/src/migrate.mjs'); assert.deepEqual(m.migrate('plan'),{permissionMode:'ask',collaborationMode:'plan'}); assert.deepEqual(m.migrate('full'),{permissionMode:'full',collaborationMode:'default'});");
    push_node(&mut out, root, "cloud_persistence", "const m=await load('repos/cloud/src/preferences.mjs'); const reader=await load('repos/cloud/src/read.mjs'); const values=new Map(); m.save(values,{permissionMode:'auto',collaborationMode:'plan'}); assert.equal(values.get('permission-mode-v2'),'auto'); assert.equal(values.get('collaboration-mode'),'plan'); assert.deepEqual(reader.read(values),{permissionMode:'auto',collaborationMode:'plan'});");
    push_node(&mut out, root, "mobile_split", "const m=await load('repos/mobile/src/settings.mjs'); assert.ok(!m.permissionModes.includes('plan')); assert.ok(m.collaborationModes.includes('plan'));");
    push_node(&mut out, root, "sync_partial_updates", "const m=await load('repos/sync/src/merge.mjs'); const fs=await import('node:fs/promises'); const rollout=JSON.parse(await fs.readFile(join(root,'config/preference-rollout.json'),'utf8')); assert.deepEqual(m.merge({permissionMode:'ask',collaborationMode:'plan'},{permissionMode:'full'}),{permissionMode:'full',collaborationMode:'plan'}); assert.deepEqual(rollout.dualRead,['permission-mode','permission-mode-v2']); assert.equal(rollout.rollback,'retain-v1-read');");
    out
}

const DISTRACTORS: &[(&str, EvidenceSource, &str)] = &[
    (
        "PROJECT-UI-OLD",
        EvidenceSource::Project,
        "Archived UI snapshot mentions a removed settings panel.",
    ),
    (
        "PROJECT-BUILD-01",
        EvidenceSource::Project,
        "The repository uses Node's built-in test runner for executable fixtures.",
    ),
    (
        "PROJECT-NAMING-03",
        EvidenceSource::Project,
        "Older modules used singular route names inconsistently.",
    ),
    (
        "PROJECT-CACHE-02",
        EvidenceSource::Project,
        "A cache experiment was abandoned after staging.",
    ),
    (
        "PROJECT-DOCS-06",
        EvidenceSource::Project,
        "Operational decisions should be documented near rollout config.",
    ),
    (
        "ORG-BRAND-01",
        EvidenceSource::Org,
        "Brand colors were approved for the marketing site.",
    ),
    (
        "ORG-RETENTION-03",
        EvidenceSource::Org,
        "General logs retain metadata for thirty days.",
    ),
    (
        "ORG-ACCESS-08",
        EvidenceSource::Org,
        "Production deployment access is restricted to release engineering.",
    ),
    (
        "ORG-FINANCE-05",
        EvidenceSource::Org,
        "Cost attribution labels are required on new cloud resources.",
    ),
    (
        "ORG-REVIEW-11",
        EvidenceSource::Org,
        "Cross-component migrations require named rollback ownership.",
    ),
    (
        "SCOUT-MARKETING-01",
        EvidenceSource::Scout,
        "The marketing site deploys independently from product services.",
    ),
    (
        "SCOUT-DEV-QUEUE",
        EvidenceSource::Scout,
        "A development-only queue has no production traffic.",
    ),
    (
        "SCOUT-ARCHIVE-02",
        EvidenceSource::Scout,
        "An archived repository still references the old package name.",
    ),
    (
        "SCOUT-COVERAGE-00",
        EvidenceSource::Scout,
        "One unrelated sandbox account is unreachable.",
    ),
    (
        "SCOUT-OBS-04",
        EvidenceSource::Scout,
        "Central metrics ingestion is reachable from all production services.",
    ),
    (
        "NOISE-FUTURE-01",
        EvidenceSource::Noise,
        "A future proposal considers a different message broker.",
    ),
    (
        "NOISE-FUTURE-02",
        EvidenceSource::Noise,
        "A design exercise proposes a dashboard redesign.",
    ),
    (
        "NOISE-FUTURE-03",
        EvidenceSource::Noise,
        "A draft suggests renaming all repositories next year.",
    ),
    (
        "NOISE-EXPERIMENT-04",
        EvidenceSource::Noise,
        "A sandbox-only experiment used different identifiers and never reached staging.",
    ),
];

fn evidence(base: &[Evidence]) -> Vec<Evidence> {
    let mut result = base.to_vec();
    result.extend(DISTRACTORS.iter().map(|(id, source, text)| Evidence {
        id,
        source: *source,
        role: EvidenceRole::Distractor,
        text,
    }));
    result
}

const AUDIT: &[Evidence] = &[
    Evidence { id:"PROJECT-AUDIT-01", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"The executable audit-export contract is v2 NDJSON with schemaVersion 2; preserve the v1 route during rollout. The worker writes each v2 artifact under the exact object name audit.ndjson. The web compatibility surface is pollUntilReady(id, fetchStatus), where status ready is the successful terminal state. The violation metric is audit_export_residency_violation_total." },
    Evidence { id:"ORG-RESIDENCY-04", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Effective 2026-06-01, EU audit artifacts must remain in the audit-eu-west-1 resource in eu-west-1; this supersedes the 2024 global-US exception." },
    Evidence { id:"SCOUT-AUDIT-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Production graph and rollout labels are storage -> worker -> API -> web. Web polls API. staging-eu coverage is unreachable and must be disclosed. Rollback disables api-v2-route first." },
    Evidence { id:"ORACLE-AUDIT", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Implement v2 job, regional worker, polling client, buckets/coverage, compatibility route, and violation metric." },
    Evidence { id:"STALE-AUDIT-V1", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"2024 policy: all exports are CSV in the US bucket." },
    Evidence { id:"CONFLICT-AUDIT-US", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Unapproved cost note: route EU exports to the US bucket." },
];
const EVENT: &[Evidence] = &[
    Evidence { id:"PROJECT-EVENT-12", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"EventV2 must be additive and every consumer must dual-read v1 and v2." },
    Evidence { id:"ORG-ROLLOUT-02", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Deploy all dual-read consumers before the producer; producer is the first rollback step." },
    Evidence { id:"SCOUT-EVENT-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Observed production consumers are billing, analytics, and notifications; the inventory is complete." },
    Evidence { id:"ORACLE-EVENT", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Add v1/v2 publishers, three dual readers, and consumer-first rollout configuration." },
    Evidence { id:"STALE-EVENT-FLAG", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Old proposal: flip producer and consumers simultaneously." },
    Evidence { id:"CONFLICT-EVENT-BREAK", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Delete EventV1 immediately." },
];
const PREFS: &[Evidence] = &[
    Evidence { id:"PROJECT-PREF-07", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"Legacy plan migrates to permission ask plus collaboration plan; other legacy values retain permission and default collaboration." },
    Evidence { id:"ORG-SAFETY-09", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Plan collaboration never grants write permission; permission and collaboration persist independently." },
    Evidence { id:"SCOUT-PREF-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Core preference schema feeds cloud persistence, desktop migration, mobile settings, and partial sync merge." },
    Evidence { id:"ORACLE-PREF", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Split types/keys, migrate legacy values, persist both, update mobile, and preserve omitted sync fields." },
    Evidence { id:"STALE-PREF-DROP", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Old draft: reset every user to auto." },
    Evidence { id:"CONFLICT-PREF-FULL", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Map plan to full permission." },
];

const AUDIT_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "api_v2_job",
        required_all: &[
            "/v2/audit-exports",
            "ndjson",
            "schemaversion",
            "auditvalidation.mjs",
            "us/eu",
        ],
        required_any: &[],
        expectation: "plan specifies the executable and validated v2 NDJSON job contract",
    },
    SemanticPlanCheck {
        id: "worker_residency",
        required_all: &["storage.eu", "storage.us", "audit.ndjson"],
        required_any: &[],
        expectation: "plan preserves the exact regional worker output contract",
    },
    SemanticPlanCheck {
        id: "web_polling",
        required_all: &[],
        required_any: &[
            "polluntilready",
            "poll until ready",
            "until the job reaches",
        ],
        expectation: "plan specifies repeated status polling rather than only a URL helper",
    },
    SemanticPlanCheck {
        id: "infra_and_coverage",
        required_all: &[
            "audit-eu-west-1",
            "staging-eu",
            "unreachable",
            "storage",
            "worker",
            "api",
            "web",
            "api-v2-route",
        ],
        required_any: &[],
        expectation: "plan operationalizes residency plus deployment and rollback order",
    },
    SemanticPlanCheck {
        id: "compat_and_metrics",
        required_all: &[
            "/v1/audit-export",
            "/v2/audit-exports",
            "audit_export_residency_violation_total",
        ],
        required_any: &[],
        expectation: "plan preserves v1 and names the required violation metric",
    },
];

const EVENT_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "additive_envelopes",
        required_all: &["publishv1", "publishv2", "occurredat", "schema.mjs", "v1"],
        required_any: &[],
        expectation: "plan keeps separate publishers and a versioned additive schema",
    },
    SemanticPlanCheck {
        id: "billing_dual_read",
        required_all: &["billing", "schema", "version"],
        required_any: &[],
        expectation: "plan makes billing expose version-aware dual reads",
    },
    SemanticPlanCheck {
        id: "analytics_dual_read",
        required_all: &["analytics", "schema_version", "occurred_at"],
        required_any: &[],
        expectation: "plan defines the Python consumer stable dual-read result",
    },
    SemanticPlanCheck {
        id: "third_consumer",
        required_all: &["notifications", "schema", "version"],
        required_any: &[],
        expectation: "plan includes the third deployed consumer and its versioned result",
    },
    SemanticPlanCheck {
        id: "rollout_contract",
        required_all: &[
            "billing-consumer",
            "analytics-consumer",
            "notifications-consumer",
            "rollback",
            "producer",
            "event-migration.json",
            "dual-read",
        ],
        required_any: &[],
        expectation: "plan encodes both rollout surfaces and producer-first rollback",
    },
];

const PREFS_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "orthogonal_types",
        required_all: &[
            "permission-mode-v2",
            "permissionmodes",
            "collaborationmodes",
            "preferenceschema.mjs",
            "version 2",
            "default",
            "plan",
        ],
        required_any: &[],
        expectation: "plan defines orthogonal modes and the v2 key",
    },
    SemanticPlanCheck {
        id: "legacy_migration",
        required_all: &["legacy", "ask", "plan", "default", "full"],
        required_any: &[],
        expectation: "plan preserves non-plan legacy values with default collaboration",
    },
    SemanticPlanCheck {
        id: "cloud_persistence",
        required_all: &[
            "permission-mode-v2",
            "collaboration-mode",
            "cloud",
            "read.mjs",
            "safe defaults",
        ],
        required_any: &[],
        expectation: "plan persists both independent cloud keys",
    },
    SemanticPlanCheck {
        id: "mobile_split",
        required_all: &["mobile", "permissionmodes", "collaborationmodes"],
        required_any: &[],
        expectation: "plan splits both mobile selectors",
    },
    SemanticPlanCheck {
        id: "sync_partial_updates",
        required_all: &[
            "partial",
            "permissionmode",
            "collaborationmode",
            "merge",
            "dual-read",
            "rollback",
        ],
        required_any: &[],
        expectation: "plan preserves omitted fields during partial sync merges",
    },
];

pub fn scenarios() -> Vec<Scenario> {
    let mut scenarios = vec![
        Scenario { id:"regional-audit-export", task:"Implement regional audit-export v2 in this multi-component workspace. The API must create and validate NDJSON schemaVersion 2 jobs at `/v2/audit-exports`; keep the legacy route during rollout. The worker must select EU versus US storage, the web must poll status, infrastructure must record EU storage and any coverage gap, metrics must expose residency violations, and production deployment order and rollback must be executable. Run visible tests.", required_plan_terms:&["auditExports.mjs","auditValidation.mjs","exportJob.mjs","auditDownload.mjs","residency.json","routes.mjs","metrics.mjs","audit-export.json","rollback"], semantic_plan_checks:AUDIT_SEMANTICS, required_evidence:&["PROJECT-AUDIT-01","ORG-RESIDENCY-04","SCOUT-AUDIT-GRAPH"], forbidden_evidence:&["STALE-AUDIT-V1","CONFLICT-AUDIT-US"], oracle_plan:"Use PROJECT-AUDIT-01, ORG-RESIDENCY-04, and SCOUT-AUDIT-GRAPH. In auditExports.mjs add the /v2/audit-exports NDJSON schemaVersion job while retaining /v1/audit-export, and in auditValidation.mjs accept only schemaVersion 2 NDJSON jobs for us/eu. In exportJob.mjs route through storage.eu or storage.us and always write audit.ndjson. In auditDownload.mjs add pollUntilReady so the web polls until ready. In residency.json set audit-eu-west-1 and record staging-eu as unreachable. Keep both routes and expose audit_export_residency_violation_total. In deploy/production/audit-export.json order storage, worker, API, web and rollback the api-v2-route first.", evidence:evidence(AUDIT), seed:seed_audit, verify:verify_audit, reference_apply:reference_audit },
        Scenario { id:"rolling-event-v2", task:"Implement additive EventV2 across the producer and every deployed consumer. Preserve v1 reads, add schemaVersion 2 and occurredAt to the versioned schema, encode consumer-first rollout and producer-first rollback in both application and production deployment configuration, and run visible tests. Determine the full consumer set from available project and organizational knowledge/cartography.", required_plan_terms:&["envelope.mjs","schema.mjs","billing","analytics","notifications","event-rollout.json","event-migration.json","dual-read","rollback"], semantic_plan_checks:EVENT_SEMANTICS, required_evidence:&["PROJECT-EVENT-12","ORG-ROLLOUT-02","SCOUT-EVENT-GRAPH"], forbidden_evidence:&["STALE-EVENT-FLAG","CONFLICT-EVENT-BREAK"], oracle_plan:"Use PROJECT-EVENT-12, ORG-ROLLOUT-02, and SCOUT-EVENT-GRAPH. In envelope.mjs retain publishV1 and add publishV2 with schemaVersion 2 and occurredAt; schema.mjs must retain v1 fields and define the additive v2 fields. Make billing return an amount and version, analytics return payload plus schema_version and occurred_at, and notifications return subject plus version. Encode order billing-consumer, analytics-consumer, notifications-consumer, producer and rollback producer before consumers in event-rollout.json and deploy/production/event-migration.json with dual-read compatibility. Test both versions before enabling writes.", evidence:evidence(EVENT), seed:seed_event, verify:verify_event, reference_apply:reference_event },
        Scenario { id:"permission-collaboration-split", task:"Separate action permission from collaboration Plan Mode across core schema, cloud persistence and reads, desktop legacy migration, mobile settings, partial sync merges, and versioned rollout configuration. Legacy `plan` becomes permission `ask` plus collaboration `plan`; no path may grant write permission. Preserve other legacy values and omitted fields, retain v1 reads for rollback, then run visible tests.", required_plan_terms:&["preferences.mjs","preferenceSchema.mjs","read.mjs","migrate.mjs","settings.mjs","merge.mjs","preference-rollout.json","permission-mode-v2","collaboration-mode","partial"], semantic_plan_checks:PREFS_SEMANTICS, required_evidence:&["PROJECT-PREF-07","ORG-SAFETY-09","SCOUT-PREF-GRAPH"], forbidden_evidence:&["STALE-PREF-DROP","CONFLICT-PREF-FULL"], oracle_plan:"Use PROJECT-PREF-07, ORG-SAFETY-09, and SCOUT-PREF-GRAPH. In core preferences.mjs define permissionModes ask/auto/full, collaborationModes default/plan, permission-mode-v2, and collaboration-mode; preferenceSchema.mjs declares both fields at version 2. Desktop migrate.mjs maps legacy plan to permissionMode ask plus collaborationMode plan; legacy full and other values retain permission with collaboration default. Cloud preferences.mjs persists both keys independently and read.mjs returns both with safe defaults. Mobile settings.mjs exposes both permissionModes and collaborationModes. Sync merge.mjs performs a partial fieldwise merge so omitted fields survive. preference-rollout.json dual-reads permission-mode and permission-mode-v2, writes both v2 keys, and retains the v1 read for rollback.", evidence:evidence(PREFS), seed:seed_preferences, verify:verify_preferences, reference_apply:reference_preferences },
    ];
    scenarios.extend(crate::scenario_families::scenarios());
    scenarios.extend(crate::scenario_families_extra_a::scenarios());
    scenarios.extend(crate::scenario_families_extra_b::scenarios());
    scenarios
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture_support::apply_alternate_module_layout;

    #[test]
    fn seeds_are_complex_and_reference_implementations_pass() {
        let scenarios = scenarios();
        assert!(scenarios.len() >= 12);
        let domains = scenarios
            .iter()
            .map(Scenario::domain)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(domains.len() >= 4, "{domains:?}");
        assert!(!domains.contains("unclassified"));
        for scenario in scenarios {
            assert!(scenario.evidence.len() >= 25);
            let temp = tempfile::tempdir().unwrap();
            (scenario.seed)(temp.path()).unwrap();
            assert!(
                walkdir::WalkDir::new(temp.path())
                    .into_iter()
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_file())
                    .filter(|entry| {
                        entry
                            .path()
                            .strip_prefix(temp.path())
                            .ok()
                            .is_some_and(|path| {
                                !path.starts_with(".git") && !path.starts_with(".clark")
                            })
                    })
                    .count()
                    >= 60
            );
            assert!((scenario.verify)(temp.path()).score() < 0.4);
            (scenario.reference_apply)(temp.path()).unwrap();
            let changed = Command::new("git")
                .arg("-C")
                .arg(temp.path())
                .args(["diff", "--name-only", "HEAD"])
                .output()
                .unwrap();
            assert!(changed.status.success());
            let changed_count = String::from_utf8_lossy(&changed.stdout).lines().count();
            assert!(
                (8..=15).contains(&changed_count),
                "{} changed {changed_count} files",
                scenario.id
            );
            let result = (scenario.verify)(temp.path());
            assert_eq!(
                result.score(),
                1.0,
                "{}: {:?}",
                scenario.id,
                result.first_failure()
            );
        }
    }

    #[test]
    fn regional_audit_hidden_contract_is_observable_in_assigned_evidence() {
        let scenario = scenarios()
            .into_iter()
            .find(|scenario| scenario.id == "regional-audit-export")
            .unwrap();
        let corpus = scenario
            .evidence
            .iter()
            .filter(|evidence| evidence.role == EvidenceRole::Required)
            .fold(
                scenario.task.to_ascii_lowercase(),
                |mut corpus, evidence| {
                    corpus.push('\n');
                    corpus.push_str(&evidence.text.to_ascii_lowercase());
                    corpus
                },
            );
        for required in [
            "polluntilready(id, fetchstatus)",
            "status ready is the successful terminal state",
            "exact object name audit.ndjson",
            "audit-eu-west-1",
            "staging-eu coverage is unreachable",
            "storage -> worker -> api -> web",
            "api-v2-route first",
            "audit_export_residency_violation_total",
        ] {
            assert!(
                corpus.contains(required),
                "regional audit hidden contract {required:?} was not observable"
            );
        }
    }

    #[test]
    fn structurally_different_facade_reference_implementations_pass() {
        for scenario in scenarios() {
            let temp = tempfile::tempdir().unwrap();
            (scenario.seed)(temp.path()).unwrap();
            let facades =
                apply_alternate_module_layout(temp.path(), scenario.reference_apply).unwrap();
            assert!(
                facades.len() >= 3,
                "{} created only {} alternate facades",
                scenario.id,
                facades.len()
            );
            for relative in &facades {
                let facade = std::fs::read_to_string(temp.path().join(relative)).unwrap();
                assert!(facade.starts_with("export * from './"));
                let stem = Path::new(relative)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap();
                assert!(temp
                    .path()
                    .join(relative)
                    .with_file_name(format!("{stem}.alternate.mjs"))
                    .is_file());
            }
            let result = (scenario.verify)(temp.path());
            assert_eq!(
                result.score(),
                1.0,
                "{} alternate layout: {:?}",
                scenario.id,
                result.first_failure()
            );
        }
    }

    #[test]
    fn small_component_mutations_are_detected_by_each_hidden_assertion() {
        let mutations: &[(&str, &str, &str, &str)] = &[
            (
                "regional-audit-export",
                "repos/api/src/auditExports.mjs",
                "/v2/audit-exports",
                "/v1/audit-export",
            ),
            (
                "regional-audit-export",
                "repos/worker/src/exportJob.mjs",
                "job.region==='eu'",
                "false",
            ),
            (
                "regional-audit-export",
                "repos/web/src/auditDownload.mjs",
                "export async function pollUntilReady",
                "export async function disabled",
            ),
            (
                "regional-audit-export",
                "repos/infra/residency.json",
                "audit-eu-west-1",
                "audit-us",
            ),
            (
                "regional-audit-export",
                "repos/worker/src/metrics.mjs",
                "audit_export_residency_violation_total",
                "audit_export_completed_total",
            ),
            (
                "rolling-event-v2",
                "repos/events/src/envelope.mjs",
                "schemaVersion:2",
                "schemaVersion:1",
            ),
            (
                "rolling-event-v2",
                "repos/billing/src/consumer.mjs",
                "event.schemaVersion??1",
                "1",
            ),
            (
                "rolling-event-v2",
                "repos/analytics/src/consumer.py",
                "event.get('schemaVersion', 1)",
                "1",
            ),
            (
                "rolling-event-v2",
                "repos/notifications/src/consumer.mjs",
                "event.schemaVersion??1",
                "1",
            ),
            (
                "rolling-event-v2",
                "config/event-rollout.json",
                "\"producer\"]",
                "\"producer\",\"billing-consumer\"]",
            ),
            (
                "permission-collaboration-split",
                "repos/core/src/preferences.mjs",
                "['ask','auto','full']",
                "['ask','auto','full','plan']",
            ),
            (
                "permission-collaboration-split",
                "repos/desktop/src/migrate.mjs",
                "permissionMode:'ask'",
                "permissionMode:'full'",
            ),
            (
                "permission-collaboration-split",
                "repos/cloud/src/preferences.mjs",
                "store.set('collaboration-mode',value.collaborationMode);",
                "",
            ),
            (
                "permission-collaboration-split",
                "repos/mobile/src/settings.mjs",
                "['ask','auto','full']",
                "['ask','auto','full','plan']",
            ),
            (
                "permission-collaboration-split",
                "repos/sync/src/merge.mjs",
                "remote.collaborationMode??local.collaborationMode",
                "remote.collaborationMode",
            ),
        ];
        for (scenario_id, path, needle, replacement) in mutations {
            let scenario = scenarios()
                .into_iter()
                .find(|candidate| candidate.id == *scenario_id)
                .unwrap();
            let temp = tempfile::tempdir().unwrap();
            (scenario.seed)(temp.path()).unwrap();
            (scenario.reference_apply)(temp.path()).unwrap();
            let original = std::fs::read_to_string(temp.path().join(path)).unwrap();
            assert!(original.contains(needle), "{scenario_id}:{path}");
            std::fs::write(
                temp.path().join(path),
                original.replacen(needle, replacement, 1),
            )
            .unwrap();
            assert!(
                (scenario.verify)(temp.path()).score() < 1.0,
                "mutation survived: {scenario_id}:{path}"
            );
        }
    }

    #[test]
    fn every_oracle_changed_file_is_behaviorally_required() {
        for scenario in scenarios() {
            let inventory = tempfile::tempdir().unwrap();
            (scenario.seed)(inventory.path()).unwrap();
            (scenario.reference_apply)(inventory.path()).unwrap();
            let changed = Command::new("git")
                .arg("-C")
                .arg(inventory.path())
                .args(["diff", "--name-only", "HEAD"])
                .output()
                .unwrap();
            let paths = String::from_utf8(changed.stdout).unwrap();
            for path in paths.lines() {
                let temp = tempfile::tempdir().unwrap();
                (scenario.seed)(temp.path()).unwrap();
                (scenario.reference_apply)(temp.path()).unwrap();
                let baseline = Command::new("git")
                    .arg("-C")
                    .arg(temp.path())
                    .args(["show", &format!("HEAD:{path}")])
                    .output()
                    .unwrap();
                assert!(baseline.status.success(), "{}:{path}", scenario.id);
                std::fs::write(temp.path().join(path), baseline.stdout).unwrap();
                assert!(
                    (scenario.verify)(temp.path()).score() < 1.0,
                    "oracle change is not behaviorally required: {}:{path}",
                    scenario.id
                );
            }
        }
    }
}
