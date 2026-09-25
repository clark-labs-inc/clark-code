# Security exploration tree: source review and proposed design

Status: implemented locally, 2026-09-25. Deterministic validation is recorded in
EVALS.md. No live scan or hosted-model quality evaluation was run.

## Implemented ownership

- `crates/research-tree`: Apache-2.0 pure reusable library; typed nodes, lineage,
  crosslinks, prerequisites, deterministic autoresearch selection, bounded attempts,
  explicit outcomes, reassessment/reopening and replay. No I/O, provider or desktop
  dependency. Cargo packaging independently builds the published dependency graph.
- `crates/provider-local/src/tools/research_tree`: optional ordinary local tools
  `research_tree` and `research_tree_status`, native artifact storage, source-file
  fingerprints, permission classification, stale-write rejection and compact
  selected-branch context.
- Security skills: use a tree for broad investigations, ordinary tools for the
  experiments, and the existing scan contracts for validated findings.

The library reuses autoresearch ranking but deliberately owns a separate work-state
graph: autoresearch's ExperimentGraph models optimization/commit state, rather
than evidence-work dependencies and terminal refutations. This is the narrower
reusable abstraction identified while implementing the design below.

Artifacts live at `.agent/research-trees/<tree_id>.json` beneath the current
task scope (or workspace root), with a separate lock file. The JSON contains a
versioned event journal, objective/scope, source bindings and fixed limits.
Atomic replacement, process file locking and expected revisions prevent partial
writes and competing overwrites. Replay validates every transition; serialized
state grants no permissions and cannot mint a scan receipt.

Optional source bindings hash up to 32 relative files / 16 MiB. Only those files
have a freshness guarantee; empty bindings are explicitly unbound. Changed or
missing source files block updates but old status remains readable. Begin a new
tree after changing the premise/source. Current execution permissions always apply.

The tools cap request size at 64 KiB and journals at 4 MiB, with headroom reserved
for active outcomes. The library caps nodes at 256, selections at 1000 and events
at 4096. Selection budgets do not limit actual tool calls, wall time or tokens.
Interrupted active work survives restart; record a truthful blocked outcome
before a justified retry, preserving consumed steps. Terminal nodes do not
automatically reenter the frontier.

Journal files, locks and atomic-save temporary files are excluded from standard
and diff scan source fingerprints, including nested task scopes. Actual source
edits still invalidate both fingerprints.

## Tool usage

Discover both tools through tool_search. Begin with:
```json
{"action":"begin","tree_id":"ownership","objective":"Check export ownership","scope":"Local export implementation only","sources":["src/export.rs"],"max_nodes":32,"max_steps":64}
```

Use `action: "apply"`, the exact returned `expected_revision`, and one typed
event: proposed, recorded, reassessed, reopened or pruned. A proposed event carries
key, parent, links, dependencies, evidence_refs, rationale and assessment; the
assessment uses the existing research_rank fields. Use `action: "select"` to
activate the ranked branch. Caller-supplied Started events are refused. Selected
context includes up to 16 relevant nodes; status pages 32 nodes and 16 frontier
hints. All evidence and outcomes remain caller assertions.

Read `research_tree_status({"tree_id":"ownership"})` before resuming and after an
uncertain write. It never rewrites the journal. A null selected node, empty
frontier or exhausted budget never means the investigated system is secure.


## What exists

Before this change, the Security assistant used normal tools and optional stateless research_rank
(crates/provider-local/src/tools/research.rs). The caller supplies every
opportunity and score on each call. There is no persistent frontier, parent-child
lineage, claimed node, or branch outcome in that tool. Explicit deep scans have
a separate accepted-pass ledger and saturation contract
(crates/provider-local/src/security_deep.rs); those are not an exploration tree.

Boss has three relevant layers:

- ../boss/crates/boss/src/graph.rs: typed targets, endpoints, hypotheses,
  findings and evidence; discovery/validation/refutation edges; a priority queue,
  visited nodes and serialization with queue reconstruction.
- ../boss/crates/boss/src/research.rs: derives opportunities from pending nodes,
  discounts repeated endpoint families and ranks the frontier through autoresearch.
- ../boss/crates/boss/src/runner.rs: selects the ranked node, builds context,
  executes an agent, absorbs observations and expands children. The planner
  cannot replace the deterministically selected node. Empty-frontier generation
  and target reseeding can continue exploration.
- ../boss/crates/boss/src/research_ledger.rs: persists observations, hypotheses
  and results, then supplies a compact context dossier between cycles.

This is best-first graph exploration, not evidence of an implemented MCTS
visit/reward/backpropagation policy. A tree is a useful navigation view, while
supporting/refuting evidence naturally links across branches.

The already pinned published clark-autoresearch 0.2.0 exports ExperimentGraph,
ResearchLedger, ResearchMode, ResultVerdict and rank_opportunities. Its graph
already has parent/children, lifecycle states and pruning. Reuse these primitives;
Boss's vendored copy declares 0.1.0 and should not replace our dependency.

## Design rationale

Keep Security a free agent. Activate structured exploration for a broad
investigation, not for every explanation or code fix.

The root is the user's question and authorized resources, which may be source
files, a repository, a document, or an authorized service. It has no required
domain or organization. Each branch represents a falsifiable question; children
are narrower hypotheses or discriminating experiments.

Example:

    Can one tenant read another tenant's exports?
      Identify export entrypoints and ownership controls
        Test job creation with another tenant's object
          Positive control: same-tenant object
          Negative control: different-tenant object
        Test download authorization independently
          Trace object lookup to response
          Check expired/revoked access
      Check alternative paths to the same object

Expose a small ordinary research tool surface: begin/status, expand, select,
record outcome, and prune. Use existing graph/ledger types where they fit; add
only the host-owned execution and evidence bindings they lack.

1. Begin a task-local tree with fixed scope and bounded work budget.
2. Expand with stable semantic identities and evidence references; deduplicate.
3. Filter scope, dependencies and terminal nodes before ranking. Pick pending
   work with autoresearch, retaining a reason for the decision.
4. Investigate one bounded branch through the normal Clark provider/tool loop.
   Parallel read-only work is optional under existing delegation permissions.
5. Record tool receipt references, observations, counterevidence and an explicit
   result: supports, refutes, expands, inconclusive or blocked.
6. Add genuinely new children and rerank. Reopening a closed branch requires new
   evidence or a changed premise.
7. Stop on answered objective, cancellation, budget limit, or exhausted useful
   work. Report unresolved branches; exhaustion is not proof of security.

Persist an event journal and rebuildable snapshot in task-owned local artifacts,
using the existing sandbox and artifact boundaries. Bind the tree to its source
snapshot and permission scope. Resume must retain pruned/refuted/blocked states,
budget accounting and receipts. Cancellation/failure must not become success.
The normal conversation can show a compact tree and evidence links; no separate
web dashboard, database service or organization registry is needed.

For explicit standard/deep scans, the tree chooses investigation work only.
Coverage, accepted independent passes, PoC controls and finalization stay owned
by existing scan contracts. A high score or supported hypothesis cannot issue a
verified finding; existing host-validated receipts remain authoritative.

## What to improve instead of copying

Boss's terminal decisions include English-string matching, its target seed
adds www/non-www variants, and its runner can reopen exhausted seeds. Carry
explicit scope and typed outcome/reopen reasons instead. Its add_children skips
an already-known child before adding the edge, so a reused node can lose a new
discovery relationship: preserve cross-links separately from deduplication.

Boss's ledger records results separately from hypothesis state; recording text
alone is not a complete state-transition contract. Define host-checked transitions
and link results to actual execution receipts. Avoid treating model JSON,
confidence estimates, or graph visits as verified observations.

## Implementation acceptance

The deterministic acceptance suite covers:
- expansion preserves lineage and shared evidence links;
- refuted/pruned/out-of-scope nodes cannot be selected;
- repeated proposals do not grow duplicate branches;
- resume preserves outcomes, source identity, permissions and spent budget;
- failure/cancellation are not recorded as successful verification;
- ranked selection remains deterministic for equal inputs;
- a scripted provider can branch, refute, resume, and complete through real tools;
- explicit scan finalization rejects unsupported proof despite tree outcomes.

A later authorized comparison can measure validated findings per unit cost,
duplicate probes, coverage and unresolved work against today's agent. Source
review and deterministic tests alone cannot establish improved detection quality.
