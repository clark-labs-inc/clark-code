import assert from 'node:assert/strict';
import test from 'node:test';
import {
  MODEL,
  buildPackets,
  validateAudit,
  validatePairVerdict,
  validatePlanVerdict,
} from './judge_qwen_v2_lib.mjs';

const contractSha256 = 'c'.repeat(64);
const runLabel = 'test-run';
const behaviorContracts = [{ id: 'behavior-one', requirement: 'Implement the required boundary.' }];
const context = {
  assigned_evidence_ids: ['PROJECT-ONE'],
  injected_evidence_ids: ['PROJECT-ONE'],
  injected_context: 'evidence',
  context_sha256: 'context-hash',
  retrievals: [],
  legacy_receipt_correction: null,
};

function sourcePacket({ packetId, delivered }) {
  const plan = '1. Implement the full contract.\n2. Verify the negative path.';
  return {
    schema_version: 1,
    packet_id: packetId,
    judge_contract_sha256: 'v1-contract',
    source: { run_id: 'run-one', source_schema_version: 5, source_record_sha256: packetId },
    task: 'Implement the feature.',
    private_reference_plan: 'Reference implementation plan.',
    behavior_contracts: behaviorContracts,
    evidence_catalog: [{ id: 'PROJECT-ONE', source: 'project', role: 'required', text: 'Required evidence.' }],
    plan,
    handoff: {
      mode: delivered ? 'typed_replay_fresh' : 'none',
      plan_bank_id: 'bank-one',
      plan_id: 'plan-one',
      plan_revision: 1,
      plan_sha256: 'plan-hash',
      delivered_plan_sha256: delivered ? 'plan-hash' : null,
      source_plan_chars: plan.length,
      delivered_plan_chars: delivered ? plan.length : null,
      delivery_truncated: false,
      typed_decision_sent: delivered,
      executor_reused_provider: false,
      executor_reused_session: false,
    },
    planner_context: context,
    executor_context: { ...context, assigned_evidence_ids: [], injected_evidence_ids: [] },
    planner_trajectory: { events: [{ stream_sequence: 1, elapsed_ms: 1, event: { type: 'message_delta', text: 'plan' } }] },
    executor_trajectory: { events: [{ stream_sequence: 1, elapsed_ms: 1, event: { type: 'message_delta', text: delivered ? 'delivered' : 'discarded' } }] },
    baseline_tree_sha256: 'baseline',
    baseline_files: { 'src/a.js': 'old' },
    final_tree_sha256: delivered ? 'delivered-tree' : 'discarded-tree',
    final_files: { 'src/a.js': delivered ? 'new-delivered' : 'new-discarded' },
    hidden_verification: { checks: [{ id: 'behavior-one', passed: delivered, detail: 'receipt' }] },
    retries: [],
    provider_error: null,
  };
}

function fixture() {
  const packets = [sourcePacket({ packetId: 'discarded', delivered: false }), sourcePacket({ packetId: 'delivered', delivered: true })];
  const index = {
    entries: [
      { packet_id: 'discarded', scenario: 'scenario-one', lane: 'bank_none_discarded', repetition: 1 },
      { packet_id: 'delivered', scenario: 'scenario-one', lane: 'bank_none_typed_replay', repetition: 1 },
    ],
  };
  return { packets, index };
}

test('packet builder produces one blinded pair and one immutable plan unit', () => {
  const { packets, index } = fixture();
  const built = buildPackets(packets, index, contractSha256);
  assert.equal(built.plans.length, 1);
  assert.equal(built.pairs.length, 1);
  assert.equal(built.treatmentIndex[0].treatment, 'none');
  assert.equal(built.plans[0].plan, packets[0].plan);
  assert.doesNotMatch(JSON.stringify(built.pairs[0]), /bank_none/);
  assert.equal(Object.hasOwn(built.pairs[0], 'treatment'), false);
});

test('packet builder fails closed on plan truncation', () => {
  const { packets, index } = fixture();
  packets[1].handoff.delivery_truncated = true;
  packets[1].handoff.delivered_plan_sha256 = 'different';
  assert.throws(() => buildPackets(packets, index, contractSha256), /complete frozen plan/);
});

test('plan verdict requires exact autoregressive key and member order', () => {
  const { packets, index } = fixture();
  const packet = buildPackets(packets, index, contractSha256).plans[0];
  const verdict = {
    schema_version: 2,
    packet_id: packet.packet_id,
    judge_contract_sha256: contractSha256,
    judge: { model: MODEL, run_label: runLabel },
    behaviors: [{ behavior_id: 'behavior-one', coverage: 'correct', confidence: 'high', rationale: 'The plan directly covers the required boundary.', citations: [{ locator: 'plan', claim: 'The first step implements the contract.' }] }],
    knowledge: [{ evidence_id: 'PROJECT-ONE', influence: 'used_correctly', confidence: 'high', rationale: 'The plan operationalizes the assigned project evidence.', citations: [{ locator: 'planner_context', claim: 'PROJECT-ONE was injected before planning.' }] }],
    overall: { plan_quality: 'good', implementation_readiness: 'ready', confidence: 'high', rationale: 'The plan is concrete and covers implementation and verification.', citations: [{ locator: 'plan', claim: 'Both implementation and negative verification are specified.' }] },
    limitations: [],
  };
  assert.equal(validatePlanVerdict(verdict, packet, runLabel), verdict);
  const reordered = { packet_id: verdict.packet_id, schema_version: 2, judge_contract_sha256: contractSha256, judge: verdict.judge, behaviors: verdict.behaviors, knowledge: verdict.knowledge, overall: verdict.overall, limitations: [] };
  assert.throws(() => validatePlanVerdict(reordered, packet, runLabel), /keys\/order/);
});

test('pair verdict and independent audit are accepted only as whole model outputs', () => {
  const { packets, index } = fixture();
  const built = buildPackets(packets, index, contractSha256);
  const packet = built.pairs[0];
  const deliveredArm = built.treatmentIndex[0].delivered_arm;
  const verdict = {
    schema_version: 2,
    packet_id: packet.packet_id,
    judge_contract_sha256: contractSha256,
    judge: { model: MODEL, run_label: runLabel },
    behaviors: [{ behavior_id: 'behavior-one', execution_effect: 'delivered_better', causal_boundary: 'plan_delivery', confidence: 'high', rationale: 'Only the delivered arm completed the required behavior.', citations: [{ locator: `arm_${deliveredArm}.hidden_verification`, claim: 'The delivered arm passed the behavior receipt.' }] }],
    comparison: { delivered_arm: deliveredArm, execution_effect: 'delivered_better', planner_respected: 'fully', completion_honesty_effect: 'equivalent', confidence: 'high', rationale: 'The delivered plan governed the successful implementation.', citations: [{ locator: `arm_${deliveredArm}.handoff`, claim: 'This arm received the complete approved plan.' }] },
    limitations: [],
  };
  assert.equal(validatePairVerdict(verdict, packet, runLabel, deliveredArm), verdict);
  const audit = { schema_version: 2, candidate_packet_id: packet.packet_id, judge: { model: MODEL, run_label: `${runLabel}:audit` }, contradictions: [], decision: 'accept', rationale: 'The verdict is internally consistent with the cited packet evidence.', citations: [{ locator: 'candidate_verdict', claim: 'Local and aggregate effects agree.' }] };
  assert.equal(validateAudit(audit, packet.packet_id, `${runLabel}:audit`), audit);
  assert.throws(() => validateAudit({ ...audit, contradictions: ['contradiction'] }, packet.packet_id, `${runLabel}:audit`), /retained contradictions/);
});
