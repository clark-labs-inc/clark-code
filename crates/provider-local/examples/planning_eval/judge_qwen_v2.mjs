#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  MODEL,
  TREATMENTS,
  buildPackets,
  readJsonl,
  sha256,
  validateAudit,
  validateFinalVerdict,
  validatePairVerdict,
  validatePlanVerdict,
  writeJsonAtomic,
  writeJsonlAtomic,
} from './judge_qwen_v2_lib.mjs';

const driverPath = fileURLToPath(import.meta.url);
const sourceDirectory = path.dirname(driverPath);
const commandArguments = process.argv.slice(2);
const packetsOnly = commandArguments.includes('--packets-only');
const outputArgument = commandArguments.find((argument) => argument !== '--packets-only');
const outputDirectory = path.resolve(outputArgument ?? 'target/planning-eval-judge-v2');
const contract = fs.readFileSync(path.join(sourceDirectory, 'JUDGE_CONTRACT_V2.md'), 'utf8');
const contractSha256 = sha256(contract);
const driverSha256 = sha256(fs.readFileSync(driverPath));
const librarySha256 = sha256(fs.readFileSync(path.join(sourceDirectory, 'judge_qwen_v2_lib.mjs')));
const runLabel = process.env.JUDGE_RUN_LABEL ?? 'qwen-planning-comparative-v2';
const auditLabel = `${runLabel}:independent-audit`;
const baseUrl = (environment('CLARK_CODE_BASE_URL') ?? 'https://api.clarkslabs.com/v1').replace(/\/$/, '');
const apiKey = environment('CLARK_CODE_API_KEY');

const sourcePackets = readJsonl(path.join(outputDirectory, 'judge-packets.jsonl'));
const index = JSON.parse(fs.readFileSync(path.join(outputDirectory, 'judge-index.json'), 'utf8'));
if (sourcePackets.length === 0) throw new Error('export judge packets before running comparative judge v2');
const { plans, pairs, treatmentIndex } = buildPackets(sourcePackets, index, contractSha256);
if (plans.length !== pairs.length) throw new Error('plan and pair packet counts differ');
fs.writeFileSync(path.join(outputDirectory, 'JUDGE_V2_INSTRUCTIONS.md'), contract);
writeJsonlAtomic(path.join(outputDirectory, 'judge-v2-plan-packets.jsonl'), plans);
writeJsonlAtomic(path.join(outputDirectory, 'judge-v2-pair-packets.jsonl'), pairs);
if (packetsOnly) {
  console.error(`comparative judge v2 exported ${plans.length} immutable plans and ${pairs.length} blinded pairs`);
  process.exit(0);
}
if (!apiKey) throw new Error('CLARK_CODE_API_KEY is required');

const receiptPath = path.join(outputDirectory, 'judge-v2-request-receipts.jsonl');
const attemptPath = path.join(outputDirectory, 'judge-v2-model-attempts.jsonl');
const auditPath = path.join(outputDirectory, 'judge-v2-audits.jsonl');
const transportDelays = [15_000, 30_000, 60_000, 120_000, 300_000];
const semanticDelays = [15_000, 30_000, 60_000, 120_000, 300_000];
const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

function environment(name) {
  const direct = process.env[name];
  if (direct?.trim()) return direct.trim();
  try {
    const line = fs.readFileSync(path.join(process.cwd(), '.env'), 'utf8')
      .split('\n').find((value) => value.trim().startsWith(`${name}=`));
    return line?.split('=').slice(1).join('=').trim().replace(/^['"]|['"]$/g, '');
  } catch {
    return undefined;
  }
}

function appendJsonl(file, value) {
  fs.appendFileSync(file, `${JSON.stringify(value)}\n`);
}

function normalizedModel(value) {
  return String(value ?? '').toLowerCase().replace(/[^a-z0-9]/g, '');
}

async function wait(scope, milliseconds) {
  const started = Date.now();
  let remaining = milliseconds;
  while (remaining > 0) {
    console.error(`${scope}: retrying in ${Math.ceil(remaining / 1000)}s`);
    await sleep(Math.min(30_000, remaining));
    remaining = milliseconds - (Date.now() - started);
  }
}

async function requestModel({ phase, packetId, system, user, maxTokens }) {
  const body = {
    model: MODEL,
    messages: [{ role: 'system', content: system }, { role: 'user', content: JSON.stringify(user) }],
    temperature: 0,
    max_tokens: maxTokens,
    stream: false,
    reasoning: { effort: 'low', exclude: true },
  };
  for (let attempt = 1; ; attempt += 1) {
    let response;
    try {
      response = await fetch(`${baseUrl}/chat/completions`, {
        method: 'POST',
        headers: { authorization: `Bearer ${apiKey}`, 'content-type': 'application/json' },
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(300_000),
      });
    } catch (error) {
      appendJsonl(receiptPath, { phase, packet_id: packetId, request_attempt: attempt, status: 'transport_error', wait_ms: attempt <= transportDelays.length ? transportDelays[attempt - 1] : 0, error: String(error) });
      if (attempt > transportDelays.length) throw error;
      await wait(`${phase} ${packetId} transport`, transportDelays[attempt - 1]);
      continue;
    }
    const raw = await response.text();
    if ([429, 502, 503, 504].includes(response.status) && attempt <= transportDelays.length) {
      const retryAfter = Number(response.headers.get('retry-after'));
      const requested = Number.isFinite(retryAfter) && retryAfter > 0
        ? Math.min(retryAfter * 1000, 300_000)
        : transportDelays[attempt - 1];
      appendJsonl(receiptPath, { phase, packet_id: packetId, request_attempt: attempt, status: response.status, wait_ms: requested, error: raw.slice(0, 500) });
      await wait(`${phase} ${packetId} status ${response.status}`, requested);
      continue;
    }
    if (!response.ok) throw new Error(`${phase} response ${response.status}: ${raw.slice(0, 500)}`);
    const envelope = JSON.parse(raw);
    if (normalizedModel(envelope.model) !== normalizedModel(MODEL)) {
      throw new Error(`${phase} resolved to ${envelope.model}, expected ${MODEL}`);
    }
    const content = envelope.choices?.[0]?.message?.content;
    if (typeof content !== 'string') throw new Error(`${phase} omitted message content`);
    appendJsonl(receiptPath, {
      phase,
      packet_id: packetId,
      request_attempt: attempt,
      status: response.status,
      wait_ms: 0,
      response_id: envelope.id ?? null,
      effective_model: envelope.model,
      input_tokens: envelope.usage?.prompt_tokens ?? 0,
      output_tokens: envelope.usage?.completion_tokens ?? 0,
      cost_usd: envelope.usage?.cost ?? 0,
    });
    return content;
  }
}

function parseModelJson(content) {
  const value = JSON.parse(content.trim());
  if (content.includes('```')) throw new Error('model output contained a Markdown fence');
  return value;
}

async function generateValid({ phase, packetId, system, user, maxTokens, validate }) {
  let previousFailure = null;
  for (let attempt = 1; attempt <= 5; attempt += 1) {
    try {
      const content = await requestModel({
        phase,
        packetId,
        system,
        user: previousFailure ? { packet: user, previous_invalid_output: previousFailure } : user,
        maxTokens,
      });
      const value = validate(parseModelJson(content));
      appendJsonl(attemptPath, { phase, packet_id: packetId, semantic_attempt: attempt, accepted_schema: true, content });
      return value;
    } catch (error) {
      previousFailure = String(error);
      appendJsonl(attemptPath, { phase, packet_id: packetId, semantic_attempt: attempt, accepted_schema: false, error: previousFailure });
      if (attempt === 5) throw new Error(`${phase} exhausted model-output retries: ${previousFailure}`);
      await wait(`${phase} ${packetId} malformed`, semanticDelays[attempt - 1]);
    }
  }
  throw new Error(`${phase} unreachable retry state`);
}

function commonPrompt(label) {
  return `You are ${label}. Return exactly one JSON object with keys in the requested order and no Markdown or prose. The host validates identity, ordered structure, and whole-response lifecycle but never changes semantic output. Generate local evidence judgments before aggregate conclusions. Use only exact enum values. Every rationale must be concise and at least twenty characters. Every citations array must contain one or more objects with ordered keys exactly [locator,claim], for example [{"locator":"plan","claim":"The plan states the requirement."}]. Never use citation strings or keys named id, packet_id, packet_locator, line, or source. Never expose private chain-of-thought. judge must be {"model":"${MODEL}","run_label":"${runLabel}"}.`;
}

function planPrompt(packet) {
  const behaviorIds = packet.behavior_contracts.map(({ id }) => id);
  const evidenceIds = packet.planner_context.assigned_evidence_ids;
  return `${commonPrompt('a plan-only software judge')} Judge the frozen plan without using any executor outcome. Do not infer quality from length, keywords, citations, tool calls, or checklist counts. Return ordered keys [schema_version,packet_id,judge_contract_sha256,judge,behaviors,knowledge,overall,limitations]. schema_version=2, packet_id=${JSON.stringify(packet.packet_id)}, judge_contract_sha256=${JSON.stringify(packet.judge_contract_sha256)}. behaviors must contain IDs exactly in this order ${JSON.stringify(behaviorIds)}; item keys [behavior_id,coverage,confidence,rationale,citations], coverage=[omitted,incorrect,partial,correct], confidence=[low,medium,high]. knowledge must contain IDs exactly in this order ${JSON.stringify(evidenceIds)}; item keys [evidence_id,influence,confidence,rationale,citations], influence=[not_used,cited_only,used_correctly,used_incorrectly,unverifiable]. Judge influence, not factual availability. overall keys [plan_quality,implementation_readiness,confidence,rationale,citations], plan_quality=[poor,mixed,good,excellent], implementation_readiness=[not_ready,partially_ready,ready,exceptionally_ready]. limitations is an array.`;
}

function pairPrompt(packet) {
  const behaviorIds = packet.behavior_contracts.map(({ id }) => id);
  return `${commonPrompt('a blinded paired software-trajectory judge')} The two arms share one frozen plan and differ in whether its complete bytes were delivered. Identify the delivered arm from the handoff receipt. Compare both complete trajectories directly; do not score them independently and subtract. Hidden checks are factual evidence, not scores. Return ordered keys [schema_version,packet_id,judge_contract_sha256,judge,behaviors,comparison,limitations]. schema_version=2, packet_id=${JSON.stringify(packet.packet_id)}, judge_contract_sha256=${JSON.stringify(packet.judge_contract_sha256)}. behaviors must contain IDs exactly in this order ${JSON.stringify(behaviorIds)}; item keys [behavior_id,execution_effect,causal_boundary,confidence,rationale,citations]. execution_effect=[delivered_better,equivalent,delivered_worse,mixed,unresolved], causal_boundary=[plan,plan_delivery,executor,verification,capacity,fixture,mixed,unresolved]. comparison keys [delivered_arm,execution_effect,planner_respected,completion_honesty_effect,confidence,rationale,citations], delivered_arm=[a,b], planner_respected=[no,partially,mostly,fully,unresolved], completion_honesty_effect uses the execution_effect enum. confidence=[low,medium,high]. limitations is an array.`;
}

function auditPrompt(candidateId, auditRunLabel) {
  return `You are an independent Qwen semantic audit. Read the complete source packet and candidate verdict. Find internal contradictions, incorrect claims about packet facts or identities, causal claims unsupported by cited evidence, and aggregate conclusions inconsistent with the candidate's own local judgments. Do not reject merely because another reasonable judgment is possible. Do not rewrite or repair the candidate. Return ordered keys [schema_version,candidate_packet_id,judge,contradictions,decision,rationale,citations]. schema_version=2, candidate_packet_id=${JSON.stringify(candidateId)}, judge={"model":"${MODEL}","run_label":"${auditRunLabel}"}. First emit contradictions as an array of precise strings, then decision=[accept,reject]. accept requires an empty contradictions array; reject requires at least one contradiction. rationale is required. citations must be a non-empty array of objects with ordered keys exactly [locator,claim], for example [{"locator":"candidate_verdict.overall","claim":"The aggregate matches the local judgments."}]. Never use citation strings or keys named id, packet_id, packet_locator, line, or source. Return JSON only.`;
}

async function independentlyAccepted({ kind, packet, system, user, validateCandidate }) {
  let rejection = null;
  for (let attempt = 1; attempt <= 5; attempt += 1) {
    const candidate = await generateValid({
      phase: `${kind}_candidate`,
      packetId: packet.packet_id,
      system,
      user: rejection ? { packet: user, independent_audit_rejection: rejection } : user,
      maxTokens: kind === 'plan' ? 12_000 : kind === 'pair' ? 8_000 : 6_000,
      validate: validateCandidate,
    });
    const audit = await generateValid({
      phase: `${kind}_audit`,
      packetId: packet.packet_id,
      system: auditPrompt(packet.packet_id, auditLabel),
      user: { source_packet: user, candidate_verdict: candidate },
      maxTokens: 5_000,
      validate: (value) => validateAudit(value, packet.packet_id, auditLabel),
    });
    appendJsonl(auditPath, { kind, packet_id: packet.packet_id, candidate_sha256: sha256(JSON.stringify(candidate)), audit });
    if (audit.decision === 'accept') return candidate;
    rejection = audit;
    if (attempt === 5) throw new Error(`${kind} ${packet.packet_id} rejected by five independent audits`);
    await wait(`${kind} ${packet.packet_id} audit rejection`, semanticDelays[attempt - 1]);
  }
  throw new Error(`${kind} ${packet.packet_id} unreachable audit state`);
}

async function judgedCollection({ kind, packets: packetList, output, cached, prompt, validate, user }) {
  const byId = new Map(cached.map((value) => [value.packet_id, value]));
  for (const [indexValue, packet] of packetList.entries()) {
    const existing = byId.get(packet.packet_id);
    if (existing) {
      validate(existing, packet);
      console.error(`${kind} ${indexValue + 1}/${packetList.length}: cached ${packet.packet_id}`);
      continue;
    }
    console.error(`${kind} ${indexValue + 1}/${packetList.length}: judging ${packet.packet_id}`);
    const value = await independentlyAccepted({
      kind,
      packet,
      system: prompt(packet),
      user: user(packet),
      validateCandidate: (candidate) => validate(candidate, packet),
    });
    byId.set(packet.packet_id, value);
    writeJsonlAtomic(output, packetList.filter((item) => byId.has(item.packet_id)).map((item) => byId.get(item.packet_id)));
  }
  return packetList.map((packet) => byId.get(packet.packet_id));
}

const planOutput = path.join(outputDirectory, 'judge-v2-plan-verdicts.jsonl');
const planVerdicts = await judgedCollection({
  kind: 'plan',
  packets: plans,
  output: planOutput,
  cached: readJsonl(planOutput),
  prompt: planPrompt,
  validate: (value, packet) => validatePlanVerdict(value, packet, runLabel),
  user: (packet) => packet,
});
const planById = new Map(planVerdicts.map((value) => [value.packet_id, value]));
const deliveredArmByPair = new Map(treatmentIndex.map((item) => [item.pair_packet_id, item.delivered_arm]));
const pairOutput = path.join(outputDirectory, 'judge-v2-pair-verdicts.jsonl');
const pairVerdicts = await judgedCollection({
  kind: 'pair',
  packets: pairs,
  output: pairOutput,
  cached: readJsonl(pairOutput),
  prompt: pairPrompt,
  validate: (value, packet) => validatePairVerdict(value, packet, runLabel, deliveredArmByPair.get(packet.packet_id)),
  user: (packet) => ({ packet, accepted_plan_verdict: planById.get(packet.source.plan_packet_id) }),
});
const pairById = new Map(pairVerdicts.map((value) => [value.packet_id, value]));

const evidence = treatmentIndex.map((item) => ({
  treatment: item.treatment,
  repetition: item.repetition,
  delivered_arm: item.delivered_arm,
  plan_verdict: planById.get(item.plan_packet_id),
  pair_verdict: pairById.get(item.pair_packet_id),
}));
const evaluationId = sha256(`final-judge-v2:${contractSha256}:${evidence.map((item) => sha256(JSON.stringify(item))).join(':')}`);
const finalPacket = {
  schema_version: 2,
  evaluation_id: evaluationId,
  judge_contract_sha256: contractSha256,
  run: { run_id: sourcePackets[0].source.run_id, frozen_plans: plans.length, matched_pairs: pairs.length, repetitions: [...new Set(treatmentIndex.map(({ repetition }) => repetition))].length },
  treatment_order: TREATMENTS,
  accepted_model_evidence: evidence,
};
writeJsonAtomic(path.join(outputDirectory, 'judge-v2-final-packet.json'), finalPacket);

const finalPrompt = `${commonPrompt('the final planning-evaluation adjudicator')} The packet contains only plan verdicts and direct matched-pair verdicts that independent Qwen audits accepted. Assess repeated evidence, disagreements, capacity limitations, and whether plan delivery and additional context helped. Do not count labels or convert them to numeric scores. Return ordered keys [schema_version,evaluation_id,judge_contract_sha256,judge,mechanisms,plan_delivery_effect,context_effect,overall,limitations]. schema_version=2, evaluation_id=${JSON.stringify(evaluationId)}, judge_contract_sha256=${JSON.stringify(contractSha256)}. mechanisms must contain treatments exactly in this order ${JSON.stringify(TREATMENTS)}; item keys [treatment,plan_effect,execution_effect,evidence_use,confidence,rationale,citations], plan_effect and execution_effect=[beneficial,neutral,harmful,mixed,unresolved], evidence_use=[absent,selective,effective,misused,mixed,unresolved]. plan_delivery_effect and context_effect keys [verdict,confidence,rationale,citations], verdict uses the same effect enum. overall keys [conclusion,confidence,rationale,citations], conclusion=[supported,promising_not_proven,no_benefit,harmful,unresolved]. confidence=[low,medium,high]. limitations is an array.`;
const finalVerdictPath = path.join(outputDirectory, 'judge-v2-final-verdict.json');
let finalVerdict;
if (fs.existsSync(finalVerdictPath)) {
  finalVerdict = validateFinalVerdict(JSON.parse(fs.readFileSync(finalVerdictPath, 'utf8')), finalPacket, runLabel);
} else {
  finalVerdict = await independentlyAccepted({
    kind: 'final',
    packet: { packet_id: evaluationId },
    system: finalPrompt,
    user: finalPacket,
    validateCandidate: (value) => validateFinalVerdict(value, finalPacket, runLabel),
  });
  writeJsonAtomic(finalVerdictPath, finalVerdict);
}

const receipts = readJsonl(receiptPath);
const manifest = {
  schema_version: 2,
  evaluation_id: evaluationId,
  judge_contract_sha256: contractSha256,
  judge_driver_sha256: driverSha256,
  judge_library_sha256: librarySha256,
  model: MODEL,
  run_label: runLabel,
  frozen_plans: plans.length,
  matched_pairs: pairs.length,
  accepted_plan_verdicts: planVerdicts.length,
  accepted_pair_verdicts: pairVerdicts.length,
  independent_audits: readJsonl(auditPath).length,
  request_receipts: receipts.length,
  input_tokens: receipts.reduce((sum, item) => sum + (item.input_tokens ?? 0), 0),
  output_tokens: receipts.reduce((sum, item) => sum + (item.output_tokens ?? 0), 0),
  provider_reported_cost_usd: receipts.reduce((sum, item) => sum + (item.cost_usd ?? 0), 0),
  plan_packets_sha256: sha256(fs.readFileSync(path.join(outputDirectory, 'judge-v2-plan-packets.jsonl'))),
  pair_packets_sha256: sha256(fs.readFileSync(path.join(outputDirectory, 'judge-v2-pair-packets.jsonl'))),
  plan_verdicts_sha256: sha256(fs.readFileSync(planOutput)),
  pair_verdicts_sha256: sha256(fs.readFileSync(pairOutput)),
  final_verdict_sha256: sha256(fs.readFileSync(finalVerdictPath)),
};
writeJsonAtomic(path.join(outputDirectory, 'judge-v2-manifest.json'), manifest);

const report = `# Qwen comparative planning judgment v2\n\n- Conclusion: \`${finalVerdict.overall.conclusion}\` (${finalVerdict.overall.confidence})\n- Plan delivery: \`${finalVerdict.plan_delivery_effect.verdict}\` (${finalVerdict.plan_delivery_effect.confidence})\n- Additional context: \`${finalVerdict.context_effect.verdict}\` (${finalVerdict.context_effect.confidence})\n\n${finalVerdict.overall.rationale}\n\n## Mechanisms\n\n| Treatment | Plan | Execution | Evidence use | Confidence |\n|---|---|---|---|---|\n${finalVerdict.mechanisms.map((item) => `| ${item.treatment} | ${item.plan_effect} | ${item.execution_effect} | ${item.evidence_use} | ${item.confidence} |`).join('\n')}\n\n## Limitations\n\n${finalVerdict.limitations.map((item) => `- ${item}`).join('\n') || 'None reported by the model.'}\n`;
fs.writeFileSync(path.join(outputDirectory, 'judge-v2-report.md'), report);
console.error(`comparative judge v2 accepted ${planVerdicts.length} plan verdicts, ${pairVerdicts.length} pair verdicts, and one final adjudication`);
