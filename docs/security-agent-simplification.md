# Security agent simplification

Security previously opened an organization-bound Insights workspace. The UI
loaded memberships, registered/synced the selected repository, queried posture,
findings, candidates, campaigns and scan history, and made Git checkout selection
a launch prerequisite. The product selected the full scan skill by default and
attached an organization Cloud Advisor recipe. Saved conversation metadata kept
those organization/repository/object bindings.

## New runtime contract

Security is an ordinary local/remote agent with the explicit `security:assistant`
skill and the product's existing model/access policy. It can explain, investigate,
edit and verify without a Git repository, cloud registration, or Insights. The
product keeps exact scan/diff/deep skills as optional requests. Those retain their
existing snapshot, coverage, isolated control and seal requirements.

The UI is chat-first with no organization selector or Insights canvas. Local scan
history remains artifact-backed. Opening that history never registers a target or
uploads findings. Ordinary conversation and artifact synchronization still uses
its existing account authorization.

`research_rank` reuses published `clark-autoresearch =0.2.0` opportunity ranking
through a deferred ordinary tool. It accepts at most 64 hypotheses/work items,
retains caller-provided evidence references and rationale, excludes caller-marked
out-of-scope work and returns prioritized probing/validation hints. It introduces
no server, provider call, persistent research database or scanner scheduler. Scores
are caller estimates, not proof or authorization; the agent investigates with its
existing tools and can rank again after observing results.

Broad investigations can also use `research_tree` and `research_tree_status`
for local persistent branches, evidence, refutations, deterministic next-work
selection and restart recovery. The portable `research-tree` crate owns pure
state transitions; the provider owns artifact I/O and current permissions.
See [security-tree-exploration.md](security-tree-exploration.md).

## Ownership and interfaces

- Foundation: UI, general Security skill, ordinary tool registry and optional
  Autoresearch ranking; canonical Security context retains only kind/workflow.
- Private desktop composition: default skill/catalog, existing paid/model policy,
  removal of automatic advisor and desktop Security cloud RPC implementations.
- Clark backend: account-owned Security conversation persistence and forward
  metadata migration; organization-free paid access including assigned seats.
  Scientist's organization and research contracts are preserved.

The web Security dashboard and its API clients are removed, along with the
organization scan backend and explicit CLI cloud-export machinery. Historical stored findings,
evidence vaults, and infrastructure have not been deleted. Implementation
validation used no paid model evaluations or credentialed local signing.
Deployment and desktop publication are separate release gates.

See EVALS.md for current validation and claim limits.
