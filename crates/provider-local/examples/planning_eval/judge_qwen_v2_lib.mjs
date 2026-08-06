import crypto from 'node:crypto';
import fs from 'node:fs';

export const MODEL = 'qwen/qwen3.7-flash';
export const PLAN_QUALITIES = ['poor', 'mixed', 'good', 'excellent'];
export const CONFIDENCES = ['low', 'medium', 'high'];
export const EFFECTS = ['delivered_better', 'equivalent', 'delivered_worse', 'mixed', 'unresolved'];
export const CONTEXT_EFFECTS = ['beneficial', 'neutral', 'harmful', 'mixed', 'unresolved'];
export const TREATMENTS = ['none', 'deferred_retrieval', 'source_preactivation', 'prefetched_capsule'];

export function sha256(value) {
  return crypto.createHash('sha256').update(value).digest('hex');
}

export function readJsonl(file) {
  if (!fs.existsSync(file)) return [];
  return fs.readFileSync(file, 'utf8').split('\n').filter(Boolean).map(JSON.parse);
}

export function writeJsonlAtomic(file, values) {
  const temporary = `${file}.tmp`;
  fs.writeFileSync(temporary, values.length ? `${values.map((value) => JSON.stringify(value)).join('\n')}\n` : '');
  fs.renameSync(temporary, file);
}

export function writeJsonAtomic(file, value) {
  const temporary = `${file}.tmp`;
  fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`);
  fs.renameSync(temporary, file);
}

function fail(message) {
  throw new Error(message);
}

export function exactKeys(value, expected, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) fail(`${label} must be an object`);
  const actual = Object.keys(value);
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    fail(`${label} keys/order ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
  }
}

function oneOf(value, allowed, label) {
  if (!allowed.includes(value)) fail(`${label}=${JSON.stringify(value)}`);
}

function citationArray(value, label) {
  if (!Array.isArray(value) || value.length === 0) fail(`${label} requires citations`);
  for (const [index, citation] of value.entries()) {
    exactKeys(citation, ['locator', 'claim'], `${label}.citations[${index}]`);
    if (typeof citation.locator !== 'string' || !citation.locator.trim()) fail(`${label} citation locator is empty`);
    if (typeof citation.claim !== 'string' || !citation.claim.trim()) fail(`${label} citation claim is empty`);
  }
}

function reasoned(value, label) {
  if (typeof value.rationale !== 'string' || value.rationale.trim().length < 20) fail(`${label} rationale is too short`);
  citationArray(value.citations, label);
}

function judgeIdentity(value, label, runLabel) {
  exactKeys(value, ['model', 'run_label'], label);
  if (value.model !== MODEL || value.run_label !== runLabel) fail(`${label} identity mismatch`);
}

function same(left, right, label) {
  if (JSON.stringify(left) !== JSON.stringify(right)) fail(`${label} differs inside a frozen pair`);
}

function treatmentFor(lane) {
  const prefix = lane.replace(/_(discarded|typed_replay)$/, '');
  const treatments = new Map([
    ['bank_none', 'none'],
    ['bank_all', 'deferred_retrieval'],
    ['bank_all_preactivated', 'source_preactivation'],
    ['bank_all_prefetched', 'prefetched_capsule'],
  ]);
  const treatment = treatments.get(prefix);
  if (!treatment) fail(`unsupported paired lane ${lane}`);
  return treatment;
}

function compactArm(packet) {
  return {
    handoff: packet.handoff,
    executor_context: packet.executor_context,
    executor_trajectory: packet.executor_trajectory,
    baseline_tree_sha256: packet.baseline_tree_sha256,
    baseline_files: packet.baseline_files,
    final_tree_sha256: packet.final_tree_sha256,
    final_files: packet.final_files,
    hidden_verification: packet.hidden_verification,
    retries: packet.retries,
    provider_error: packet.provider_error,
  };
}

export function buildPackets(sourcePackets, index, contractSha256) {
  const entries = new Map(index.entries.map((entry) => [entry.packet_id, entry]));
  const joined = sourcePackets.map((packet) => {
    const identity = entries.get(packet.packet_id);
    if (!identity) fail(`packet ${packet.packet_id} missing from index`);
    return { packet, identity };
  }).filter(({ identity }) => /_(discarded|typed_replay)$/.test(identity.lane));
  const groups = new Map();
  for (const item of joined) {
    const bankId = item.packet.handoff.plan_bank_id;
    if (!bankId) fail(`${item.identity.lane} omitted plan_bank_id`);
    const values = groups.get(bankId) ?? [];
    values.push(item);
    groups.set(bankId, values);
  }

  const plans = [];
  const pairs = [];
  const treatmentIndex = [];
  for (const [bankId, items] of [...groups.entries()].sort(([left], [right]) => left.localeCompare(right))) {
    if (items.length !== 2) fail(`plan bank ${bankId} has ${items.length} paired arms`);
    const discarded = items.find(({ identity }) => identity.lane.endsWith('_discarded'));
    const delivered = items.find(({ identity }) => identity.lane.endsWith('_typed_replay'));
    if (!discarded || !delivered) fail(`plan bank ${bankId} lacks discarded or typed arm`);
    same(discarded.packet.plan, delivered.packet.plan, `${bankId} plan`);
    same(discarded.packet.planner_context, delivered.packet.planner_context, `${bankId} planner_context`);
    same(discarded.packet.planner_trajectory, delivered.packet.planner_trajectory, `${bankId} planner_trajectory`);
    if (discarded.packet.handoff.plan_sha256 !== delivered.packet.handoff.plan_sha256) fail(`${bankId} source plan hash differs`);
    if (discarded.packet.handoff.delivered_plan_sha256 !== null) fail(`${bankId} discarded arm received plan bytes`);
    const handoff = delivered.packet.handoff;
    if (handoff.delivery_truncated || handoff.plan_sha256 !== handoff.delivered_plan_sha256 || handoff.source_plan_chars !== handoff.delivered_plan_chars) {
      fail(`${bankId} delivered arm did not receive the complete frozen plan`);
    }
    const treatment = treatmentFor(delivered.identity.lane);
    if (treatment !== treatmentFor(discarded.identity.lane)) fail(`${bankId} treatment mismatch`);
    const planPacketId = sha256(`plan-judge-v2:${contractSha256}:${bankId}:${handoff.plan_sha256}`);
    const planPacket = {
      schema_version: 2,
      packet_id: planPacketId,
      judge_contract_sha256: contractSha256,
      source: { plan_bank_id: bankId, plan_sha256: handoff.plan_sha256, scenario: delivered.identity.scenario, repetition: delivered.identity.repetition },
      task: delivered.packet.task,
      private_reference_plan: delivered.packet.private_reference_plan,
      behavior_contracts: delivered.packet.behavior_contracts,
      evidence_catalog: delivered.packet.evidence_catalog,
      plan: delivered.packet.plan,
      planner_context: delivered.packet.planner_context,
      planner_trajectory: delivered.packet.planner_trajectory,
    };
    plans.push(planPacket);

    const pairId = sha256(`pair-judge-v2:${contractSha256}:${bankId}:${discarded.packet.packet_id}:${delivered.packet.packet_id}`);
    const deliveredIsA = Number.parseInt(sha256(`${pairId}:arm-order`).slice(0, 2), 16) % 2 === 0;
    const armA = deliveredIsA ? delivered.packet : discarded.packet;
    const armB = deliveredIsA ? discarded.packet : delivered.packet;
    pairs.push({
      schema_version: 2,
      packet_id: pairId,
      judge_contract_sha256: contractSha256,
      source: { plan_packet_id: planPacketId, plan_bank_id: bankId, scenario: delivered.identity.scenario, repetition: delivered.identity.repetition },
      task: delivered.packet.task,
      behavior_contracts: delivered.packet.behavior_contracts,
      shared_plan: delivered.packet.plan,
      arm_a: compactArm(armA),
      arm_b: compactArm(armB),
    });
    treatmentIndex.push({ treatment, repetition: delivered.identity.repetition, plan_packet_id: planPacketId, pair_packet_id: pairId, delivered_arm: deliveredIsA ? 'a' : 'b' });
  }
  treatmentIndex.sort((left, right) => left.repetition - right.repetition || TREATMENTS.indexOf(left.treatment) - TREATMENTS.indexOf(right.treatment));
  return { plans, pairs, treatmentIndex };
}

export function validatePlanVerdict(value, packet, runLabel) {
  exactKeys(value, ['schema_version', 'packet_id', 'judge_contract_sha256', 'judge', 'behaviors', 'knowledge', 'overall', 'limitations'], 'plan_verdict');
  if (value.schema_version !== 2 || value.packet_id !== packet.packet_id || value.judge_contract_sha256 !== packet.judge_contract_sha256) fail('plan verdict identity mismatch');
  judgeIdentity(value.judge, 'plan_verdict.judge', runLabel);
  const behaviorIds = packet.behavior_contracts.map(({ id }) => id);
  if (JSON.stringify(value.behaviors?.map(({ behavior_id }) => behavior_id)) !== JSON.stringify(behaviorIds)) fail('plan behavior IDs/order mismatch');
  for (const item of value.behaviors) {
    exactKeys(item, ['behavior_id', 'coverage', 'confidence', 'rationale', 'citations'], `plan.behavior.${item.behavior_id}`);
    oneOf(item.coverage, ['omitted', 'incorrect', 'partial', 'correct'], `${item.behavior_id}.coverage`);
    oneOf(item.confidence, CONFIDENCES, `${item.behavior_id}.confidence`);
    reasoned(item, `plan.behavior.${item.behavior_id}`);
  }
  const evidenceIds = packet.planner_context.assigned_evidence_ids;
  if (JSON.stringify(value.knowledge?.map(({ evidence_id }) => evidence_id)) !== JSON.stringify(evidenceIds)) fail('plan knowledge IDs/order mismatch');
  for (const item of value.knowledge) {
    exactKeys(item, ['evidence_id', 'influence', 'confidence', 'rationale', 'citations'], `plan.knowledge.${item.evidence_id}`);
    oneOf(item.influence, ['not_used', 'cited_only', 'used_correctly', 'used_incorrectly', 'unverifiable'], `${item.evidence_id}.influence`);
    oneOf(item.confidence, CONFIDENCES, `${item.evidence_id}.confidence`);
    reasoned(item, `plan.knowledge.${item.evidence_id}`);
  }
  exactKeys(value.overall, ['plan_quality', 'implementation_readiness', 'confidence', 'rationale', 'citations'], 'plan.overall');
  oneOf(value.overall.plan_quality, PLAN_QUALITIES, 'plan.overall.plan_quality');
  oneOf(value.overall.implementation_readiness, ['not_ready', 'partially_ready', 'ready', 'exceptionally_ready'], 'plan.overall.implementation_readiness');
  oneOf(value.overall.confidence, CONFIDENCES, 'plan.overall.confidence');
  reasoned(value.overall, 'plan.overall');
  if (!Array.isArray(value.limitations)) fail('plan limitations must be an array');
  return value;
}

export function validatePairVerdict(value, packet, runLabel, deliveredArm) {
  exactKeys(value, ['schema_version', 'packet_id', 'judge_contract_sha256', 'judge', 'behaviors', 'comparison', 'limitations'], 'pair_verdict');
  if (value.schema_version !== 2 || value.packet_id !== packet.packet_id || value.judge_contract_sha256 !== packet.judge_contract_sha256) fail('pair verdict identity mismatch');
  judgeIdentity(value.judge, 'pair_verdict.judge', runLabel);
  const behaviorIds = packet.behavior_contracts.map(({ id }) => id);
  if (JSON.stringify(value.behaviors?.map(({ behavior_id }) => behavior_id)) !== JSON.stringify(behaviorIds)) fail('pair behavior IDs/order mismatch');
  for (const item of value.behaviors) {
    exactKeys(item, ['behavior_id', 'execution_effect', 'causal_boundary', 'confidence', 'rationale', 'citations'], `pair.behavior.${item.behavior_id}`);
    oneOf(item.execution_effect, EFFECTS, `${item.behavior_id}.execution_effect`);
    oneOf(item.causal_boundary, ['plan', 'plan_delivery', 'executor', 'verification', 'capacity', 'fixture', 'mixed', 'unresolved'], `${item.behavior_id}.causal_boundary`);
    oneOf(item.confidence, CONFIDENCES, `${item.behavior_id}.confidence`);
    reasoned(item, `pair.behavior.${item.behavior_id}`);
  }
  exactKeys(value.comparison, ['delivered_arm', 'execution_effect', 'planner_respected', 'completion_honesty_effect', 'confidence', 'rationale', 'citations'], 'pair.comparison');
  if (value.comparison.delivered_arm !== deliveredArm) fail('pair verdict identified the wrong delivered arm');
  oneOf(value.comparison.execution_effect, EFFECTS, 'pair.comparison.execution_effect');
  oneOf(value.comparison.planner_respected, ['no', 'partially', 'mostly', 'fully', 'unresolved'], 'pair.comparison.planner_respected');
  oneOf(value.comparison.completion_honesty_effect, EFFECTS, 'pair.comparison.completion_honesty_effect');
  oneOf(value.comparison.confidence, CONFIDENCES, 'pair.comparison.confidence');
  reasoned(value.comparison, 'pair.comparison');
  if (!Array.isArray(value.limitations)) fail('pair limitations must be an array');
  return value;
}

export function validateAudit(value, candidateId, runLabel) {
  exactKeys(value, ['schema_version', 'candidate_packet_id', 'judge', 'contradictions', 'decision', 'rationale', 'citations'], 'audit');
  if (value.schema_version !== 2 || value.candidate_packet_id !== candidateId) fail('audit identity mismatch');
  judgeIdentity(value.judge, 'audit.judge', runLabel);
  if (!Array.isArray(value.contradictions) || value.contradictions.some((item) => typeof item !== 'string' || !item.trim())) fail('audit contradictions invalid');
  oneOf(value.decision, ['accept', 'reject'], 'audit.decision');
  if (value.decision === 'accept' && value.contradictions.length !== 0) fail('accepting audit retained contradictions');
  if (value.decision === 'reject' && value.contradictions.length === 0) fail('rejecting audit omitted contradictions');
  reasoned(value, 'audit');
  return value;
}

export function validateFinalVerdict(value, packet, runLabel) {
  exactKeys(value, ['schema_version', 'evaluation_id', 'judge_contract_sha256', 'judge', 'mechanisms', 'plan_delivery_effect', 'context_effect', 'overall', 'limitations'], 'final_verdict');
  if (value.schema_version !== 2 || value.evaluation_id !== packet.evaluation_id || value.judge_contract_sha256 !== packet.judge_contract_sha256) fail('final verdict identity mismatch');
  judgeIdentity(value.judge, 'final_verdict.judge', runLabel);
  if (JSON.stringify(value.mechanisms?.map(({ treatment }) => treatment)) !== JSON.stringify(TREATMENTS)) fail('final mechanism IDs/order mismatch');
  for (const item of value.mechanisms) {
    exactKeys(item, ['treatment', 'plan_effect', 'execution_effect', 'evidence_use', 'confidence', 'rationale', 'citations'], `final.mechanism.${item.treatment}`);
    oneOf(item.plan_effect, CONTEXT_EFFECTS, `${item.treatment}.plan_effect`);
    oneOf(item.execution_effect, CONTEXT_EFFECTS, `${item.treatment}.execution_effect`);
    oneOf(item.evidence_use, ['absent', 'selective', 'effective', 'misused', 'mixed', 'unresolved'], `${item.treatment}.evidence_use`);
    oneOf(item.confidence, CONFIDENCES, `${item.treatment}.confidence`);
    reasoned(item, `final.mechanism.${item.treatment}`);
  }
  for (const [label, item] of [['plan_delivery_effect', value.plan_delivery_effect], ['context_effect', value.context_effect]]) {
    exactKeys(item, ['verdict', 'confidence', 'rationale', 'citations'], `final.${label}`);
    oneOf(item.verdict, CONTEXT_EFFECTS, `final.${label}.verdict`);
    oneOf(item.confidence, CONFIDENCES, `final.${label}.confidence`);
    reasoned(item, `final.${label}`);
  }
  exactKeys(value.overall, ['conclusion', 'confidence', 'rationale', 'citations'], 'final.overall');
  oneOf(value.overall.conclusion, ['supported', 'promising_not_proven', 'no_benefit', 'harmful', 'unresolved'], 'final.overall.conclusion');
  oneOf(value.overall.confidence, CONFIDENCES, 'final.overall.confidence');
  reasoned(value.overall, 'final.overall');
  if (!Array.isArray(value.limitations)) fail('final limitations must be an array');
  return value;
}
