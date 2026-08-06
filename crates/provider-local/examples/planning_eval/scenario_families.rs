use crate::fixture_support::{push_node, seed_repository_history, seed_scaffold, write};
use crate::model::{
    Evidence, EvidenceRole, EvidenceSource, Scenario, SemanticPlanCheck, Verification,
};
use std::path::Path;

pub(super) struct FamilySpec {
    pub(super) name: &'static str,
    pub(super) files: &'static [(&'static str, &'static str, &'static str)],
    pub(super) checks: &'static [(&'static str, &'static str)],
}

pub(super) fn seed_family(root: &Path, spec: &FamilySpec) -> Result<(), String> {
    seed_scaffold(root, spec.name)?;
    for (path, baseline, _) in spec.files {
        write(root, path, baseline)?;
    }
    write(
        root,
        "tests/family-visible.test.mjs",
        "import test from 'node:test'; import assert from 'node:assert/strict'; test('baseline workspace loads',()=>assert.equal(typeof process.version,'string'));\n",
    )?;
    seed_repository_history(
        root,
        &format!("Establish {} compatibility baseline", spec.name),
    )
}

pub(super) fn apply_family(root: &Path, spec: &FamilySpec) -> Result<(), String> {
    for (path, _, reference) in spec.files {
        write(root, path, reference)?;
    }
    Ok(())
}

pub(super) fn verify_family(root: &Path, spec: &FamilySpec) -> Verification {
    let mut result = Verification::default();
    for (id, script) in spec.checks {
        push_node(&mut result, root, id, script);
    }
    result
}

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

const FAMILY_DISTRACTORS: &[Evidence] = &[
    Evidence {
        id: "PROJECT-GENERIC-01",
        source: EvidenceSource::Project,
        role: EvidenceRole::Distractor,
        text: "The repository uses Node built-in tests for deterministic contract checks.",
    },
    Evidence {
        id: "PROJECT-GENERIC-02",
        source: EvidenceSource::Project,
        role: EvidenceRole::Distractor,
        text: "An archived UI migration is outside the current service boundary.",
    },
    Evidence {
        id: "PROJECT-GENERIC-03",
        source: EvidenceSource::Project,
        role: EvidenceRole::Distractor,
        text: "Operational configuration is versioned with application changes.",
    },
    Evidence {
        id: "PROJECT-GENERIC-04",
        source: EvidenceSource::Project,
        role: EvidenceRole::Distractor,
        text: "A prior cache experiment never reached production.",
    },
    Evidence {
        id: "PROJECT-GENERIC-05",
        source: EvidenceSource::Project,
        role: EvidenceRole::Distractor,
        text: "Shared logger helpers must not receive credentials.",
    },
    Evidence {
        id: "ORG-GENERIC-01",
        source: EvidenceSource::Org,
        role: EvidenceRole::Distractor,
        text: "Release engineering owns production credentials.",
    },
    Evidence {
        id: "ORG-GENERIC-02",
        source: EvidenceSource::Org,
        role: EvidenceRole::Distractor,
        text: "Marketing services deploy on an independent schedule.",
    },
    Evidence {
        id: "ORG-GENERIC-03",
        source: EvidenceSource::Org,
        role: EvidenceRole::Distractor,
        text: "Cost labels are required for cloud resources.",
    },
    Evidence {
        id: "ORG-GENERIC-04",
        source: EvidenceSource::Org,
        role: EvidenceRole::Distractor,
        text: "Rollback ownership must be named for migrations.",
    },
    Evidence {
        id: "ORG-GENERIC-05",
        source: EvidenceSource::Org,
        role: EvidenceRole::Distractor,
        text: "General application logs retain metadata for thirty days.",
    },
    Evidence {
        id: "SCOUT-GENERIC-01",
        source: EvidenceSource::Scout,
        role: EvidenceRole::Distractor,
        text: "A development-only worker has no production traffic.",
    },
    Evidence {
        id: "SCOUT-GENERIC-02",
        source: EvidenceSource::Scout,
        role: EvidenceRole::Distractor,
        text: "The support dashboard reads metrics but is not on the write path.",
    },
    Evidence {
        id: "SCOUT-GENERIC-03",
        source: EvidenceSource::Scout,
        role: EvidenceRole::Distractor,
        text: "An archived repository still imports the old package name.",
    },
    Evidence {
        id: "SCOUT-GENERIC-04",
        source: EvidenceSource::Scout,
        role: EvidenceRole::Distractor,
        text: "One unrelated sandbox account is unreachable.",
    },
    Evidence {
        id: "SCOUT-GENERIC-05",
        source: EvidenceSource::Scout,
        role: EvidenceRole::Distractor,
        text: "Central observability is reachable by every production component.",
    },
    Evidence {
        id: "NOISE-GENERIC-01",
        source: EvidenceSource::Noise,
        role: EvidenceRole::Distractor,
        text: "A future design exercise proposes a different message broker.",
    },
    Evidence {
        id: "NOISE-GENERIC-02",
        source: EvidenceSource::Noise,
        role: EvidenceRole::Distractor,
        text: "A draft suggests renaming all repositories next year.",
    },
    Evidence {
        id: "NOISE-GENERIC-03",
        source: EvidenceSource::Noise,
        role: EvidenceRole::Distractor,
        text: "A sandbox prototype used unrelated identifiers.",
    },
    Evidence {
        id: "NOISE-GENERIC-04",
        source: EvidenceSource::Noise,
        role: EvidenceRole::Distractor,
        text: "An unapproved dashboard redesign is not an implementation requirement.",
    },
];

pub(super) fn evidence(base: &[Evidence]) -> Vec<Evidence> {
    let mut result = base.to_vec();
    result.extend_from_slice(FAMILY_DISTRACTORS);
    result
}

const OAUTH_FILES: &[(&str, &str, &str)] = &[
    ("repos/auth/src/jwks.mjs", "export const verifyKeys=['key-old']; export const signingKey='key-old';\n", "export const verifyKeys=['key-next','key-old']; export const signingKey='key-next';\n"),
    ("repos/gateway/src/verify.mjs", "export const verify=(kid)=>kid==='key-old';\n", "import {verifyKeys} from '../../auth/src/jwks.mjs'; export const verify=(kid)=>verifyKeys.includes(kid);\n"),
    ("repos/worker/src/tokenVerify.mjs", "export const verify=(kid)=>kid==='key-old';\n", "import {verifyKeys} from '../../auth/src/jwks.mjs'; export const verify=(kid)=>verifyKeys.includes(kid);\n"),
    ("repos/admin/src/rotation.mjs", "export const rotationPlan=()=>['replace-key'];\n", "export const rotationPlan=()=>['publish-next','dual-verify','sign-next','retire-previous'];\n"),
    ("config/key-rotation.json", "{\"graceHours\":0,\"stages\":[\"replace\"]}\n", "{\"graceHours\":72,\"stages\":[\"publish-next\",\"dual-verify\",\"sign-next\",\"retire-previous\"]}\n"),
    ("deploy/production/auth-keys.json", "{\"order\":[\"signer\",\"verifiers\"]}\n", "{\"order\":[\"publish-jwks\",\"gateway\",\"worker\",\"signer\"],\"rollback\":[\"sign-old\",\"retain-both-verifiers\"]}\n"),
    ("repos/observability/src/authMetrics.mjs", "export const metrics=['auth_verify_total'];\n", "export const metrics=['auth_verify_total','auth_unknown_kid_total','auth_old_key_verify_total'];\n"),
    ("docs/key-rotation.md", "Replace the active key during maintenance.\n", "Publish key-next, dual-verify for 72 hours, sign with key-next, then retire key-old. Roll back by signing with key-old while retaining both verification keys.\n"),
];
const OAUTH_CHECKS: &[(&str, &str)] = &[
    ("overlap_verification", "const k=await load('repos/auth/src/jwks.mjs'); assert.deepEqual(k.verifyKeys,['key-next','key-old']);"),
    ("all_consumers_verify", "const g=await load('repos/gateway/src/verify.mjs'); const w=await load('repos/worker/src/tokenVerify.mjs'); for(const kid of ['key-next','key-old']){assert.equal(g.verify(kid),true);assert.equal(w.verify(kid),true)} assert.equal(g.verify('unknown'),false);"),
    ("signing_cutover", "const k=await load('repos/auth/src/jwks.mjs'); const a=await load('repos/admin/src/rotation.mjs'); assert.equal(k.signingKey,'key-next'); assert.deepEqual(a.rotationPlan(),['publish-next','dual-verify','sign-next','retire-previous']);"),
    ("graceful_rollout", "const fs=await import('node:fs/promises'); const c=JSON.parse(await fs.readFile(join(root,'config/key-rotation.json'),'utf8')); const d=JSON.parse(await fs.readFile(join(root,'deploy/production/auth-keys.json'),'utf8')); assert.equal(c.graceHours,72); assert.deepEqual(d.order,['publish-jwks','gateway','worker','signer']); assert.equal(d.rollback[0],'sign-old');"),
    ("rotation_observability", "const m=await load('repos/observability/src/authMetrics.mjs'); const fs=await import('node:fs/promises'); const doc=await fs.readFile(join(root,'docs/key-rotation.md'),'utf8'); assert.ok(m.metrics.includes('auth_unknown_kid_total')); assert.match(doc,/retaining both verification keys/);"),
];
const OAUTH_SPEC: FamilySpec = FamilySpec {
    name: "oauth-key-rotation",
    files: OAUTH_FILES,
    checks: OAUTH_CHECKS,
};
family_functions!(seed_oauth, apply_oauth, verify_oauth, OAUTH_SPEC);
const OAUTH_EVIDENCE: &[Evidence] = &[
    Evidence { id:"PROJECT-OAUTH-OVERLAP", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"Key rotations preserve the previous verification key during a 72-hour overlap; signing moves only after all verifiers dual-read." },
    Evidence { id:"ORG-OAUTH-ORDER", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Effective 2026-05-01, publish JWKS then gateway and worker verifiers before signer cutover; rollback signs old while retaining both verifiers." },
    Evidence { id:"SCOUT-OAUTH-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Observed token graph includes auth signer, gateway verifier, and background-worker verifier; central auth metrics is reachable." },
    Evidence { id:"ORACLE-OAUTH", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Implement overlapping keys, both verifiers, staged admin/config/deploy flow, rollback, and unknown-kid metrics." },
    Evidence { id:"STALE-OAUTH-REPLACE", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Superseded runbook replaces the signing and verification key simultaneously." },
    Evidence { id:"CONFLICT-OAUTH-DROP", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Unapproved shortcut removes key-old immediately after publishing key-next." },
];
const OAUTH_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "overlap_verification",
        required_all: &["jwks.mjs", "key-next", "key-old", "dual-verify"],
        required_any: &[],
        expectation: "plan retains both verification keys during overlap",
    },
    SemanticPlanCheck {
        id: "all_consumers_verify",
        required_all: &["gateway", "worker", "verify"],
        required_any: &[],
        expectation: "plan updates every observed verifier",
    },
    SemanticPlanCheck {
        id: "signing_cutover",
        required_all: &["sign-next", "publish-next", "retire-previous"],
        required_any: &[],
        expectation: "plan separates publish verify sign and retirement stages",
    },
    SemanticPlanCheck {
        id: "graceful_rollout",
        required_all: &["72", "publish-jwks", "sign-old", "retain-both-verifiers"],
        required_any: &[],
        expectation: "plan encodes grace period order and rollback",
    },
    SemanticPlanCheck {
        id: "rotation_observability",
        required_all: &["auth_unknown_kid_total", "key-rotation.md"],
        required_any: &[],
        expectation: "plan includes discriminating metrics and runbook",
    },
];

const WEBHOOK_FILES: &[(&str, &str, &str)] = &[
    ("repos/payments/src/receiver.mjs", "export const receive=async(event,effects)=>effects.charge(event);\n", "export const receive=async(event,ledger,effects)=>{if(!ledger.claim(event.id))return 'duplicate'; await effects.bill(event); await effects.notify(event); return 'processed';};\n"),
    ("repos/ledger/src/idempotency.mjs", "export const createLedger=()=>({claim:()=>true});\n", "export const createLedger=()=>{const seen=new Set();return {claim:(id)=>{if(seen.has(id))return false;seen.add(id);return true;},seen}};\n"),
    ("repos/billing/src/webhook.mjs", "export const bill=(event)=>event.amount;\n", "export const bill=(event)=>({eventId:event.id,amount:event.amount,status:'recorded'});\n"),
    ("repos/notifications/src/webhook.mjs", "export const notify=()=>true;\n", "export const notify=(event)=>({dedupeKey:event.id,channel:'receipt'});\n"),
    ("config/webhook-retry.json", "{\"attempts\":1}\n", "{\"attempts\":5,\"backoffSeconds\":[1,5,30,120,300],\"deadLetter\":\"payments-webhook-dlq\"}\n"),
    ("deploy/production/payment-webhooks.json", "{\"order\":[\"receiver\",\"ledger\"]}\n", "{\"order\":[\"ledger\",\"billing\",\"notifications\",\"receiver\"],\"rollback\":[\"disable-ingress\",\"drain-inflight\",\"retain-ledger\"]}\n"),
    ("repos/observability/src/webhookMetrics.mjs", "export const metrics=['webhook_total'];\n", "export const metrics=['webhook_total','webhook_duplicate_total','webhook_dlq_total'];\n"),
    ("docs/webhook-recovery.md", "Retry failed webhook requests.\n", "Disable ingress, drain in-flight handlers, retain the idempotency ledger, and replay the DLQ by event ID.\n"),
];
const WEBHOOK_CHECKS: &[(&str, &str)] = &[
    ("atomic_idempotency", "const l=await load('repos/ledger/src/idempotency.mjs'); const ledger=l.createLedger(); assert.equal(ledger.claim('evt-1'),true); assert.equal(ledger.claim('evt-1'),false);"),
    ("single_side_effects", "const r=await load('repos/payments/src/receiver.mjs'); const l=await load('repos/ledger/src/idempotency.mjs'); const calls=[]; const effects={bill:async e=>calls.push(['bill',e.id]),notify:async e=>calls.push(['notify',e.id])}; assert.equal(await r.receive({id:'evt-1'},l.createLedger(),effects),'processed'); const ledger=l.createLedger(); await r.receive({id:'evt-2'},ledger,effects); assert.equal(await r.receive({id:'evt-2'},ledger,effects),'duplicate'); assert.deepEqual(calls.slice(-2),[['bill','evt-2'],['notify','evt-2']]);"),
    ("consumer_dedupe_shapes", "const b=await load('repos/billing/src/webhook.mjs'); const n=await load('repos/notifications/src/webhook.mjs'); assert.deepEqual(b.bill({id:'e',amount:7}),{eventId:'e',amount:7,status:'recorded'}); assert.equal(n.notify({id:'e'}).dedupeKey,'e');"),
    ("bounded_retry_dlq", "const fs=await import('node:fs/promises'); const c=JSON.parse(await fs.readFile(join(root,'config/webhook-retry.json'),'utf8')); assert.equal(c.attempts,5); assert.equal(c.backoffSeconds.at(-1),300); assert.equal(c.deadLetter,'payments-webhook-dlq');"),
    ("safe_rollout_recovery", "const fs=await import('node:fs/promises'); const d=JSON.parse(await fs.readFile(join(root,'deploy/production/payment-webhooks.json'),'utf8')); const doc=await fs.readFile(join(root,'docs/webhook-recovery.md'),'utf8'); const m=await load('repos/observability/src/webhookMetrics.mjs'); assert.equal(d.order[0],'ledger'); assert.deepEqual(d.rollback,['disable-ingress','drain-inflight','retain-ledger']); assert.match(doc,/replay the DLQ by event ID/); assert.ok(m.metrics.includes('webhook_duplicate_total'));"),
];
const WEBHOOK_SPEC: FamilySpec = FamilySpec {
    name: "payment-webhook-idempotency",
    files: WEBHOOK_FILES,
    checks: WEBHOOK_CHECKS,
};
family_functions!(seed_webhook, apply_webhook, verify_webhook, WEBHOOK_SPEC);
const WEBHOOK_EVIDENCE: &[Evidence] = &[
    Evidence { id:"PROJECT-WEBHOOK-ID", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"Provider event ID is the durable idempotency key; the ledger claim precedes billing and receipt side effects." },
    Evidence { id:"ORG-WEBHOOK-RETRY", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Webhook retries stop after five attempts with a 300-second maximum backoff, then enter payments-webhook-dlq." },
    Evidence { id:"SCOUT-WEBHOOK-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Production graph is receiver -> ledger -> billing and notifications; recovery disables ingress, drains in-flight work, and retains ledger state." },
    Evidence { id:"ORACLE-WEBHOOK", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Implement atomic event-ID claims, deduped consumers, bounded retry/DLQ, ledger-first rollout, recovery, and duplicate metrics." },
    Evidence { id:"STALE-WEBHOOK-MEMORY", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Old prototype deduplicated webhook IDs in process memory." },
    Evidence { id:"CONFLICT-WEBHOOK-RETRY", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Unapproved note retries forever and clears the ledger during rollback." },
];
const WEBHOOK_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "atomic_idempotency",
        required_all: &["idempotency.mjs", "event id", "claim"],
        required_any: &[],
        expectation: "plan claims the durable provider event ID atomically",
    },
    SemanticPlanCheck {
        id: "single_side_effects",
        required_all: &["receiver.mjs", "billing", "notifications", "duplicate"],
        required_any: &[],
        expectation: "plan prevents repeated side effects",
    },
    SemanticPlanCheck {
        id: "consumer_dedupe_shapes",
        required_all: &["eventid", "dedupekey"],
        required_any: &[],
        expectation: "plan gives consumers stable dedupe result shapes",
    },
    SemanticPlanCheck {
        id: "bounded_retry_dlq",
        required_all: &["five", "300", "payments-webhook-dlq"],
        required_any: &[],
        expectation: "plan specifies bounded retry and DLQ",
    },
    SemanticPlanCheck {
        id: "safe_rollout_recovery",
        required_all: &[
            "ledger-first",
            "disable ingress",
            "drain",
            "retain",
            "webhook_duplicate_total",
        ],
        required_any: &[],
        expectation: "plan encodes safe rollout rollback and observability",
    },
];

const SEARCH_FILES: &[(&str, &str, &str)] = &[
    ("repos/search/src/schema.mjs", "export const versions=[1]; export const fields={1:['title']};\n", "export const versions=[1,2]; export const fields={1:['title'],2:['title','normalizedTitle','tenantId']};\n"),
    ("repos/search/src/writer.mjs", "export const targets=()=>['products-v1'];\n", "export const targets=(dualWrite)=>dualWrite?['products-v1','products-v2']:['products-v1'];\n"),
    ("repos/api/src/searchReader.mjs", "export const index='products-v1';\n", "export const indexFor=(aliases)=>aliases.products_read; export const fallback='products-v1';\n"),
    ("repos/search/src/backfill.mjs", "export const next=()=>0;\n", "export const next=(checkpoint,batch)=>({from:checkpoint,to:checkpoint+batch,checkpoint:checkpoint+batch});\n"),
    ("config/search-index.json", "{\"readAlias\":\"products-v1\",\"dualWrite\":false}\n", "{\"readAlias\":\"products-v2\",\"previousAlias\":\"products-v1\",\"dualWrite\":true,\"checkpointBatch\":500}\n"),
    ("deploy/production/search-index.json", "{\"order\":[\"writer\",\"index\"]}\n", "{\"order\":[\"create-v2\",\"enable-dual-write\",\"backfill\",\"verify-counts\",\"switch-alias\"],\"rollback\":[\"alias-v1\",\"keep-dual-write\"]}\n"),
    ("repos/observability/src/searchMetrics.mjs", "export const metrics=['search_total'];\n", "export const metrics=['search_total','search_dual_write_error_total','search_backfill_lag'];\n"),
    ("docs/search-migration.md", "Replace the product index.\n", "Create v2, dual-write, checkpoint the backfill, verify counts, then switch the alias. Roll back the alias to v1 while keeping dual-write enabled.\n"),
];
const SEARCH_CHECKS: &[(&str, &str)] = &[
    ("additive_index_schema", "const s=await load('repos/search/src/schema.mjs'); assert.deepEqual(s.versions,[1,2]); assert.deepEqual(s.fields[1],['title']); assert.ok(s.fields[2].includes('tenantId'));"),
    ("dual_write", "const w=await load('repos/search/src/writer.mjs'); assert.deepEqual(w.targets(true),['products-v1','products-v2']); assert.deepEqual(w.targets(false),['products-v1']);"),
    ("alias_read_fallback", "const r=await load('repos/api/src/searchReader.mjs'); assert.equal(r.indexFor({products_read:'products-v2'}),'products-v2'); assert.equal(r.fallback,'products-v1');"),
    ("resumable_backfill", "const b=await load('repos/search/src/backfill.mjs'); assert.deepEqual(b.next(500,500),{from:500,to:1000,checkpoint:1000}); const fs=await import('node:fs/promises'); const c=JSON.parse(await fs.readFile(join(root,'config/search-index.json'),'utf8')); assert.equal(c.checkpointBatch,500);"),
    ("alias_rollout_rollback", "const fs=await import('node:fs/promises'); const d=JSON.parse(await fs.readFile(join(root,'deploy/production/search-index.json'),'utf8')); const doc=await fs.readFile(join(root,'docs/search-migration.md'),'utf8'); const m=await load('repos/observability/src/searchMetrics.mjs'); assert.equal(d.order.at(-1),'switch-alias'); assert.deepEqual(d.rollback,['alias-v1','keep-dual-write']); assert.match(doc,/verify counts/); assert.ok(m.metrics.includes('search_backfill_lag'));"),
];
const SEARCH_SPEC: FamilySpec = FamilySpec {
    name: "search-index-zero-downtime",
    files: SEARCH_FILES,
    checks: SEARCH_CHECKS,
};
family_functions!(seed_search, apply_search, verify_search, SEARCH_SPEC);
const SEARCH_EVIDENCE: &[Evidence] = &[
    Evidence { id:"PROJECT-SEARCH-DUAL", source:EvidenceSource::Project, role:EvidenceRole::Required, text:"Index migrations dual-write v1 and v2, checkpoint backfills in batches of 500, and retain products-v1 as the read fallback." },
    Evidence { id:"ORG-SEARCH-ALIAS", source:EvidenceSource::Org, role:EvidenceRole::Required, text:"Switch the read alias only after document-count verification; rollback points the alias to v1 while dual-write stays enabled." },
    Evidence { id:"SCOUT-SEARCH-GRAPH", source:EvidenceSource::Scout, role:EvidenceRole::Required, text:"Observed graph includes product writer, search backfill, API alias reader, both production indexes, and centralized lag metrics." },
    Evidence { id:"ORACLE-SEARCH", source:EvidenceSource::Oracle, role:EvidenceRole::Required, text:"Implement additive v2 schema, dual writer, alias reader/fallback, checkpoint backfill, staged deploy/rollback, and lag/error metrics." },
    Evidence { id:"STALE-SEARCH-REPLACE", source:EvidenceSource::Stale, role:EvidenceRole::Stale, text:"Superseded plan deletes products-v1 before starting the backfill." },
    Evidence { id:"CONFLICT-SEARCH-NODUAL", source:EvidenceSource::Conflict, role:EvidenceRole::Conflict, text:"Unapproved shortcut writes only v2 during migration." },
];
const SEARCH_SEMANTICS: &[SemanticPlanCheck] = &[
    SemanticPlanCheck {
        id: "additive_index_schema",
        required_all: &["schema.mjs", "versions", "tenantid", "products-v1"],
        required_any: &[],
        expectation: "plan defines an additive v2 schema while retaining v1",
    },
    SemanticPlanCheck {
        id: "dual_write",
        required_all: &["writer.mjs", "products-v1", "products-v2", "dual-write"],
        required_any: &[],
        expectation: "plan preserves writes to both indexes",
    },
    SemanticPlanCheck {
        id: "alias_read_fallback",
        required_all: &[
            "searchreader.mjs",
            "products_read",
            "fallback",
            "products-v1",
        ],
        required_any: &[],
        expectation: "plan uses an alias with an explicit v1 fallback",
    },
    SemanticPlanCheck {
        id: "resumable_backfill",
        required_all: &["backfill.mjs", "checkpoint", "500"],
        required_any: &[],
        expectation: "plan makes backfill resumable and bounded",
    },
    SemanticPlanCheck {
        id: "alias_rollout_rollback",
        required_all: &[
            "verify counts",
            "switch",
            "alias-v1",
            "keep-dual-write",
            "search_backfill_lag",
        ],
        required_any: &[],
        expectation: "plan encodes verified alias cutover rollback and metrics",
    },
];

pub fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            id: "oauth-key-rotation",
            task: "Rotate OAuth signing keys without rejecting in-flight tokens. Preserve key-old verification during a 72-hour overlap, update every observed verifier before signing with key-next, encode staged rollout and rollback, add discriminating metrics, and run visible tests.",
            required_plan_terms: &["jwks.mjs","verify.mjs","tokenVerify.mjs","rotation.mjs","key-rotation.json","auth-keys.json","authMetrics.mjs","key-rotation.md"],
            semantic_plan_checks: OAUTH_SEMANTICS,
            required_evidence: &["PROJECT-OAUTH-OVERLAP","ORG-OAUTH-ORDER","SCOUT-OAUTH-GRAPH"],
            forbidden_evidence: &["STALE-OAUTH-REPLACE","CONFLICT-OAUTH-DROP"],
            oracle_plan: "Use PROJECT-OAUTH-OVERLAP, ORG-OAUTH-ORDER, and SCOUT-OAUTH-GRAPH. jwks.mjs keeps key-next and key-old for dual-verify while signingKey becomes key-next. Update gateway verify.mjs and worker tokenVerify.mjs to verify both keys. rotation.mjs stages publish-next, dual-verify, sign-next, retire-previous. key-rotation.json uses a 72 hour grace; auth-keys.json orders publish-jwks, gateway, worker, signer and rolls back with sign-old plus retain-both-verifiers. authMetrics.mjs adds auth_unknown_kid_total and key-rotation.md documents retirement and rollback.",
            evidence: evidence(OAUTH_EVIDENCE),
            seed: seed_oauth,
            verify: verify_oauth,
            reference_apply: apply_oauth,
        },
        Scenario {
            id: "payment-webhook-idempotency",
            task: "Make payment-webhook processing durably idempotent across receiver, ledger, billing, and notifications. Bound retries, route exhaustion to a DLQ, encode ledger-first rollout and safe recovery, add duplicate metrics, and run visible tests.",
            required_plan_terms: &["receiver.mjs","idempotency.mjs","billing","notifications","webhook-retry.json","payment-webhooks.json","webhookMetrics.mjs","webhook-recovery.md"],
            semantic_plan_checks: WEBHOOK_SEMANTICS,
            required_evidence: &["PROJECT-WEBHOOK-ID","ORG-WEBHOOK-RETRY","SCOUT-WEBHOOK-GRAPH"],
            forbidden_evidence: &["STALE-WEBHOOK-MEMORY","CONFLICT-WEBHOOK-RETRY"],
            oracle_plan: "Use PROJECT-WEBHOOK-ID, ORG-WEBHOOK-RETRY, and SCOUT-WEBHOOK-GRAPH. idempotency.mjs atomically claims the provider event ID before receiver.mjs invokes billing and notifications; duplicates return without side effects. Billing returns eventId and notifications returns dedupeKey. webhook-retry.json makes five attempts with maximum 300 second backoff then payments-webhook-dlq. payment-webhooks.json is ledger-first and rollback must disable ingress, drain in-flight work, and retain ledger state. webhookMetrics.mjs adds webhook_duplicate_total and webhook-recovery.md documents DLQ replay.",
            evidence: evidence(WEBHOOK_EVIDENCE),
            seed: seed_webhook,
            verify: verify_webhook,
            reference_apply: apply_webhook,
        },
        Scenario {
            id: "search-index-zero-downtime",
            task: "Migrate the product search index to an additive v2 schema without downtime. Dual-write both indexes, checkpoint backfill, switch the read alias only after verification, preserve v1 fallback and rollback, add lag/error metrics, and run visible tests.",
            required_plan_terms: &["schema.mjs","writer.mjs","searchReader.mjs","backfill.mjs","search-index.json","search-index.json","searchMetrics.mjs","search-migration.md"],
            semantic_plan_checks: SEARCH_SEMANTICS,
            required_evidence: &["PROJECT-SEARCH-DUAL","ORG-SEARCH-ALIAS","SCOUT-SEARCH-GRAPH"],
            forbidden_evidence: &["STALE-SEARCH-REPLACE","CONFLICT-SEARCH-NODUAL"],
            oracle_plan: "Use PROJECT-SEARCH-DUAL, ORG-SEARCH-ALIAS, and SCOUT-SEARCH-GRAPH. schema.mjs retains products-v1 fields, declares versions 1 and 2, and adds v2 normalizedTitle and tenantId. writer.mjs dual-writes products-v1 and products-v2. searchReader.mjs reads products_read with products-v1 fallback. backfill.mjs persists a checkpoint every 500 documents and config keeps checkpointBatch 500. Deploy create-v2, enable-dual-write, backfill, verify counts, then switch alias; rollback alias-v1 and keep-dual-write. searchMetrics.mjs adds search_backfill_lag and the search-migration.md runbook records the verified cutover.",
            evidence: evidence(SEARCH_EVIDENCE),
            seed: seed_search,
            verify: verify_search,
            reference_apply: apply_search,
        },
    ]
}
