# Agent orchestration benchmark

This example measures whether multi-agent coding improves repository-task reliability enough to justify its latency, token, cost, and coordination overhead. It deliberately separates two evidence levels:

- `scripted`: deterministic Provider/AgentEvent traffic that validates orchestration mechanics, safety invariants, grading, artifacts, and failure recovery without model calls.
- `live`: real `LocalAgentProvider` sessions against synthetic repositories. This is model-quality and runtime evidence and can spend credits.

Scripted success is never reported as model success. Live success is never accepted from a final message alone; the harness grades the retained repository independently.

## Lanes

Every lane emits the same `BenchmarkRecord` schema and is paired by scenario, variant, and repetition.

Every local or scripted attempt also emits the default `/root` execution lifecycle. The benchmark replays that trace independently and records root executions, attempts, recoveries, duplicate terminal tool receipts, and missing/invalid lifecycle traces. ACP readers remain an explicit exception because their harness does not expose the foundation lifecycle contract.

The summary also includes an eight-case default-agent recovery A/B: clean completion, safe transient and rate-limit recovery, active mutation, pending permission, exhausted budget, exhausted attempt count, and non-transient provider failure. It compares correctness, attempts, weighted tokens, cost, replay validity, and duplicate tool receipts against a one-attempt baseline.

| Lane | Shape | Purpose |
| --- | --- | --- |
| `single` | one strong writer | baseline |
| `planned-single` | one strong writer prompted to plan first | controls for planning without delegation |
| `reader-writer` | parallel readers, one strong writer | isolates decomposition/context benefit |
| `reviewed` | readers, writer, reviewer, rework, verifier | maximum reliability lane |
| `cheap-subagents` | cheap parallel readers, strong writer | cost-quality tradeoff |
| `homogeneous-strong` | strong readers and writer | strong-model ceiling |
| `brokered-cloud` | scripted research reader plus local writer | validates the host-injected boundary contract offline; live ownership stays in the product composition |
| `mixed-harness` | OS-sandboxed ACP readers plus local writer | compares an external coding harness at the same read-only boundary |

Only one task may hold the writer lease. Readers, reviewers, and verifiers have a read-only permission ceiling. Result reporting and acceptance are separate control-plane transitions so a reviewer can reject a reported writer attempt and accept a later rework attempt.

## Scenario catalog

The catalog uses freshly seeded Git repositories (plus a non-Git case), private deterministic rubrics, pre-existing dirty user changes where relevant, retained event streams, and a scratch `HOME` outside the checkout. It includes variants for:

- trivial and substantial multi-file changes;
- genuinely independent work and false parallelism;
- overlapping edits and stale reads;
- hidden cross-module contracts, decoys, and misleading docs;
- worker crash, missing/false/duplicate handoff, flaky verification, and reviewer bugs;
- permission escalation, tree-budget exhaustion, context truncation, and restart/resume;
- remote/non-Git execution and the scripted host-injected cloud-agent boundary.

The hidden rubric prefers behavioral checks when several implementations are valid. Exact-file checks are reserved for tasks whose requested output is exact.

## Safety gates

Offline is the default. Live mode requires all of the following:

1. `--live`;
2. `ORCHESTRATION_BENCH_LIVE=1`;
3. `MODEL_API_KEY` or `PRODUCT_API_KEY` (the runner loads the root `.env` without logging values);
4. a scenario and/or lane filter, unless `--full-live-matrix` is explicit.

Live defaults add further bounds: 600 seconds per agent, 400k tokens per orchestration tree, 12 benchmark runs per invocation, and a $2 inter-run cost stop. Each is configurable. Cost caps are checked between retained runs because providers report authoritative cost only after a run finishes. The neutral foundation rejects live `brokered-cloud` runs because it owns no product research tool; that lane must be run by the product composition that installs the brokered ToolPack. The normalized Provider usage also does not expose cache-hit tokens, so `non_cached_input_available` is false and the benchmark records zero rather than relabeling total input as non-cached input.

The live mixed-harness lane takes an explicit ACP command as a JSON string array. The benchmark wraps it in macOS `sandbox-exec` with filesystem writes denied, uses a scratch `HOME`, rejects permission requests, snapshots the repository around the parallel reader batch, and never routes the writer through ACP. It is deliberately unavailable on platforms where that boundary is not implemented.

Read-only sessions set `write_file`, `edit_file`, and `bash` to deny. Writer sessions allow those tools only inside the synthetic sandbox, deny destructive/network command prefixes, and still inherit provider safety classification. No session can commit, push, widen its role permission ceiling, or write outside its synthetic checkout without failing the rubric.

## Commands

List the catalog without model calls:

```bash
cargo run -p provider-local --example orchestration_benchmark -- --list
```

Run the complete deterministic matrix:

```bash
cargo run -p provider-local --example orchestration_benchmark -- \
  --out target/orchestration-benchmark/offline-full
```

Run a bounded live A/B comparison:

```bash
ORCHESTRATION_BENCH_LIVE=1 \
cargo run -p provider-local --example orchestration_benchmark -- \
  --live \
  --scenario independent-modules-1 \
  --lane single,cheap-subagents,reviewed \
  --max-live-runs 3 \
  --max-live-cost-usd 0.30 \
  --out target/orchestration-benchmark/live-ab
```

Compare a configured ACP harness against the local single-agent baseline:

```bash
ORCHESTRATION_BENCH_LIVE=1 \
cargo run -p provider-local --example orchestration_benchmark -- \
  --live \
  --scenario independent-modules-1 \
  --lane single,mixed-harness \
  --acp-model external-model-name \
  --acp-command-json '["your-acp-agent","--acp"]' \
  --max-live-runs 2 \
  --out target/orchestration-benchmark/live-mixed
```

## Artifacts and interpretation

An invocation retains:

- `results.jsonl`: one versioned record per scenario/lane/repetition;
- `summary.json` and `report.md`: lane aggregates, paired single-agent deltas, tail latency, pass/safety rates, cost, trigger quality, recovery, and Pareto membership;
- per-lane lifecycle totals: root executions/attempts/recoveries, trace replay failures, and duplicate tool receipts;
- `runs/<run-id>/repo`: resulting synthetic repository;
- `runs/<run-id>/attempts/*.events.json`: normalized raw Provider events;
- `runs/<run-id>/record.json`: self-contained run record;
- control-delivery and restart receipts for relevant scripted scenarios.

Reliability is lexicographic: a lane with any unauthorized/out-of-scope write, lost user change, permission widening, concurrent writer, missing causal trace, or accepted unverifiable result cannot be a ship candidate. Among safe lanes, completion and correctness dominate latency, token use, and cost. One repetition is a pilot, not a production go/no-go; tail and variance claims require repeated live variants.
