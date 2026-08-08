# Multi-repository orchestration benchmark

This benchmark is an executable acceptance contract for multi-writer orchestration. It is deliberately red for the foundation's current single-writer architecture. A root agent that directly edits every repository can make the visible tests pass, but it still fails if it cannot prove isolation, pinned baselines, replayable handoffs, targeted recovery, and fresh integration.

The existing [`orchestration_benchmark`](../orchestration_benchmark/README.md) measures single-repository reader/writer orchestration and the default-agent lifecycle. This suite adds the missing multi-repository control plane.

## What is actually graded

Each run creates several independent synthetic Git repositories. Solutions and hidden checks never appear in the public task manifest. The grader observes the real filesystem and Git state rather than trusting an agent's final message.

Hard gates cover:

- behavioral correctness across repository boundaries;
- an explicit repository/contract graph;
- exact baseline SHA pinning;
- one isolated, content-addressed patch package per writer;
- bounded parallelism only for independent work;
- compatibility decisions for every producer/consumer edge;
- independent application of every patch to fresh clones;
- result-tree digest verification after replay;
- preservation of pre-existing dirty user files and branch heads;
- targeted retry after a child crash or stale baseline;
- cheap-reader/strong-writer model routing;
- independent reviewer receipts in review lanes;
- brokered cloud routing for cloud-only repositories;
- trigger discipline, including a scenario where delegation is harmful;
- token budget, useful-token, duplicate-read, cost, wall-time, and aggregate agent-time accounting.

Passing visible tests without replayable packages is a failure. Passing after overwriting a user's dirty file is a failure. Restarting the entire tree after one child fails is a failure. Claiming parallelism with non-overlapping writer intervals is a failure.

## Scenarios

| Scenario | Repositories | Failure pressure |
| --- | --- | --- |
| `api-sdk-web` | API, SDK, web | contract propagation and end-to-end behavior |
| `rolling-event-compatibility` | producer, old consumer, new consumer | rolling compatibility; old consumer must survive |
| `generated-client-staleness` | service, generated client | schema and generated artifact must remain synchronized |
| `targeted-child-recovery` | worker, CLI | writer crash, artifact retry, dirty user notes |
| `cloud-local-auth-rollout` | auth API, mobile, infra | cloud-only repository assignment |
| `sequential-dependency-chain` | core, app | anti-case: multi-agent lane must decline delegation |
| `baseline-drift-replan` | library, service | reject stale work and replan only one repository |

## A/B lanes

The `single` and `equal-budget-single` controls distinguish orchestration benefit from simply spending more tokens. Multi-agent lanes compare cheap readers, all-strong workers, diverse review, and a local/cloud mix at the same 400k tree budget.

The report computes:

- behavioral, replay, conformance, and hard-gated pass rates;
- verified successes per 100k tokens;
- useful-token and duplicate-read ratios;
- cost and wall-time deltas against the equal-budget single-agent control;
- a conservative value gate: at least +10 percentage points in pass rate, positive correctness-adjusted yield, no more than 1.25x the control tokens, and at least three external repetitions.

Scripted runs can validate benchmark mechanics and expose missing capabilities. They are never allowed to make a model-quality or “more tokens help” claim. Only an external candidate with measured usage can cross the value gate.

## Commands

List scenarios and lanes without model calls:

```bash
cargo run -p provider-local --example multi_repo_orchestration_benchmark -- --list
```

Capture the foundation's expected-red baseline:

```bash
cargo run -p provider-local --example multi_repo_orchestration_benchmark -- \
  --candidate current-agent \
  --allow-red \
  --out target/multi-repo-benchmark/current-agent
```

Omit `--allow-red` in CI. The same command then exits non-zero until the required features exist.

Prove that the fixtures and rubric are solvable with the deterministic reference adapter:

```bash
cargo run -p provider-local --example multi_repo_orchestration_benchmark -- \
  --candidate reference \
  --out target/multi-repo-benchmark/reference
```

Evaluate a real orchestrator through the versioned JSON protocol:

```bash
cargo run -p provider-local --example multi_repo_orchestration_benchmark -- \
  --candidate external \
  --candidate-command-json '["./your-candidate","--benchmark-worker"]' \
  --candidate-timeout-seconds 900 \
  --repetitions 3 \
  --out target/multi-repo-benchmark/external
```

The external program receives one `CandidateRequest` JSON object on stdin. The public task omits fixture solutions, hidden checks, and the expected delegation decision. A separate adapter-level `control` field identifies a fault the wrapper must inject below the manager-agent boundary. The program may write `CandidateResult` JSON to `result_path` or stdout. Commands are passed as a JSON array and executed directly; the benchmark does not invoke a shell. Runs are terminated after the configured timeout.

On macOS, external candidates run under `sandbox-exec`: reads, processes, and network access remain available, but filesystem writes are restricted to that synthetic run directory. `HOME` and `TMPDIR` are also placed inside it. Other platforms fail closed because this repository does not yet have an equivalent write sandbox. `--unsafe-external` is an explicit escape hatch intended only for disposable CI machines.

No API key is read and no provider is called by the built-in `current-agent` or `reference` adapters. A real model run happens only through an explicitly supplied external command.

## Artifacts

Every invocation retains:

- `results.jsonl`: full run records, checks, receipts, usage, and hard failures;
- `summary.json`: lane aggregates and equal-token comparisons;
- `report.md`: human-readable results and exact failing gates;
- `runs/<id>/task-manifest.json`: the public task shown to the candidate;
- `runs/<id>/candidate-result.json`: the candidate's claim and receipts;
- `runs/<id>/workspace/repos/*`: the resulting independent repositories;
- `runs/<id>/artifacts/*.patch`: content-addressed writer handoffs;
- `runs/<id>/fresh-replay/repos/*`: independent integration evidence.

The reference adapter is an oracle for lifecycle mechanics, not a coding-model baseline: it is given fixture solutions internally so that a reference failure means the benchmark itself is broken. The current foundation adapter is intentionally given enough fixture knowledge to make visible behavior pass; its failure therefore isolates missing orchestration guarantees rather than code-generation quality.
