# Clark Plan Mode Evaluation v3 — Handoff and Adherence Preregistration

Status: implementation-gate freeze candidate; no v3 live trials have run.

The 17 completed v2 live cases are diagnostic pilot data. They must not be
reported as confirmatory evidence about Clark Plan Mode because the v2 harness
opened a new provider/session and pasted plan Markdown into a user message
instead of exercising Clark's typed plan-decision transition.

This amendment is frozen before any v3 live trial. A later deterministic defect
may cause a documented amendment and a new freeze hash, but no scored live case
may be repaired in place.

## 1. Questions and estimands

The evaluation separates four causal stages:

1. evidence availability and successful retrieval;
2. semantic quality of the proposed plan;
3. fidelity of the approved-plan handoff;
4. executor adherence and final behavior.

Primary handoff estimand:

- paired hidden-behavior Fix Rate for `bank_none_typed_replay -
  bank_none_markdown`;
- paired hidden-behavior Fix Rate for `bank_all_typed_replay -
  bank_all_markdown`;
- paired semantic-decision preservation for both contrasts.

The persisted append-only plan bank is now a deterministic harness gate. Within
each contrast, both executor arms must reference the same bank ID, plan ID,
revision, source hash, planning contract, task prompt, source assignment, and
planner trajectory. Independently replanning each arm is not a valid handoff
contrast. Oracle typed-versus-Markdown remains a positive-control diagnostic,
not the generated-plan primary.

Primary planning estimand:

- paired semantic-plan score for frozen `bank_all - bank_none` proposals;
- paired hidden-behavior Fix Rate for `bank_all_typed_replay -
  bank_none_typed_replay`, where both executors are new sessions and receive no
  independent Project, Org, or Scout evidence;
- both reported intention-to-treat and among bank entries where every assigned
  source produced a successful retrieval receipt.

Secondary estimands:

- `real_plan_current - real_plan_fresh` as an ecological mechanism diagnostic,
  not a planner-only source estimate, because same-session Fresh retains
  provider and retrieval state;
- `oracle_real_fresh - oracle_markdown_fresh`;
- each Project/Org/Scout source's marginal contribution;
- preactivated retrieval versus deferred tool discovery;
- tool-mediated retrieval versus an equivalent host-prefetched evidence
  capsule.

No source benefit may be inferred from a lane in which the assigned retrieval
treatment was not received.

## 2. Product-real handoff arms

### Stage A: handoff isolation

All Stage A planners use repository evidence only.

1. `no_plan`: task-only fresh executor.
2. `markdown_fresh`: planner runs, then a new provider/session receives task
   plus quoted Markdown. This preserves the v2 control.
3. `real_plan_current`: one provider/session; the planner emits a typed
   `ProposedPlan`; the host sends `PlanDecision::Implement(Current)`; the app's
   normal “Implement the approved plan.” turn starts execution.
4. `real_plan_fresh`: same as above with
   `PlanDecision::Implement(Fresh)`.
5. `typed_replay_fresh`: a clean provider/session restores the exact typed
   proposal through `ResumeItem::ProposedPlan`, performs the real Fresh
   decision transition, and executes. This is the clean-retry and
   cross-session typed-plan arm.
6. `plan_discarded`: planner runs but the executor receives no plan.

`real_plan_current` and `real_plan_fresh` cannot share an identical generated
plan and planner transcript without snapshotting a provider session before the
decision. They are therefore a mechanism diagnostic, not the primary causal
handoff estimate.

### Oracle handoff controls

Each scenario has a decision-complete oracle plan whose semantic assertions
must all pass before the scenario is eligible.

1. `oracle_real_fresh`: oracle plan through typed Fresh approval.
2. `oracle_markdown_fresh`: byte-identical oracle plan through Markdown.
3. `oracle_discarded`: oracle plan produced but not delivered.

If `oracle_real_fresh` is weak, no generated-plan result from that scenario may
be used to diagnose planner quality.

## 3. Knowledge-source experiment

Knowledge-source effects are measured only after Stage A shows a functional
typed handoff.

Assigned source factors:

- Project Memory;
- Org Memory;
- Scout cartography.

The complete 2^3 factorial remains diagnostic. Confirmatory source contrasts
are:

- none;
- Project only;
- Org only;
- Scout only;
- all three.

For each source, three delivery mechanisms are distinguished:

1. deferred discovery: production `tool_search` behavior;
2. preactivated tool: same production tool schema without discovery friction;
3. prefetched capsule: the host obtains the same evidence and injects a
   bounded, provenance-preserving evidence capsule.

This separates evidence value from deferred-tool usability.

### Treatment receipt

Each case records:

- source offered;
- discovery query and activated schemas;
- tool arguments;
- tool results and updates;
- successful production-shaped response;
- returned evidence IDs and ordering;
- whether the plan cited and operationalized the evidence.

Project treatment requires a real `memory(recall)` call. Seeing the
always-loaded catalog is not sufficient in the deferred or preactivated
tool-mediated arms. Org treatment in those arms requires a successful
`organization_knowledge` response containing assigned evidence. Scout
treatment requires successful enrollment and snapshot retrieval containing
assigned evidence and coverage. The preactivated arm must additionally prove
the exact registered schemas were visible on the first request.

The prefetched arm instead requires an exact capsule hash, assigned evidence
IDs, provenance, and an absence of Project/Org/Scout retrieval calls. It must
not seed Project Memory or register Scout orchestration for that planner.

## 4. Scenario topology and fidelity

The final confirmatory suite requires at least twelve scenario families in at
least four engineering domains. The current implementation has twelve families
across seven classified domains, two validated source layouts for every oracle
implementation, and real-provider lifecycle handoff probes. Remaining
deterministic gates and explicit operator resumption still precede live trials.

Each family must contain:

- 60–200 source/config/test files;
- 8–15 necessary edits across 4–7 meaningful components;
- at least one shared interface and one stateful or asynchronous boundary;
- a migration, compatibility, or rollout constraint;
- nontrivial baseline tests and hidden integration assertions;
- two validated, structurally different correct implementations;
- at least one plausible locally passing but globally wrong implementation;
- deterministic setup without dependency installation or grading network
  access.

Required information must have an origin:

- explicit task contract;
- repository source or history;
- Project Memory;
- Org Memory;
- Scout observation.

Every hidden assertion records its allowed origins. A hidden assertion that
requires an unstated or undiscoverable product choice quarantines the scenario.

## 5. Temporal and provenance model

Every knowledge item records:

- stable ID;
- source and authoritative scope;
- subject, predicate, object;
- effective time and known/observed time;
- current, superseded, retracted, or uncertain status;
- confidence;
- evidence locator and excerpt;
- supersedes/retracts link where applicable;
- coverage and reachability.

Project Memory uses `.clark/memory/MEMORY.md` as a catalog plus individual fact
files. Org Memory uses the production organization-knowledge response. Scout
uses production enrollment/query schemas and returns stable entities, edges,
claims, bitemporal observations, and explicit coverage.

Repository context includes deterministic Git history and production-shaped
historical commit results. It is constant across source lanes unless
repository history itself is the registered factor.

## 6. Noise, stale, conflict, and coverage controls

Diagnostic conditions:

- relevant current evidence;
- high-volume valid distractors;
- superseded evidence;
- current and superseded evidence together;
- explicit retraction;
- conflicting sources with authoritative resolution;
- source unavailable;
- Scout truncated coverage;
- Scout unreachable environment;
- empty but successful source.

The plan is graded on temporal resolution, provenance choice, and uncertainty
disclosure—not merely whether it mentions an evidence ID.

## 7. Semantic plan grading

Every scenario has behavioral plan assertions sharing IDs with hidden
executable assertions.

Plan checks evaluate:

- exact compatibility boundary;
- precise interfaces and result shapes;
- all deployed consumers/components;
- temporal/source-derived decisions;
- rollout and rollback representation;
- meaningful verification capable of rejecting the wrong implementation.

Lexical path/evidence coverage remains a diagnostic metric only. It is never
the plan-quality primary endpoint.

Oracle plans must score 1.0 on semantic checks. Keyword-rich counterexamples
that omit or contradict the behavior must fail.

## 8. Execution adherence

For each behavior ID:

- `passed_as_planned`;
- `passed_without_plan`;
- `failed_despite_plan`;
- `failed_without_plan`.

Aggregate adherence endpoints:

- planned-behavior recall;
- execution success conditional on a correct planned behavior;
- silent failure count;
- approved versus executor-checklist step similarity;
- planned-path precision/recall;
- unplanned semantic decisions;
- first potentially mutating trajectory event associated with the failure.

`update_plan` is recorded as an execution checklist, not treated as equivalent
to the approved proposal.

## 9. First-cause taxonomy

Every failed behavior receives the earliest supported cause:

1. evidence absent;
2. deferred tool not discovered;
3. retrieval attempted but failed;
4. evidence returned but ignored;
5. plan omitted the behavior;
6. plan contradicted the behavior;
7. approved plan was not delivered;
8. executor contradicted a correct plan;
9. executor omitted a correct planned step;
10. executor verification was nondiscriminating;
11. fixture/grader defect;
12. model-capacity failure.

Ambiguous causes remain `unresolved`; they are not forced into a preferred
category. Case schema v4 records a cause and candidate causes per failed
behavior. It assigns a concrete cause only when retained receipts establish
the boundary: no plan, no delivered bytes, capacity failure, or a correct
delivered plan followed by no workspace mutation. Missing/incorrect plan
semantics and changed workspaces remain `unresolved` when receipts cannot
distinguish evidence treatment, omission, contradiction, or the responsible
tool event.

## 10. Retained receipts

Each case retains:

- exact authenticated route and response model;
- scenario, fixture, hidden-check, prompt, plan, and evidence hashes;
- typed plan ID, revision, status, hash, and implementation context;
- whether provider/session were reused;
- planner and executor normalized tool/checklist/plan/terminal events;
- raw tool arguments and public results;
- retrieval gateway requests and responses;
- Project Memory files and catalog hashes;
- Scout enrollment identity binding, entities, edges, claims, and coverage;
- workspace baseline and final snapshot;
- hidden verification output;
- semantic-plan, adherence, and first-cause classifications;
- tokens, provider-reported cost, latency, retries, and waits.

No hidden reasoning, credentials, authorization headers, secret values, or
unbounded binary payloads are retained.

## 11. Repetition and statistical design

Scenario is the primary cluster. Model calls are paired by scenario, frozen
seed, repetition, and plan where applicable.

Pilot:

- one repetition of every core handoff arm;
- one repetition of source delivery diagnostics;
- excluded from confirmatory estimates.

Confirmatory minimum:

- twelve scenario families;
- three repetitions per primary arm;
- five repetitions preferred for the two primary contrasts.

The preregistered stopping target is a hierarchical-bootstrap 95% interval
half-width no larger than 0.08 Fix Rate for the primary handoff contrast.
Scenario and repetition are resampled hierarchically. With fewer than twelve
families, results are explicitly developmental regardless of the number of
within-scenario repetitions.

Report:

- paired mean and median;
- hierarchical percentile interval;
- per-family effect;
- sign consistency;
- intention-to-treat and treatment-received estimates;
- availability and retry rates separately from quality.

No asymptotic p-value is used for the small clustered suite.

## 12. Free-route and retry policy

All live model calls must use the authenticated Clark catalog mapping:

- tier `clark-code`;
- option `free`;
- label `DeepSeek V4 Flash Latest`;
- response model DeepSeek V4 Flash Latest.

Any non-Free or non-DeepSeek response fails closed. There is no fallback.

Retryable:

- 429;
- 502, 503, or 504 before usable model output;
- equivalent typed capacity error.

Not retryable:

- authentication or route mismatch;
- invalid request;
- tool-contract failure;
- context overflow;
- semantic/model-quality failure;
- any executor attempt that produced usable output and mutated the workspace.

Route probe delays: 15, 30, 60, 120, and 300 seconds.

Planner/executor phase delays: 60 and 300 seconds, with `Retry-After` honored
and capped at 300 seconds. Progress is emitted at least every 30 seconds.

A typed executor retry uses a clean seed plus the frozen proposal restored
through `ResumeItem::ProposedPlan` and a real Fresh approval transition.

## 13. Capacity and cost safety

- Live mode remains explicitly environment-gated.
- A maximum live-case count is required.
- Route verification precedes every resumable run.
- Append-only records are flushed after every completed case.
- Three repeated deterministic failures stop the affected branch.
- Pilot defects stop confirmatory work immediately.
- Provider-reported upstream cost is retained even when the product tier is
  Free; no monetary-zero claim is inferred from the route label alone.

## 14. Deterministic gate before live work

All must pass:

1. baseline fixtures fail meaningful hidden checks;
2. two correct implementations pass;
3. semantic oracle plans score 1.0;
4. semantic mutations fail their mapped checks;
5. real Current, real Fresh, typed replay, and Markdown arms serialize
   distinct handoff receipts;
6. typed execution observes the exact proposal ID/revision/hash;
7. Plan Mode produces no workspace mutation;
8. production Org, repository, and Scout clients parse gateway responses;
9. Scout contains stable entities, edges, claims, and coverage;
10. full trajectories retain tool inputs, results, checklist revisions, and
    terminal status;
11. retry tests prove clean typed replay and five-minute wait caps;
12. the 456-row, 38-lane offline matrix and its 48 immutable plan-bank entries
    are deterministic and resumable;
13. tests and warning-free Clippy pass;
14. any generated-plan handoff comparison proves both executor arms consumed
    the same plan-bank ID, revision, and source hash;
15. the delivery-mechanism bank keys remain distinct, prefetched entries retain
    no Project Memory files, and every bank-backed case exactly matches its
    frozen task prompt and planning contract;
16. lifecycle probes reproduce and assert the exact next-request evidence for
    stale revision approval, feedback loss, resume, compaction, long-plan
    truncation, generic-mode bypass, duplicate decisions, and non-terminal
    proposal emission, plus a delayed conflicting decision reopening approval,
    approval before planner termination, approval-event deferral, hidden
    read-authorization leakage across Fresh, unplanned writes, completion with
    missing obligations, checklist substitution, and approval after workspace
    drift; a standalone artifact manifest maps every finding to its probe.

Live v3 trials remain paused until this gate is satisfied and the operator
explicitly resumes them.
