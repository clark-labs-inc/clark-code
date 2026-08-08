# Universal work-graph orchestration benchmark

This suite is an executable, failure-seeking contract for dependency-aware coding orchestration. It stays generic: a task may need a compiler, database, generator, build cache, test service, remote compute worker, or another repository, but the scheduler sees only tasks, resources, immutable artifacts, decisions, and verification.

It complements the multi-repository benchmark. The existing suite proves Git isolation and replayable cross-repository patches. This suite proves the missing lifecycle control plane: an environment can prepare while code work proceeds, a dependent agent wakes only when the environment is healthy, completed artifacts survive targeted recovery, and pure waiting consumes no model tokens.

## Target lifecycle contract

The target contract keeps the multi-agent lifecycle behaviors that transfer cleanly:

- one host-owned registry and shared concurrency/budget ceiling;
- explicit task identities, parent/child lifecycle, cancellation, and completion delivery;
- selected execution environments inherited by workers rather than rediscovered;
- bounded depth and concurrency;
- different worker roles and model tiers;
- terminal and process ownership scoped to the owning task.

The benchmark adds what a repository-level orchestrator still needs above those primitives:

- typed dependency edges instead of prose handoffs;
- mutable resources represented by safe lease identifiers, never raw process handles;
- immutable content-addressed artifacts with input and Git-baseline lineage;
- host-side readiness, health, expiry, and cleanup;
- dependency wakeups instead of manager-agent polling;
- stale-artifact invalidation and targeted recovery;
- independent fresh verification before completion;
- a simple default experience that hides models, agents, workspaces, leases, and graph topology.

## Two independent proof boundaries

Every built-in run creates real synthetic Git repositories. Hidden file checks, branch-head checks, allowed write scopes, and preservation of dirty user files establish behavioral correctness. The lifecycle grader separately inspects task, resource, artifact, wakeup, recovery, verification, safety, usage, and interaction receipts.

Editing the correct files without authoritative lifecycle evidence fails. Producing a perfect trace without fixing the repositories also fails.

The deterministic reference candidate is an oracle for benchmark mechanics. It receives hidden solutions internally and emits host-simulation receipts. It is not evidence of model quality or production autonomy. `current-agent` is deliberately allowed to make the hidden code changes, then remains red because the foundation does not yet expose the required production work-graph trace.

## Generic scenario catalog

| Scenario | Dependency pressure |
| --- | --- |
| `toolchain-bootstrap-fix` | long toolchain preparation overlaps diagnosis and gates implementation |
| `generated-contract-pipeline` | contract change produces a generated artifact consumed by another project |
| `service-migration-health` | database readiness, migration artifact, service update, health verification |
| `reusable-build-cache` | two independent writers reuse one prepared capability |
| `remote-compute-integration` | remote validation and local implementation converge at integration |
| `targeted-resource-recovery` | failed environment restarts without discarding good code artifacts |
| `resource-lease-expiry` | expired capability is renewed while completed diagnosis is preserved |
| `targeted-worker-recovery` | one failed writer retries without restarting its independent sibling |
| `baseline-drift-invalidation` | obsolete artifact is rejected and only dependent work is redone |
| `large-parallel-feature-recovery` | eight writers across four repositories, two shared environments, a four-task ceiling, targeted writer retry, and fresh final integration |
| `sequential-small-fix` | anti-case: delegation is more dangerous than useful |

No fixture assumes Android, iOS, web, or a particular build system. Domain-specific suites can later compile their setup into the same resource and artifact contracts.

The large case is the primary scale gate. It requires real width rather than two-file theater: four initially runnable writers must fill but never exceed the lane ceiling, downstream writers consume exact predecessor artifacts, environment readiness overlaps useful work, and an injected failure retries only its owner while seven completed artifacts survive. The serialized control and graph candidate use the same synthetic repositories and task estimates so wall-time gains cannot come from silently deleting scope.

## A/B lanes and value gate

The suite includes:

- `single`: ordinary one-agent control;
- `equal-budget-single`: the single agent receives the same tree-level budget as multi-agent lanes;
- `naive-parallel`: negative control with eager spawning, shared state, polling, and duplicate setup;
- `work-graph-strong`: strong model at every agent node;
- `work-graph-cheap-support`: cheap inspection/provisioning agents and strong writers;
- `work-graph-diverse-review`: cheap support plus an independent reviewer;
- `work-graph-cloud`: the same graph with cloud-eligible work routed through a cloud harness.

Reports include hidden correctness, lifecycle conformance, wall time, aggregate agent time, tokens, cost, polling tokens, duplicate setup tokens, and verified successes per 100k tokens. Work-graph lanes are compared with `equal-budget-single`, which has the same token ceiling; the report exposes actual token and cost ratios rather than assuming the candidate spent its allowance.

A value claim requires at least three paired external repetitions, production-host trace identities, at least +10 percentage points in verified pass rate, better lifecycle conformance and verified yield, no more than 1.25x tokens or cost, at least 10% lower wall time, and zero model polling tokens. Simulation can never unlock the value claim.

## Commands

List the catalog without reading an API key or calling a model:

```bash
cargo run -p provider-local --example work_graph_orchestration_benchmark -- --list
```

Prove that the fixtures and rubric are solvable:

```bash
cargo run -p provider-local --example work_graph_orchestration_benchmark -- \
  --candidate reference \
  --out target/work-graph-benchmark/reference
```

The intentionally chaotic `naive-parallel` lane is a negative control and is expected to fail. The command exits non-zero only when a required `work-graph-*` lane fails.

Capture the foundation's expected-red baseline:

```bash
cargo run -p provider-local --example work_graph_orchestration_benchmark -- \
  --candidate current-agent \
  --allow-red \
  --out target/work-graph-benchmark/current-agent
```

Omit `--allow-red` in CI once a production adapter exists. The gate will stay red until the adapter provides authoritative production traces and the default user flow.

Evaluate a real orchestrator through the versioned JSON protocol:

```bash
cargo run -p provider-local --example work_graph_orchestration_benchmark -- \
  --candidate external \
  --candidate-command-json '["./candidate-adapter","--benchmark-worker"]' \
  --repetitions 3 \
  --out target/work-graph-benchmark/external
```

The external process receives one `CandidateRequest` on stdin and writes `CandidateResult` to `result_path` or stdout. The public task includes repository paths and baselines but excludes hidden solutions, the oracle task graph, the expected delegation decision, and the injected fault. Fault injection is passed separately in `control` so a trusted adapter can apply it below the manager-agent boundary.

On macOS the process is write-sandboxed to its synthetic run directory. Other platforms fail closed unless `--unsafe-external` is explicitly used on a disposable machine. Built-in reference and current-agent runs never load `.env`, read an API key, or call a provider.

## Retained evidence

Each invocation writes:

- `results.jsonl`: complete graded run records;
- `summary.json`: aggregates and equal-token comparisons;
- `report.md`: readable results and exact hard failures;
- `runs/<id>/task-manifest.json`: public candidate input;
- `runs/<id>/candidate-result.json`: lifecycle claims and receipts;
- `runs/<id>/workspace/repos/*`: real synthetic Git repositories and resulting changes.
