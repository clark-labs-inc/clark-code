# Same-model loop sentinel

This harness tests `clark-code:free` as an occasional lifecycle sentinel. It
does not ask the model to improve implementation quality. The sentinel sees a
compact host-state packet, makes one forced typed `submit_loop_decision` call,
and stops without tools or prose.

The runtime design is fail-closed:

- exact user cancellation and exhausted verification budgets stop in the host
  without a model call;
- productive state deltas do not trigger the sentinel;
- ambiguous terminal re-prompts and repeated whole-state cycles with no novelty
  can trigger one sentinel call;
- failure count is not a stop signal: long exploration remains productive when
  it tests new hypotheses, targets, or evidence;
- `defer_to_host` never expands a retry or cancellation boundary; and
- timeout, malformed output, or provider failure falls back to the existing
  deterministic host boundary.

The model never has unilateral stop authority. A deterministic validator only
accepts `done` after a committed terminal answer, `cancelled` after native
cancellation, `verification_incomplete` after exhaustion or a missing recovery
path, and `stalled_no_progress` after a repeated whole-state cycle with no
novelty. An unsupported stop becomes `defer_to_host`. Receipts retain both the
raw model decision and the enforced action so false-stop tendencies stay visible.

The live matrix includes the reconstructed production incident
`bf36da49-7925-4e4d-a315-e929b314907c`, repeated-prose and repeated-failure stop
cases, one bounded recovery control, and shadow-only controls from the
160-iteration productive run, an expected missing-file path, and 24 unsuccessful
but novel exploration turns. Shadow controls measure false stops even though
production would not invoke the sentinel there.

Run intentionally with the current Desktop product credential:

```bash
CLARK_LOOP_SENTINEL_LIVE=1 \
CLARK_CODE_API_KEY=... \
cargo run -p provider-local --example loop_sentinel -- \
  --repetitions 5 \
  --concurrency 4 \
  --out target/loop-sentinel/run-id
```

The output directory must not exist. `receipt.json` records exact route, raw
and enforced decisions, stop recall, raw and enforced false-stop rates,
one-shot/schema compliance, latency, tokens, and provider-reported cost. Passing
requires 100% stop recall, zero enforced false stops, at least 75% raw decision
accuracy, and 100% strict one-shot route-valid decisions in the retained sample.
This qualifies the sentinel contract only; it does not by itself authorize
production integration.

## Retained result

`target/loop-sentinel/20260802-atomic-r5-c4/receipt.json` is the current atomic
protocol variance run: 35 exact Free-route calls and 10 deterministic host
controls. All 20 defer controls remained non-terminal, including five packets
representing 24 failed-but-novel exploration turns. The raw lifecycle action
stopped all ten production-derived incident packets and four of five repeated
zero-novelty cycles. All calls were strict, one-shot, route-valid, and
non-timeout, but the stricter status validator accepted only 10/15 required
stops; the gate is red. Average latency was 10,119 ms and maximum latency was
39,945 ms. This is evidence for an advisory signal, not an autonomous stopper.
