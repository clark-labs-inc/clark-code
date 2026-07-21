# Planning A/B benchmark

This benchmark compares the current decision-complete Plan Mode contract with
the concise, autoregressively ordered candidate through the real
`LocalAgentProvider`. Both profiles use the same model, synthetic repositories,
tool loop, and read-only enforcement.

It emits one JSONL row per profile/scenario plus a paired aggregate. The score
is intentionally deterministic:

- 25%: a typed `ProposedPlan` was produced within three turns;
- 25%: the repository digest did not change;
- 50%: scenario-specific paths and contract terms appear in the proposal.

Token counts are accumulated across every turn and model call. The last call's
context size, provider-reported cost, elapsed time, turns, and tool calls are
reported as efficiency diagnostics, not folded into quality. Compare paired
runs on the same model and require non-inferior quality with lower input usage
and no material regression in turns before changing the default prompt.

The benchmark makes paid model calls and fails closed unless every gate is set:

```bash
CLARK_CODE_LIVE=1 \
CLARK_CODE_API_KEY=ck_live_... \
CLARK_CODE_MODEL=clark-code \
PLANNING_EVAL_SCENARIOS=typed-boundary,preference-migration,parser-fix \
PLANNING_EVAL_REPETITIONS=2 \
PLANNING_EVAL_REASONING_EFFORT=low \
PLANNING_EVAL_MAX_COST_USD=5 \
cargo run -p provider-local --example planning_eval
```

Do not use results from different models, scenario lists, or repository
fixtures as an A/B comparison. The reasoning effort defaults to `low`; set it
explicitly when recording a comparison. Each turn is cancelled and recorded as
timed out after three minutes rather than leaving a paid benchmark hung or
discarding the rest of the paired run.
