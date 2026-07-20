# Planning A/B benchmark

This benchmark compares the legacy Plan Mode reminder with the new
decision-complete three-phase contract through the real `LocalAgentProvider`.
Both profiles use the same model, synthetic repositories, tool loop, and
read-only enforcement.

It emits one JSONL row per profile/scenario plus a paired aggregate. The score
is intentionally deterministic:

- 25%: a typed `ProposedPlan` was produced within three turns;
- 25%: the repository digest did not change;
- 50%: scenario-specific paths and contract terms appear in the proposal.

Token counts, provider-reported cost, elapsed time, turns, and tool calls are
reported as efficiency diagnostics, not folded into quality. Compare paired
runs on the same model and require a positive quality delta without a material
regression in cost or turns before changing the default prompt.

The benchmark makes paid model calls and fails closed unless every gate is set:

```bash
CLARK_CODE_LIVE=1 \
CLARK_CODE_API_KEY=ck_live_... \
CLARK_CODE_MODEL=clark-code \
PLANNING_EVAL_SCENARIOS=typed-boundary,preference-migration,parser-fix \
PLANNING_EVAL_MAX_COST_USD=5 \
cargo run -p provider-local --example planning_eval
```

Do not use results from different models, scenario lists, or repository
fixtures as an A/B comparison.
