# Planning and context evaluation

The current protocol is v3. Its frozen-design candidate is
[`PREREGISTRATION_V3.md`](./PREREGISTRATION_V3.md). The earlier v2 live output
is diagnostic only: it pasted a rendered plan into a new user message and did
not exercise Clark's typed `ProposedPlan` to `PlanDecision` execution boundary.
It cannot support a claim about whether the product respects an approved plan.
The repository-grounded failure map is
[`WEAK_POINTS_V3.md`](./WEAK_POINTS_V3.md).

This benchmark measures two separate questions:

1. Does Plan Mode produce grounded, read-only, implementation-ready plans?
2. Does a frozen plan improve a fresh executor's result, and do Project Memory,
   Org Memory, or Scout discoveries improve that plan?

It uses twelve executable synthetic multi-repository families across
compliance, distributed systems, security, payments, data-platform, product
configuration, and client-sync domains. They include regional audit export,
event envelopes, permission/collaboration preferences, OAuth key rotation,
payment idempotency, search indexing, cache invalidation, legal hold, mobile
sync, feature-flag scope, shard rebalancing, and template versioning. Every
scenario has a 60+ file independent workspace, an 8–15 file oracle
implementation, at least 25 evidence candidates, five behavioral hidden checks,
visible Node tests, and required plus stale/conflicting evidence. The
eligibility gate reverts every oracle-changed file individually and requires a
hidden behavioral regression, preventing decorative complexity from counting;
it also proves a second facade/delegated module layout passes the same hidden
behaviors so the grader is not tied to one exact source structure.
Evidence relevance labels remain private to the grader; the model sees only
source identity, evidence ID, and the discovery text.

The source-factorial lanes cover all eight Project/Org/Scout planner
combinations. Additional controls measure no plan, context given only to the
executor, context given to both phases, oracle context, irrelevant context,
stale context, conflicting context, and a planner run whose proposal is
discarded before fresh execution. Three product-real handoff arms isolate the
execution boundary:

- `real_plan_current`: approve the typed proposal with `Current` and continue
  in the same provider/session;
- `real_plan_fresh`: approve the typed proposal with `Fresh` and continue in
  the same provider/session;
- `typed_replay_fresh`: persist and replay the typed proposal into a new
  provider/session before sending a real `Fresh` decision.

The old Markdown-fresh lane remains only as a delivery-mechanism control.
The confirmatory handoff lanes use an append-only plan bank: one generated
proposal is frozen with its exact planning contract, task, context, retrieval
receipts, trajectory, ID, revision, and hash, then replayed byte-for-byte into
Markdown, typed-replay, and discarded executor arms. Separate `bank_none_*`
and `bank_all_*` trios isolate handoff from planner sampling variance.

The `bank_all_*` treatment is independently frozen for three knowledge-delivery
mechanisms:

- deferred discovery, where the initial request sees `tool_search` and the
  model must activate relevant production tools;
- preactivated tools, where the exact registered memory, Org, and Scout schemas
  are visible on the first request without a discovery turn;
- a host-prefetched capsule containing the same evidence IDs and provenance,
  with memory and Scout retrieval disabled for that planner.

The immutable bank key includes the mechanism, so the 38-lane, 12-scenario
offline matrix produces 456 cases and 48 frozen plan entries: none and all
under deferred discovery, plus all under preactivation and prefetch. Each
mechanism reuses one plan across its Markdown, typed-replay, and discarded
execution arms.

Project facts use the real
`.clark/memory/MEMORY.md` catalog plus frontmatter-bearing fact files and must
be recalled with the production memory tool. A loopback Clark gateway proxies
model traffic while serving production-shaped Org Memory search and Scout
enrollment/bitemporal snapshot contracts. It records queries, rankings,
exact safe request/response bodies and hashes, evidence IDs, temporal fields,
and Scout coverage gaps without exposing hidden relevance labels.

Each JSONL case retains:

- requested and effective model, product route, and Free-tier proof;
- fixture, task, Plan Mode contract, and context hashes;
- injected evidence IDs and evidence citations in the proposed plan;
- planner read-only digest, plan grounding, and negative-evidence use;
- typed handoff mode, plan ID, revision, content hash, decision, and
  provider/session continuity;
- normalized planner and executor `AgentEvent` trajectories, including raw
  tool inputs and public tool outputs;
- factual hidden-check and retrieval receipts retained only as evidence for the
  model judge, never promoted to semantic quality or adherence scores;
- schema-v4 causal attribution for every failed live behavior, including the
  earliest supported cause, conservative candidate causes when unresolved,
  and the first potentially mutating trajectory event;
- fresh-executor hidden checks and first failing contract;
- tokens, context size, tool calls, latency, timeouts, and provider-reported
  upstream cost;
- every route/phase retry, requested and actual wait, partial-output state, and
  whether a discarded executor workspace had mutations.

The report includes per-lane means and paired hierarchical bootstrap confidence
intervals (scenarios first, repetitions second). Runs append JSONL and resume by
scenario/lane/repetition key. Treat offline mode as harness validation only,
not model-quality evidence; offline causal-attribution receipts are explicitly
marked not applicable because the oracle implementation is applied directly.

Twelve real-provider lifecycle probes additionally reproduce the remaining
non-model failure boundaries: feedback loss, stale revision approval, complete
long-plan delivery, generic mode-switch approval bypass, duplicate-decision
context loss, a conflicting delayed decision
reopening approval, non-terminal proposal emission, approval racing planner
termination, approval-event deferral, planner read-authorization leakage into
Fresh execution, unplanned writes, and approval after workspace drift. Every
offline artifact includes `lifecycle-findings.json`, which maps each remaining
finding to its deterministic test and explains why the approved plan loses
identity, exact bytes, isolation, enforcement, or truth at that boundary.

Separate treatment probes verify the repaired boundaries: approved plans regain
typed developer authority after resume and compaction, checklist updates must
preserve host-assigned plan-step IDs, a completed step reinjects the next
step's evidence contract before generation, and an attempted terminal answer
reopens execution until the typed contract is reconciled.

## Deterministic validation

```bash
cargo test -p provider-local --example planning_eval
cargo run -p provider-local --example planning_eval -- \
  --offline --repetitions 3 \
  --output target/planning-eval-v3/offline-gate
```

Offline rows intentionally report execution-given-plan and retrieval-treatment
as not applicable. They validate fixture integrity, schemas, hidden checks,
oracles, lane construction, and report generation; they are not model-quality
observations.

## Live runs

Live mode is intentionally fail-closed. It accepts only `clark-code:free`,
loads the authenticated catalog before benchmark cases, and requires the
option to carry all three Clark product markers: `tier_id=clark-code`,
`model_option_id=free`, and `label=Free`. It then probes the product route and
accepts only DeepSeek V4 Flash Latest or its concrete numeric dated snapshot as
the effective response model. It never accepts an arbitrary raw model or a
paid fallback lacking that authenticated Free-tier mapping.

```bash
CLARK_CODE_LIVE=1 \
CLARK_CODE_MODEL=clark-code:free \
cargo run -p provider-local --example planning_eval -- \
  --live \
  --scenarios regional-audit-export \
  --lanes no_plan,bank_none_markdown,bank_none_typed_replay,bank_all_markdown,bank_all_typed_replay \
  --repetitions 3 \
  --max-live-cases 21 \
  --output target/planning-eval-v3/live-pilot
```

`CLARK_CODE_API_KEY` and optional `CLARK_CODE_BASE_URL` may be set in the
environment or the repository's ignored `.env`. The live cap counts
scenario/lane/repetition cases plus any missing frozen plan-bank generations;
the route-verification probe is separate.
The provider's `usage.cost` is retained as upstream-cost telemetry; the
catalog's product mapping, rather than that internal price signal, establishes
whether the user-facing route is Free.

Planner and executor model loops use the production uncapped iteration
default. The harness retains a ten-minute wall-clock timeout per phase so an
unattended evidence run can terminate a hung transport; that timeout is an
infrastructure outcome, not a model-quality score or a product loop limit.

Do not start a v3 live run until every deterministic gate in
`PREREGISTRATION_V3.md` passes and the operator explicitly resumes live
evaluation. In particular, the live matrix must include the typed handoff arms
above; a Markdown-only pilot is not a substitute.

HTTP 429/502/503/504 capacity failures do not become model-quality failures.
The route probe retries with 15, 30, 60, 120, and 300 second delays. Planner and
executor phases retry from clean state with 60 and 300 second delays; every
wait is capped at five minutes and prints progress every 30 seconds. No paid
fallback exists.

The v3 design and decision rules live beside this README in
`PREREGISTRATION_V3.md`. The eventual high-fidelity matrix requires at least
12 independent scenario families and three repetitions per primary
scenario/lane pair; five repetitions are preferred when Free-tier capacity
permits.

## Model-owned comparative judgment

Export source-grounded judge packets, then run the Qwen v2 comparative judge:

```bash
cargo run -p provider-local --example planning_eval -- \
  --judge-input target/planning-eval-v3/live-pilot \
  --output target/planning-eval-v3/live-pilot-judge-v2

JUDGE_RUN_LABEL=live-pilot-qwen-v2 \
node crates/provider-local/examples/planning_eval/judge_qwen_v2.mjs \
  target/planning-eval-v3/live-pilot-judge-v2
```

The v2 judge uses one plan-only Qwen judgment per immutable plan-bank entry,
one blinded direct Qwen comparison per typed-versus-discarded pair, and one
final Qwen adjudication across the context treatments. Every candidate is read
by a separate Qwen audit call. A rejection discards the complete candidate and
regenerates it; the host never merges or repairs semantic fields. Exact ordered
keys, enums, IDs, and plan-byte identity are fail-closed protocol boundaries.
The driver retries transport, capacity, malformed output, and audit rejection
with waits up to five minutes and writes request, audit, token, cost, and hash
receipts beside the verdicts.

Validate the ordered schemas and pairing boundary without a model call:

```bash
node --test crates/provider-local/examples/planning_eval/judge_qwen_v2.test.mjs
node crates/provider-local/examples/planning_eval/judge_qwen_v2.mjs \
  target/planning-eval-v3/offline-gate-judge-v2 --packets-only
```
