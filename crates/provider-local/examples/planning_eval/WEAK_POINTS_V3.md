# Why an approved Clark plan may not govern execution

Status: repository-grounded pre-repair investigation. The findings below are
retained as the audit baseline.

## Post-audit repair status

The typed execution-contract repair closes four boundaries from this baseline:

- `propose_plan` now emits ordered global invariants and typed execution steps
  before its Markdown rendering; the host assigns stable `step-N` identities;
- resume, compaction, later user turns, periodic model turns, and step
  transitions reinject bounded structured developer reminders;
- `update_plan` must preserve every approved step ID and title exactly once;
- an unresolved approved contract reopens generation instead of accepting the
  first terminal prose answer.

The remaining findings are emitted by `lifecycle::findings()`. The deterministic
suite retains explicit control-mode probes for comparison, while production
defaults the reminder/enforcement treatment on.

This note distinguishes planner quality from plan delivery and enforcement.
The current product has a real typed proposal and approval transition, but the
approved plan becomes a developer instruction. It is not an executable
contract. A run can therefore receive the exact approved text and still ignore,
truncate, forget, reinterpret, or silently diverge from it.

## Priority map

The weak points fall into five different failure classes and should not be
collapsed into one “the model ignored the plan” metric:

1. **Authority identity is unsound:** proposal emission is non-terminal;
   approval lacks revision/hash; generic mode switching bypasses approval;
   decisions are not state-checked or idempotent; approval acknowledgement is
   not durably atomic.
2. **Transport is lossy:** Fresh drops task/research, long plans are truncated,
   resume and compaction can remove authority, and Current/Fresh preserve
   different hidden state.
3. **Enforcement is absent:** Markdown has no behavioral schema, writes and
   shell calls and delegated work are not plan-bound, the checklist is
   independent, deviations do not require review, and completion is not
   reconciled to approved behavior.
4. **Truth can drift:** approval is not bound to a repository or knowledge
   snapshot, and retrieval availability does not prove treatment or temporal
   resolution.
5. **Evaluation can confound causes:** Markdown pasting, same-session source
   leakage, independently sampled plans, keyword grading, oracle application,
   retrying mutated attempts, and toy fixtures can all manufacture an apparent
   plan benefit or failure.

The first repair boundary is authority identity and durability. Stronger model
prompts cannot compensate for approving the wrong bytes or losing the approval
transition. The second is durable transport. Behavioral enforcement and model
quality evaluation become interpretable only after those two are proven.

## Current authority chain

The product currently turns a reviewed plan into execution through this chain:

```text
displayed ProposedPlan(id, revision, full Markdown, awaiting)
  -> PlanDecision(plan_id only)
  -> mutate in-memory current proposal to approved + switch mode
  -> consume one-shot exited flag on the next prompt
  -> render at most 6,000 characters as a developer instruction
  -> model invents an independent execution checklist and tool trajectory
  -> normal permissions/sandbox govern writes
  -> run/goal completion decides when work is done
```

The plan can stop governing execution at every arrow:

- reviewed revision is not the decision identity;
- approval is not durably acknowledged;
- the next prompt may be delayed, duplicated, interrupted, resumed, or
  compacted;
- delivered bytes may differ from reviewed bytes;
- checklist steps, delegated tasks, writes, and tests do not reference the
  approved revision;
- completion does not reconcile planned obligations with observed behavior.

That is why a single final-score comparison cannot answer “did the executor
respect the plan?” The first divergent boundary has to be identified.

## Evidence level

- **Deterministically reproduced on the real provider boundary:** feedback
  loss, stale-revision approval, resume authority loss, compaction authority
  loss, mode-toggle bypass, middle truncation, duplicate Fresh approval,
  conflicting delayed decisions, non-terminal proposal emission, approval
  before planner termination, approval-event deferral, Fresh read-authorization
  leakage, unplanned writes, false successful completion, checklist
  substitution, and approval after workspace drift.
- **Structurally proven from current contracts:** free-form plan semantics,
  missing evidence-snapshot identity, optimistic desktop mode projection, and
  lossy delegation.
- **Not yet measured as model behavior:** the frequency and user impact of
  these failures under repeated Qwen planning/execution. Live work remains
  paused, so no offline result is presented as that measurement.

## 1. Approval identifies a plan, not the displayed revision

`PlanningState::next_proposal` preserves the plan ID across revisions and only
increments `revision`. `ClientResponse::PlanDecision` carries `plan_id` but no
revision or content hash. The response handler matches only the ID and approves
the provider's current proposal.

Consequence: a stale UI action for revision 1 can approve revision 2 because
both share the same ID. There is no compare-and-swap check proving that the
approved bytes are the bytes the user reviewed.

Required probe: display revision 1, create revision 2 before the delayed
approval arrives, then retain displayed ID/revision/hash and actual approved
ID/revision/hash.

## 2. Continue-planning feedback is not applied by the decision response

The provider destructures `PlanDecision::ContinuePlanning { .. }` and changes
the collaboration mode, but does not append the decision's feedback to the
model transcript. The existing end-to-end test sends a separate user prompt
after the typed decision, which hides this boundary from hosts that expect the
feedback field itself to reach the planner.

Consequence: requested corrections can disappear, while the next proposal
looks like a legitimate new revision.

Required probe: send feedback only in the typed decision, then verify whether
the next model request contains it. Compare with a separate user-message arm.

## 3. Fresh execution deliberately destroys everything except the plan

`PlanDecision::Implement(Fresh)` clears the transcript and retains
`PlanningState::proposed_plan`. The next app turn says only “Implement the
approved plan.” The developer instruction carries the proposal, but the
original task, research trail, tool results, negative evidence, user answers,
and unresolved uncertainty are gone unless the planner compressed them into
the proposal.

Consequence: Fresh is a lossy information bottleneck, not merely a clean
executor. A concise plan can be correct yet omit a constraint the planner
assumed remained visible.

Required probes:

- exact same proposal under Current and Fresh;
- Fresh plus task replay;
- Fresh plus provenance-preserving evidence capsule;
- Fresh with an oracle plan that explicitly contains every hidden behavior.

## 4. The plan is injected once and durable replay can forget its authority

The transition from Plan to Default sets the one-shot `PlanningState::exited`
flag. The next prompt consumes that flag and injects the approved-plan
developer instruction. In-memory transcripts may retain that custom developer
message during the session, but durable transcript conversion drops custom
messages. A resumed session restores the typed `ProposedPlan`; if it starts in
Default mode with `exited == false`, the approved plan is not reinjected.

Consequence: interruption, restart, handoff, or certain compaction/resume paths
can preserve the plan object for UI state while removing the instruction that
tells the executor to follow it.

Required probes:

- approve, execute one partial turn, serialize, restart, and continue;
- interrupt immediately after approval but before the first execution prompt;
- compact during a long execution and verify the next wire request;
- compare same-session continuation with `ResumeItem::ProposedPlan` replay.

## 5. Long plans must remain byte-identical at delivery

Approved plans are now reinjected and resumed in full. The evaluator retains
the stored and delivered hashes and lengths and fails the fidelity claim if
they differ. Model-created proposals are rejected at their existing 12,000
character input boundary rather than accepted and silently rewritten later.

Required receipt: full proposal hash and length, delivered-plan hash and
length, and a false truncation flag for every typed handoff.

## 6. The plan has no machine-checkable behavioral schema

`ProposedPlan` is free-form Markdown plus ID, revision, and status. It has no
stable step IDs, dependency graph, required files, invariants, evidence links,
verification obligations, or allowed deviations.

Consequence: the runtime cannot distinguish a stylistic rewrite from a missing
compatibility boundary, and cannot align tool calls or test outcomes to the
approved decisions.

Required evaluator representation: scenario-owned semantic behavior IDs mapped
to both plan checks and executable hidden checks. Lexical path and evidence
coverage remain diagnostic only.

## 7. The execution checklist is a separate, advisory plan

`update_plan` creates `PlanningState::execution_checklist`; it is explicitly
separate from `proposed_plan`. The model can omit it, rewrite its steps, or
track a different decomposition. There is no required mapping from approved
proposal steps to checklist steps. Standing-goal completion can also mark all
checklist items complete even when the model omitted the redundant final
update.

Consequence: a green checklist does not prove adherence. It can conceal skipped
approved work or unapproved design decisions.

Required receipts: approved-plan semantic IDs, every checklist revision,
step-similarity mapping, omitted approved steps, and newly introduced semantic
decisions.

## 8. Tool execution is not bound to the approved plan

After approval, write and shell tools are governed by normal sandbox and
permission policy. Tool calls do not carry plan ID/revision, planned step ID,
or a deviation reason. No guard rejects a write outside the plan or pauses for
replanning when repository evidence contradicts it.

Consequence: “plan respected” is entirely model obedience. The typed transition
proves authority and transport, not enforcement.

Required trajectory grading: identify the first tool call that contradicts or
leaves the approved semantic path, not merely the final failing test.

## 9. Completion is not checked against the approved plan

Run completion is based on the agent-loop outcome, goal state, and whatever
checks the model chose to run. `ProposedPlanStatus::Approved` has no executed,
superseded-during-execution, partially-complete, failed, or verified state.
There is no final comparison between the approved plan, checklist, changed
files, and test receipts.

Consequence: the run can declare success with planned behaviors missing, or
pass hidden behavior by an unplanned implementation. Final outcome alone
cannot diagnose whether planning helped.

Required classification per behavior:

- passed as planned;
- passed without plan;
- failed despite plan;
- failed without plan.

## 10. The planning prompt optimizes terseness against completeness

The concise profile asks for 3–7 terse steps, at most three sentences each,
after reading only a few relevant files. That is valuable for ordinary UI
plans, but it creates a structural compression pressure for changes spanning
many components, rollout phases, or knowledge sources. There is no semantic
self-check before `propose_plan`.

Consequence: a planner can follow the prompt perfectly and still emit a plan
too small to carry the task and evidence through Fresh execution.

Required experiment: vary topology and required semantic decisions while
holding model and evidence constant; measure plan length, semantic recall, and
Fresh-minus-Current outcome.

## 11. Knowledge retrieval is availability, not treatment receipt

Project Memory's catalog can be visible without a `memory(recall)` call. Org
and Scout tools can be offered but never discovered, called, or successfully
parsed. Returned evidence can be cited but not operationalized. Fresh execution
then loses the raw retrieval result unless it is represented in the plan or
separately reoffered.

Consequence: source-lane assignment alone cannot establish that Project, Org,
or Scout knowledge affected the plan.

Required treatment receipt:

- source and tool offered;
- discovery and activation;
- exact query/arguments;
- successful response and returned evidence IDs;
- temporal/provenance resolution;
- plan citation and corresponding semantic behavior;
- whether the executor independently received the evidence.

## 12. Current and Fresh have different failure mechanisms

Current preserves the task and research but can dilute the approved plan inside
a long transcript, retain stale hypotheses, and approach compaction limits.
Fresh emphasizes the plan but drops uncompressed context. Neither is an
unconditionally stronger treatment.

Consequence: combining them into one “real plan” result hides the mechanism.

Required analysis: report Current, Fresh, and typed replay separately, with
context tokens, compaction events, delivered-plan bytes, and first divergence.

## 13. Evaluator risks that can manufacture a plan benefit

The evaluator itself can overstate adherence if it:

- pastes Markdown into a user prompt instead of exercising `PlanDecision`;
- gives Markdown the full plan while the typed product truncates it;
- applies an oracle implementation in offline mode and calls that executor
  quality;
- scores keyword/path overlap as plan correctness;
- grades only final tests and misses unplanned success;
- assigns a source treatment without a successful retrieval receipt;
- labels retrieval as deferred, preactivated, or prefetched without proving
  first-request tool visibility and preventing cross-mechanism leakage;
- reuses related toy scenarios as if they were independent evidence;
- retries mutated or partially productive executor attempts;
- ignores restart, compaction, stale approval, and multi-turn execution.

The v2 live pilot had the first defect and remains diagnostic only. V3 now uses
separate immutable plan-bank keys and provider configurations for deferred
discovery, first-request tool preactivation, and host-prefetched capsules. The
prefetched arm retains no Project Memory and disables Scout retrieval; bank
records must exactly match their frozen task prompt and planning contract.
Live work still must not run until all deterministic gates cover the
product-real transition, oracle handoff, receipt fidelity, and high-complexity
scenario topology.

## 14. First-cause decision order

For every failed behavior, classify the earliest evidenced boundary:

1. required fact was unavailable;
2. retrieval tool was not discovered;
3. retrieval failed;
4. evidence was returned but ignored;
5. plan omitted or contradicted the behavior;
6. displayed and approved plan identity differed;
7. approved bytes were truncated or not delivered;
8. executor omitted or contradicted a correct delivered plan;
9. executor chose nondiscriminating verification;
10. fixture/grader was defective;
11. model capacity failed.

If receipts do not distinguish two causes, report `unresolved`; do not infer
planner failure from executor failure or vice versa.

The evaluator now emits this as a schema-v4 `causal_attribution` receipt for
each failed live behavior. It records a concrete cause only at a supported
boundary and otherwise retains the competing candidates plus the first
potentially mutating executor event. Offline oracle application is marked not
applicable rather than manufactured into causal evidence.

## 15. `propose_plan` is not a terminal or freezing operation

The tool description and result tell the model to end the turn, but the agent
loop does not stop on `ToolSignal::ProposedPlan`. The same model turn can call
`propose_plan` again, producing a new revision with the same ID after the first
revision was already rendered for review.

Consequence: “the plan shown to the user” is not an atomic handoff point.
Approval can race a still-running planner, and the ID-only decision contract
cannot prove which same-turn revision was reviewed. The desktop renders and
enables decision buttons as soon as the proposal event arrives; it does not
require the planner run to be terminal. If that run is still active, the
automatically generated “Implement the approved plan.” message is queued until
the run settles, so proposal state, collaboration mode, and the active planner
loop can interleave across approval.

Required product invariant: emitting a proposal must either terminate the
planner turn or create an immutable approval token containing plan ID,
revision, and content hash. The host must not enable approval until that
planner run reaches a terminal state.

## 16. The generic mode switch bypasses typed approval

`Provider::set_collaboration_mode(Default)` calls `PlanningState::set_mode`
directly. Leaving Plan Mode sets the one-shot `exited` flag even when the
current proposal is still `AwaitingDecision`. The next prompt then calls
`plan_mode_exit_note`, whose text says “Implement the approved plan” without
checking the proposal status. The desktop exposes this generic mode switch
through the collaboration pill as a separate path from the proposed-plan card.
It updates local collaboration state before the provider acknowledges the
change and does not roll that state back if the provider call fails.

Consequence: a mode toggle can promote unapproved plan bytes to developer
authority without a `PlanDecision::Implement` event. The audit trail then says
“approved” in the model instruction while typed state still says awaiting
decision. A failed provider transition can additionally leave the UI and
provider disagreeing about whether Plan Mode is active.

Required product invariant: leaving Plan Mode with a pending proposal must be
an explicit cancel/discard transition. Only a revision-bound typed decision may
create approved-plan execution authority.

## 17. Plan decisions are not state-checked or idempotent

The response handler accepts any matching plan ID regardless of current plan
status or collaboration mode. Replaying `Implement(Fresh)` after a successful
Current approval is accepted, clears the transcript, and does not queue a new
exit note because the session is already in Default mode.

Consequence: a network retry or duplicated host event can erase the task,
research, earlier execution, and approved-plan instruction while appearing to
repeat a successful action. A delayed `ContinuePlanning` can likewise move an
already-approved session back into Plan Mode.

Required product invariant: decisions must be compare-and-swap transitions
over `(plan_id, revision, hash, status)`, with an idempotency key and a durable
transition receipt. Exact duplicates should return the original outcome;
conflicting or stale transitions should fail closed.

## 18. Fresh context is not fresh tool state

`Implement(Fresh)` clears `SessionState::transcript`, but the provider's
session-scoped `ReadTracker` survives. Files read by the planner therefore
remain authorized for the read-before-edit guard even though the fresh executor
never received their contents. The same session also retains activated deferred
tools, the evidence-bearing workspace, Project Memory files, and the planner's
gateway configuration; other provider-scoped state such as background tasks,
permission policy, and repository identity likewise has no single explicit
Fresh reset contract.

Consequence: Fresh combines model amnesia with hidden operational memory. An
executor can make an immediate edit that would be rejected in a genuinely new
session. It can also independently re-retrieve planner-assigned Org, Scout, or
Project evidence, so a same-session `all - none` comparison can mistake
executor retrieval or leaked capability for plan quality/adherence.

Required product invariant: define Fresh as an explicit state-reset manifest.
Either clear read authorization and other epistemic state, or replay the exact
evidence that justifies retaining each capability and record it in the handoff.

## 19. Plans are not bound to workspace or evidence snapshots

`ProposedPlan` contains no repository baseline hash, Git head, dirty-tree
fingerprint, Project Memory revision, Org claim cut, Scout bitemporal snapshot,
or tool-schema revision. Concurrent agents and knowledge updates can change the
facts between proposal, approval, and execution without invalidating the plan.

Consequence: the executor may faithfully implement a plan that was correct for
a different tree or evidence cut. Conversely, necessary deviation after a
concurrent change looks like disobedience because there is no declared
staleness or replan transition.

Required product invariant: bind approval to immutable workspace and knowledge
receipts, check them before the first mutation, and require an explicit
supersede/replan or reviewed deviation when a relevant baseline changed.

## 20. Approval acknowledgement has a durability gap

`Provider::respond` mutates in-memory plan state and returns no event stream.
The desktop optimistically marks every matching plan ID approved and then sends
“Implement the approved plan.” The provider emits the approved
`ProposedPlanUpdated` event only as an initial event of that subsequent prompt.

Consequence: a crash or send failure between response and execution can leave
the UI, provider memory, and persisted event history with different approval
truth. On restart, the durable replay may contain only the awaiting proposal—or
an approved proposal without the one-shot authority transition.

Required product invariant: make approval itself a durable, revision-bound
event with an acknowledgement receipt before the UI changes state. Starting
execution should reference that durable transition, not create it implicitly.

## 21. Delegation creates a second lossy plan

The coding-delegation tool accepts model-authored `objective`, workstream
objectives, path leases, and integration checks. It builds a separate
`MultiRepoPlan`; the Clark `ProposedPlan` ID, revision, hash, semantic
obligations, and evidence receipts are not fields in that plan. Child writer
prompts contain the generated task objective and optional cross-repository
decisions, not the approved Clark plan. Read-only delegation similarly forwards
model-authored workstream objectives and acceptance criteria.

Consequence: the root executor can correctly receive an approved plan and then
compress or reinterpret it into a weaker child assignment. Path isolation,
patch hashing, review, and integration checks prove controlled execution of the
delegated task, but not fidelity to the user-approved plan. A successful child
or integration receipt can therefore look like plan adherence while omitting an
approved behavior before the child ever sees it.

Required product invariant: derive delegated tasks from a revision-bound
approved execution contract. Every child receipt should carry the parent plan
ID/revision/hash, mapped semantic obligations, evidence cut, and allowed paths;
integration must reject missing obligations and surface any child-introduced
decision as a reviewed deviation.

Required evaluator arm: authorize coding delegation for scenarios with
separable components, freeze the same approved plan, and compare single-agent
execution with delegation while grading obligation preservation at root-to-task,
task-to-patch, and patch-to-integration boundaries.

## 22. Deterministic lifecycle reproductions

The v3 evaluator now drives the real `LocalAgentProvider` through a fake
OpenAI-compatible stream and captures the exact subsequent wire request. Sixteen
offline probes reproduce the current behavior:

- typed `ContinuePlanning.feedback` is absent from the next planner request
  unless the host sends a separate user turn;
- delayed approval of revision 1 injects revision 2 because the shared ID is
  the only selector;
- an approved resumed `ProposedPlan` remains historical data but is not
  reinjected as developer authority;
- manual compaction can replace the approved instruction with an unconstrained
  summary and the next turn does not restore typed plan authority;
- the generic mode switch labels an `AwaitingDecision` proposal as approved in
  the next developer message;
- a decision placed in the omitted middle of a long proposal is absent from
  typed Fresh delivery;
- a duplicate Fresh approval clears context without reinjecting the plan;
- a delayed `ContinuePlanning` decision for that same ID reopens an already
  approved session;
- one model turn can emit two visible revisions because `propose_plan` does not
  terminate execution;
- approval is accepted while that same planner turn is still blocked in a
  later model request and has not emitted `RunFinished`;
- approval returns successfully without emitting a plan-state event, and its
  first `Approved` event is deferred until a later execution prompt starts;
- Fresh execution inherits the planner's hidden file-read authorization and
  can edit without rereading model-visible contents;
- the executor can successfully create a file that the approved plan
  explicitly forbids;
- execution can emit `RunFinished(Done)` while the plan's only required
  artifact is absent;
- an independently authored, unrelated checklist can be marked complete while
  approved work is absent and the run still finishes successfully; and
- repository contents can change after proposal emission without invalidating
  approval or triggering replanning.

These are authority, transport, enforcement, truth-drift, and lifecycle
findings, not Qwen quality findings. They exist before any live planner or
executor variance is introduced. Offline artifacts also emit
`lifecycle-findings.json`, a standalone mapping from each failure to the exact
deterministic provider test and the authority or state boundary it violates.
