# research-tree

A pure, reusable research work graph. It owns no provider, filesystem, network,
organization, scanner, or UI. The host supplies scope/source identity, persists
the journal, performs authorized work, and records observations.

`ResearchTree::new(identity, limits)` starts an empty graph. Apply a `Proposed`
event with a stable caller-defined semantic key. `ranked()` uses
`clark-autoresearch` 0.2.0's opportunity ranking over pending, in-scope work whose
explicit dependencies have completed or been refuted. Parent/crosslink edges
express provenance; only `dependencies` gate execution.

`select()` deterministically activates the first ranked item and journals a
`Started` event, charging one step. Concurrent active work is allowed. A caller
records `Supports`, `Refutes`, `Expands`, `Inconclusive`, or `Blocked` from an
active item. These are caller assertions, not validation certificates.
`Supports` requires evidence references. Supports/expands/inconclusive finish
that attempt; refutes and blocked retain distinct terminal states. Reopening a
terminal item requires at least one previously unseen evidence reference and a
rationale; references are opaque host-supplied strings, not verified files.

Repeated proposals with the same key merge evidence and crosslinks without
resetting terminal work. A different parent becomes an additional crosslink.
Changing assessment or dependencies in a duplicate proposal is rejected;
propose a distinct work item for a distinct contract. `Reassessed` updates a
pending item's estimated scores/validation focus only with new evidence and a
rationale; it cannot change declared scope. This lets later observations change
ranking without resetting completed work. Reference order is stable.
Pruning is allowed for pending/blocked work. Blocking an interrupted active item
uses the ordinary recorded `Blocked` outcome.

Persist `tree.journal()` (identity, limits, events), then reconstruct with
`ResearchTree::replay`. Do not trust serialized derived node snapshots. Replay
validates every transition, including deterministic selection order. Hosts must
check the identity against current scope/source before replaying or extending a
journal and enforce authorization independently of `in_scope`.

Limits are 1–256 nodes, 1–1000 attempts, 4096 journal events, and 64 references
per list. Keys, identity, evidence references, rationale, and finite normalized
scores are bounded and validated. Hosts should additionally bound serialized
input/output sizes. Every failed event is atomic. Journal slots are reserved for each active attempt's terminal observation. Exhausted budgets do not
silently reset or discard pending work. No hosted-model or scan-quality claim is
implied by this library's deterministic tests.

This deliberately uses autoresearch's ranking primitives rather than wrapping
`ExperimentGraph`: the latter models candidate optimization/commit state,
whereas this library schedules evidence-gathering work with shared provenance,
explicit dependency readiness, replay, and terminal-result deduplication.

The crate is Apache-2.0, with no platform dependencies or I/O APIs. Minimal host
usage (the host chooses actual hypotheses and collects actual evidence):

```rust
use research_tree::{Identity, Limits, ResearchTree};
let tree = ResearchTree::new(
    Identity { scope_id: "requested-repository".into(), source_id: "source-revision".into() },
    Limits { max_nodes: 32, max_steps: 64 },
)?;
let journal = tree.journal();
let resumed = ResearchTree::replay(journal.identity, journal.limits, journal.events)?;
# Ok::<(), research_tree::TreeError>(())
```

`context(key, limit)` returns the selected node first, its ancestor lineage next,
then related dependencies and crosslinks in deterministic breadth-first order.
It deduplicates shared nodes and bounds output to 1–32 nodes, so a selected
branch remains accessible even when the full graph exceeds a UI/page limit.
The host remains responsible for token/byte presentation budgets.
