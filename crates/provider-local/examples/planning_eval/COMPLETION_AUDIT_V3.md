# Planning evaluation v3 completion audit

Audit date: 2026-07-29

This audit evaluates the full requested outcome. It does not treat deterministic
offline execution as evidence about DeepSeek planner or executor quality.

## Requirement status

| Requirement | Status | Authoritative evidence | Remaining proof |
| --- | --- | --- | --- |
| Preregister scenario topology, temporal/provenance semantics, source factorials, planner/executor isolation, controls, grading, retention, power, retries, and cost safety before live work | Achieved | `PREREGISTRATION_V3.md` sections 2–14 | None for the design artifact |
| High-fidelity complex-project simulation | Achieved offline | 12 families, 7 domains, 60+ files per seed, 8–15 oracle changes, 25+ evidence items, five hidden behaviors, and two accepted implementation layouts; fixture mutation tests reject every missing oracle file | Live model interaction with these workspaces |
| Realistic Project Memory, Org Memory, and Scout/cartography boundaries | Implemented and deterministically validated | Real `.clark/memory` layout; production-shaped Org and Scout gateway contracts; deferred, preactivated, and prefetched delivery mechanisms; source-treatment receipts | Repeated live evidence that DeepSeek discovers, retrieves, interprets, and carries this information into execution |
| Full Project/Org/Scout source factorial and noise/stale/conflict controls | Achieved offline | Eight source combinations plus noisy, stale, conflicting, oracle, discarded, and executor-only controls in the 38-lane manifest | Repeated live outcomes |
| Product-real typed Current, Fresh, and replayed Fresh handoffs | Implemented and deterministically validated | Typed `PlanDecision` runners, immutable plan bank, exact handoff hashes, and lifecycle probes | Repeated live executor outcomes from frozen proposals |
| Hidden executable grading and plan-adherence grading | Achieved offline | Five semantic and hidden checks per scenario, mutation sensitivity tests, behavior-level adherence categories, and schema-v4 causal attribution | Live trajectories to populate causal attribution |
| First-cause attribution without forced inference | Implemented | Every failed live behavior can record a supported cause or `unresolved` candidate set plus its first potentially mutating event; offline oracle rows are marked not applicable | Live failures and successes |
| Exact receipt retention | Implemented; live values unexercised | Schema v4 retains route, prompts, contexts, handoff, memory/retrieval, trajectories, outputs, usage, retries, hashes, verification, and causal attribution | Authenticated live receipts |
| Bounded transient-capacity retries | Implemented and unit tested | Route delays 15/30/60/120/300 seconds; planner/executor delays 60/300 seconds; `Retry-After` capped at 300 seconds; clean typed replay; only 429/502/503/504 and recognized capacity failures retry | Actual Free-tier capacity behavior during authorized live runs |
| Fail closed on non-Free or non-DeepSeek routing | Implemented and unit tested | Live route requires `clark-code:free`, authenticated catalog markers `clark-code` / `free` / `DeepSeek V4 Flash Latest`, and effective DeepSeek V4 Flash Latest response; no fallback | Current authenticated catalog and response receipt |
| Offline route claims are truthful | Achieved | Offline route is `none`, verification method is `no model call`, and `free_tier_verified` is false | None |
| Statistical design and paired analysis | Implemented | Scenario-clustered hierarchical bootstrap, frozen-plan pairing, preregistered primary contrasts, and 23 generated paired-effect slots | At least three live repetitions per primary scenario/lane pair; five preferred |
| Repeated live DeepSeek V4 Flash Latest Free-tier trials | **Missing by explicit pause** | No qualifying live schema-v4 artifact exists | Explicit operator resumption, authenticated Free-tier route, bounded retry execution, and several completed repetitions |
| Evidence-backed variance and paired-effect conclusions | **Missing** | Offline output is harness validation only | Completed repeated live matrix |

## Latest deterministic receipt

Artifact:
`target/planning-eval-v3/offline-12-family-causal-r4`

- 456 schema-v4 cases;
- 12 scenario families across 7 domains;
- 38 lanes and 48 immutable plan-bank entries;
- 16 deterministic planner-lifecycle findings;
- zero case errors;
- zero offline claims of authenticated Free-tier verification;
- causal attribution explicitly not applicable on all offline oracle rows.

Hashes:

```text
results.jsonl             215e47e78da37a4621e424eacbd0af60d09a2b9fbc2b2c5cd95ed186d2ec7213
plan-bank.jsonl           72eb56d1060c4dcc88afaed249455bcf54c65cd81a6446b5fb142d8f356facb2
summary.json              8d82e9ed254ac171dfabc06a5c325c69acb5dc118d534d49283cc670cc88a2d4
lifecycle-findings.json   10f9aafb6309fa76a7761ac8db6cfa32d12b95864f8168582043c61ce9b6be43
```

Re-running the same command resumed all 456 cases and reproduced all four
hashes byte-for-byte.

## Validation receipts

```text
cargo +1.97.0 test -p provider-local --example planning_eval
42 passed; 0 failed

cargo +1.97.0 clippy -p provider-local --example planning_eval -- -D warnings
passed
```

## Completion conclusion

The design, deterministic implementation, fixture validation, route safety,
retry policy, lifecycle investigation, and offline receipt fidelity are
complete. The requested end state is not complete because repeated
authenticated DeepSeek V4 Flash Latest Free-tier trials and their variance analysis have
not run. Running them requires explicit operator authorization under the
repository's live-model policy.
