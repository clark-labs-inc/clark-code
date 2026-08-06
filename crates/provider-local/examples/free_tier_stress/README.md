# Free-tier production stress

This live harness exercises the product's actual included coding route,
`clark-code:free`, through `LocalAgentProvider`. It never substitutes a direct
paid model and rejects any model fallback recorded by the provider.

Each repetition runs one isolated multi-turn trajectory:

1. exact terminal delivery;
2. hostile file-content prompt injection plus trusted read-back;
3. twelve ordered `read_file` calls in one task;
4. a write followed by canonical read-back;
5. typed goal creation and completion;
6. a missing-file tool error followed by an exact, two-response self-stop; and
7. an explicit provider cancellation with a bounded settle time.

The harness is live-only and fail-closed. It requires
`CLARK_FREE_STRESS_LIVE=1` and `CLARK_CODE_API_KEY`. The requested model is
hardcoded; there is no model argument. Output directories must not exist so a
new run cannot overwrite earlier evidence.

```bash
CLARK_FREE_STRESS_LIVE=1 \
CLARK_CODE_API_KEY=... \
cargo run -p provider-local --example free_tier_stress -- \
  --repetitions 12 \
  --concurrency 4 \
  --max-provider-cost-usd 1.00 \
  --out target/free-tier-stress/run-id
```

`receipt.json` retains the source state, configuration, every scenario verdict,
typed run outcome, tools, model-response transport identities, retry counters,
usage, and bounded final text. Provider credentials are never serialized.

Evidence boundaries:

- This is a live host/provider sample, not packaged-app or cross-platform proof.
- A 429, transport failure, or provider outage is retained as infrastructure,
  not relabeled as model quality.
- Cancellation is a provider boundary. The UI Stop interaction remains covered
  by the frontend and resilience harnesses.
- The pass gate is 90% for each behavioral family, with zero route violations,
  hangs, or cancellation failures.

## Retained 2026-08-02 evidence

- `target/free-tier-stress/20260802-pilot-r1/receipt.json` used a legacy
  Platform credential and correctly failed closed: five model calls returned
  `model_not_found`; only the provider-cancellation case passed. This is a
  credential-class/route failure, not a Free-model quality result.
- `target/free-tier-stress/20260802-productkey-pilot-r1/receipt.json` passed all
  six cases in the previous harness revision with the Desktop product credential.
- `target/free-tier-stress/20260802-productkey-r12-c4/receipt.json` passed
  72/72 cases across 12 concurrent trajectories in that six-case revision. It
  records 142 model responses, 2,835,755 input tokens, 12,884 output tokens, no provider retry,
  no route violation, and only
  `deepseek/deepseek-v4-flash-0731` through DeepInfra. Case latency ranged from
  1,175 ms to 19,954 ms (p50 5,492 ms; p95 10,298 ms).

The stricter seven-case revision has not yet been run live. Its new
`missing_file_stop` oracle rejects retries, search, mutation, optional tools,
and any response count other than the tool call plus the terminal answer.

The receipts are ignored local artifacts. The tracked conclusion above is
durable; a release claim still requires a packaged-app and cross-platform run.
