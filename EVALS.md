# Evaluations, benchmarks, and simulations

Last source-and-artifact audit: 2026-08-04.

This is the repository-level index for Clark Desktop evaluation work. It says
which harness owns each claim, whether it is deterministic or live, what has
actually been observed, and where the receipts live. Detailed contracts stay
beside their harnesses; this file is the routing and current-state layer.

## Evidence rules

Use the narrowest claim supported by the evidence:

1. A unit or integration test proves its named contract.
2. A scripted/reference simulation proves harness mechanics and fixture
   solvability, not model quality or production autonomy.
3. A live provider run proves only the retained scenario/model/route sample.
4. A host run does not prove a packaged desktop or guest-VM journey.
5. A deterministic guest matrix does not prove authenticated or paid real use.
6. A model selector, process exit, final message, or green checklist is not a
   substitute for typed receipts and independent behavioral verification.

Additional operating rules:

- `target/` is ignored. Its receipts are local evidence and can disappear;
  promote durable conclusions into a tracked report before cleanup or release.
- `/tmp` evidence is disposable. Check that it still exists before citing it.
- Never call a live provider unless the user explicitly authorizes the exact
  model and run. Keep planner, executor/child, and judge identities explicit.
- Capacity, transport, image, and sandbox-start failures are infrastructure
  outcomes, not model-quality outcomes. Retain and classify the first failure.
- For planning/context evals, semantic quality is model-judged. Hidden checks
  are factual evidence only. The host must not score, relabel, merge, repair,
  or otherwise post-fix a model verdict.
- Tool and verdict JSON property order is part of the autoregressive contract.
  Validate exact ordered keys and regenerate the whole response on failure.
- Scripted/reference successes must never be reported as live successes.
- Prefer new output directories. Harnesses that refuse overwrite do so to
  preserve provenance, not as an inconvenience to bypass.

## Current map

| Surface | Primary entrypoint | Evidence class | Current retained state |
| --- | --- | --- | --- |
| Consolidated pre-release | `scripts/run-pre-release-benchmarks.sh` | deterministic plus opt-out live paid | 2026-08-06 offline receipt passed all release-blocking deterministic families; a separate bounded Qwen live lane passed |
| Release environment preflight | `harness/release-environment-preflight.mjs` | local redacted readiness contract | Discovers Desktop/Clark/Scientist ignored env files, validates secure modes, exact UTM guests, SSH reachability, worker paths, and Clark/OpenRouter route readiness without persisting secrets |
| Scientist and RSI product runtime | `../clark-scientist/script/run_qwen_specialist_product_eval.sh` | explicit paid Qwen plus deterministic simulation oracles | Two consecutive post-repair paid runs passed; one exercised bounded semantic correction and one passed every proposal on its first attempt |
| Planning and context | `planning_eval` | offline fixture gate plus live Qwen A/B and Qwen judge | Latest comparative run says plan delivery and extra context were beneficial in one scenario |
| Free-tier runtime and termination | `free_tier_stress`, `loop_termination`, and frontend Stop regressions | deterministic production-loop replay plus explicit live `clark-code:free` host evidence | Runtime/cancellation matrix passed 72/72, while the deeper planning sample completed all five long executors but passed only 8/25 hidden checks; route/cancellation gates passed, implementation quality did not |
| Durable memory | `memory_eval` | live model plus mixed deterministic/legacy LLM grading | Fresh 2026-08-04 108-case Qwen sample averaged 81.6% at $0.19; this remains a legacy mixed-grader diagnostic |
| Persistent goals | `goal_eval` | live model plus programmatic artifact grading | Post-fix Qwen repository case passed 100/100 with an explicit completed goal |
| Single-repo orchestration | `orchestration_benchmark` | scripted and live | Fresh 196-run Qwen sample had zero safety failures but every lane remained below its 90% stop threshold |
| Multi-repo orchestration | `multi_repo_orchestration_benchmark` | reference/current/external simulation | Clark-current is intentionally expected-red by contract |
| Work-graph orchestration | `work_graph_orchestration_benchmark` | reference/current/external simulation | Clark-current is intentionally expected-red by contract |
| Scout trust and scale | `scout_benchmark` and related scale gates | deterministic, UTM, optional live adapter qualification | Fresh local 25,000-service, 1,000/10,000/100,000 fan-in, million-event, and 100,000-task gates passed with explicit cost caveats |
| Skill lifecycle | `skill_experience_benchmark` plus durable worker provider/project contracts | deterministic local lifecycle plus typed worker boundary | The local 10-stage fixture covers catalog lifecycle; worker confinement and provider translation cover the sole remote runtime, while a fresh packaged receipt remains required |
| Security | `harness/security-simulation.mjs` | deterministic/rendered plus explicit paid Qwen lane | Fresh paid Qwen lane passed 14/14 vulnerable fixtures and 3/3 protected controls with zero false positives |
| UI resilience | `harness/resilience-benchmark.mjs` | simulated browser matrix plus optional live | 2026-08-06 release smoke passed 8/8 cases; full 64-case simulated matrix passed; the attempted live Free-route control failed and is not a live success |
| Cross-platform/UTM | `docs/clark-code-simulation-and-utm-qa-runbook.md` | deterministic guests, authenticated products, optional paid real use | Fresh Ubuntu ARM product build/install/auth/key-binding/visual journey passed; a current exact-source three-platform run remains open |
| Attachments | `src-tauri/tests/attachment_benchmark.rs` plus worker protocol/provider tests | scripted local plus typed worker transport | Local model-visible ingestion remains deterministic; obsolete exec-server transport fixtures were removed, and a packaged worker attachment receipt remains required |
| Durable remote worker | `crates/code-worker::project`, `crates/code-host/tests/idempotency.rs`, `crates/code-remote/tests/live_cpu.rs`, and `provider-remote-worker` | deterministic project confinement/streaming plus ignored real SSH/paid CPU lanes | The sole remote path pins ordered progress, one terminal response, bounded backpressure, sequence-gap rejection, durable same-request replay, ambiguous/conflicting retry refusal, native capability translation, project-root and symlink confinement, and fresh CPU residency; a fresh packaged receipt remains required |
| Standalone Clark CLI | `cargo test -p clark-cli -p security-cloud-sync` and `node --test harness/clark-cli-installer.spec.mjs` | deterministic auth/access/sync and local installer fixtures | Code is visibly first and included on Free for browser/device product credentials; an existing Platform API key retains explicit metered Code billing. All four specialists fail closed on live server subscription state; paired CLI/worker installation and checksum refusal pass locally |
| Clark terminal product contract | `node --test harness/clark-tui-product.spec.mjs` | deterministic Clark-native behavior and implementation-boundary simulation | 10/10 capability groups are implemented with exactly 8 intentional commands. Desktop and CLI/headless use the same Rust cloud-conversation client and `agent-core` snapshot/resume contract; the service exposes the same account rows through JWT and API-key front doors |
| Desktop chat/worktree journeys | `app/src/lib/fakeGitRepository.spec.ts`, `src-tauri/src/project_worktree/managed/tests.rs`, test-ui browser preview | deterministic transition matrix plus screenshot-backed UI | Clean, dirty, untracked, conflicted, owned, pinned, detached-commit, navigation, and narrow-window journeys are covered |
| Quick Chat | `sessionStore.quickChat.spec.ts`, `quick_chat_workspace`, native `quick_chat_workspace_is_stable_and_confined`, and ignored `live_qwen_37_flash_quick_chat_paid_evaluation` | deterministic frontend/native/provider integration plus explicit paid Qwen evaluation | Deterministic workspace identity, non-Git provider binding, sidebar grouping, and cross-device path recognition are covered; the 2026-08-04 paid Qwen lane passed at $0.000994 |
| Specialist GUI contract | `harness/specialist-ui-smoke.mjs`, `docs/desktop-ui-automation.md` | deterministic browser UI, no provider/model call | Paid-preview and free-coverage journeys for Scout, Security, Scientist, and RSI use stable `data-qa` hooks, starter prefills, representative canvases, and a 375px overflow check; packaged native auth and live model execution remain separate gates |
| Sandbox | `exec-sandbox` `sandbox_benchmark` | deterministic compiler/native containment and latency | Harness exists; no retained report was found |
| Migration | `provider-local` `migration_eval` plus durable worker project tests | deterministic local discovery plus typed worker confinement | Integration test covers Claude/OpenAI setup discovery locally; remote inspection uses the sole worker project plugin rather than an executor simulator |
| Worktrees | `provider-local` `worktree_simulation` | deterministic real-Git simulation | Integration suite covers linked, detached, dirty, hostile-helper, and scoped-edit behavior |
| Temporal Atlas wire scale | `temporal_atlas_benchmark` | deterministic serialization benchmark | Defaults to 10,000 services; no retained report was found |
| Kimi K3 Cloud Advisor | `harness/cloud-advisor-live.mjs`, `../clark/scripts/advisor_embedding_eval.mjs`, plus `clark-services` advisor tests | deterministic contracts, versioned S3/DynamoDB storage, and explicit paid K3/embedding calls | Direct and local K3 boundaries passed; deployed dev K3/S3/billing passed; all 19 eligible OpenRouter embedding routes were attempted and Qwen3-Embedding-8B was selected as the open-weight Clark Hash model; laptop and true SSH journeys remain release gates |

## Kimi K3 Cloud Advisor

The advisor is a private, server-owned supervisory prompt around the exact
`moonshotai/kimi-k3` route. Specialists send bounded evidence and typed candidate
capabilities; K3 returns one forced `submit_advisor_decision` tool call. The
local or SSH worker retains execution authority and fails open to its baseline
behavior when advice is unavailable. A separate signed, version-bound feedback
request records what actually happened.

Deterministic contracts cover request validation, paid entitlement and
organization membership, typed advice validation, consent classification,
HMAC-bound feedback receipts, gzip JSONL structure, S3 checksums, and the
started/complete version sequence. The LocalStack storage test observed both
versions under one opaque key and verified the final compressed stream.

Two direct paid attempts using JSON-object output failed because Kimi K3
returned `schema_version` as the string `"1.0"`. Those failures are not counted
as successes. A third paid direct attempt using the forced tool contract passed
with exact resolved model identity, positive provider cost, and a retained
compressed trajectory at
`target/cloud-advisor/20260802T180639117Z-direct/` (local ignored evidence).

The latest paid integrated local Platform API run passed the complete
authentication, membership, gateway ledger, Kimi K3, versioned telemetry, and
signed feedback path. Request
`advisor-live-882ea552-050f-4bff-9807-654a0bf4817d` reported `$0.011586`
upstream cost and `$0.013904` Clark billing, created one ledger row, and stored
advisor version `AZ_Do6q9dwMztbI6380JpXkIqeKM5u96` plus feedback version
`AZ_Do6q.AHX2k_8nnZ8xcK5DHkv3q9Bn` in the LocalStack fixture.

Claim boundary: this proves exact-model compatibility and the local service
boundary, not deployed AWS persistence, a packaged laptop journey, or a true
remote SSH specialist journey. Those remain mandatory release gates.

The deployed dev boundary subsequently passed request
`advisor-live-863cfd5b-b48c-414a-bfcb-23b56f31a614` against exact Kimi K3.
It recorded `$0.02632` customer cost, 2,187 total model tokens, one gateway
ledger debit, KMS-encrypted S3 version
`xDBr7PNfML2Kuq5jYPFfRfRuRZeLpXUw`, and a matching compressed-object SHA-256.
This is deployed HTTP/storage/billing evidence, but it predates the Clark Hash
DynamoDB addition and therefore does not close that new release gate.

### Clark Hash embedding selection

`../clark/scripts/advisor_embedding_eval.mjs` fetches the live OpenRouter
embedding catalog and applies a declared eligibility rule: text input,
embedding output, and at least an 8,192-token context. The retained paid run at
`../clark/target/cloud-advisor/embedding-eval-20260802-production-privacy/report.json`
and its `qwen4-retry.json` sibling observed 31 catalog entries, excluded 12
512-token models, and attempted all 19 eligible routes over the same 18
synthetic specialist/scientific records and 18 hard semantic queries. Every
call enforced `data_collection=deny` and disabled provider fallback. Seventeen
routes returned embeddings; Qwen 4B needed a focused retry after four transient
engine-overload 429s. Both NVIDIA free routes were rejected because their
endpoint training policy does not satisfy Clark's privacy contract; that is a
deployment incompatibility, not a model-quality result. Successful calls in
the privacy-matched matrix and retry reported `$0.013314538` total provider
cost.

Closed Voyage 4 variants scored 18/18 at dense and 512-coordinate Clark Hash
retrieval. Among tested open-weight choices, `qwen/qwen3-embedding-8b` was best:
17/18 top-one and 1.0 top-three recall both dense and after compression from a
16,384-byte 4,096-dimensional `f32` vector to a 256-byte Clark Hash. The one
miss confused a null scientific result with its closely related replication
record; this remains a known post-training target, not a perfect-quality
claim. Qwen was selected over the closed perfect scorers because its Apache-2.0
weights, 32K context, official training code, and standard Transformers/vLLM
runtime preserve a self-hosting, domain-post-training, and quantization path.

The runtime contract sends a fixed semantic anchor as an isolated paid request
before every record batch. It scores the live vector through the real Rust
Clark Hash codec against pinned reference
`0248511b7a3247ca520f4d232cd1f0e2f56f4058fff410a4e427b7c4a3f67291`
and fails closed below 0.95 approximate similarity. Exact packed bytes cannot
be the gate: repeated OpenRouter Qwen calls produced two hashes while retaining
0.978714 Clark Hash similarity, exposing backend floating-point variance. The
tolerant anchor detects material provider-weight drift without rejecting that
observed numeric noise. Embedding cost is recorded separately as Clark index
infrastructure and is not added to the user's Kimi K3 ledger debit.

The paid LocalStack DynamoDB boundary then passed with three synthetic JSONL
events expanded to three 256-byte semantic records, 36 band pointers, and one
manifest in a single 40-item transaction. It reported 232 embedding tokens,
`$0.000009280` provider cost, and the post-hardening rerun manifest
`31bd42f8d26cd2da7e003c199197f295dc1209e9b8c8df0db0fc118b7b8b28dd`;
an independent scan confirmed that the raw synthetic marker existed only in
the canonical S3-side event input and not in any DynamoDB string attribute.

## Scientist and RSI product runtime

The sibling Clark Scientist workspace owns the paid product-runtime harness:

```bash
OPENROUTER_API_KEY=... \
../clark-scientist/script/run_qwen_specialist_product_eval.sh
```

It runs one Scientist discovery turn plus RSI correlation-stress and restart
regression turns against `qwen/qwen3.7-flash`. The retained receipt requires
exact requested/resolved model identity, unique provider and idempotency IDs,
stable per-turn cache identity, bounded Scout context in both RSI trajectories,
full reasoning retention, deterministic non-divergent simulation outcomes,
positive metered cost, and a clean credential scan. A host run still does not
prove the packaged Desktop, live Scout cloud acquisition, or cloud projection
publication boundaries.

Current post-repair evidence:

- `../clark-scientist/runs/20260731T172200Z-qwen-specialist-product-eval/`:
  passed with five model calls and two bounded semantic repairs. This proves
  invalid RSI proposals enter correction rather than simulation dispatch.
- `../clark-scientist/runs/20260801T032736Z-qwen-specialist-product-eval/`:
  passed with three model calls and zero repairs. Both RSI proposals were valid
  on their first attempt; both deterministic oracles passed, Scout context was
  retained, and AutoResearch plus Clark Hash state persisted.
- `../clark-scientist/runs/20260802T080126Z-qwen-specialist-product-eval/`:
  the fresh release-candidate rerun passed with exact requested/resolved Qwen
  identity, the Scientist and both RSI trajectories, deterministic oracles,
  persisted research state, and `$0.0026983` provider-reported cost. The
  preceding `20260802T075242Z` run is retained as a failed pre-repair receipt.
- `target/scientist-paid-r1/evaluation-receipt.json`: fresh preflight-injected
  run passed with three logical turns, four model calls, one bounded structured
  repair, full reasoning retention, clean credential scan, and `$0.00287466`
  provider-reported cost. The correlation attack retained its expected
  counterexample; the restart regression passed.

Cloud durability has a separate deterministic contract. The `code-worker`
`cloud_sync` tests cover complete tree discovery, journal classification,
symlink rejection, authenticated segment upload, cloud verification, and the
returned receipt; `research_simulation_smoke` proves repeated mutating science
commands remain green only when their generated artifacts are acknowledged by
the cloud fixture. The process-level protocol smoke also pins environment-only
headless startup using the normal Clark API key, organization omission for a
single-membership user, and a nonzero pre-protocol exit with explicit stderr
when the cloud key is absent. Backend selection tests pin automatic
single-organization inference and an explicit ambiguity error for users with
multiple eligible paid organizations. Even an empty artifact tree now performs
a server subscription preflight before the worker accepts protocol input, so
`science:read`/`science:write` permissions cannot substitute for paid
specialist entitlement. These tests do not prove a production S3 deployment.

## Consolidated pre-release suite

### Environment preflight

Run the single readiness check before paid, UTM, or SSH work:

```bash
node harness/release-environment-preflight.mjs --all \
  --out target/release-preflight/all
```

The checker reads process variables first, then the ignored Desktop `.env`,
`../clark/.env`, and `../clark-scientist/.env`. It never executes dotenv text,
prints values, or writes credentials to the receipt. `--all` requires the Clark
Platform key, OpenRouter plus Clark specialist sync credentials, exact Windows
and Ubuntu UTM registrations, reachable `nucleus` SSH, and an explicit remote
CPU worker, credential environment, paid model, and receipt path. Remote host,
root, trajectory, and HTTPS route use safe defaults (`nucleus`, `/tmp` paths,
and the Clark Platform endpoint) unless set. The normal consolidated runner
invokes the same preflight automatically for its paid Clark lane and for
`--utm-preflight`.

The release-oriented entrypoint is:

```bash
./scripts/run-pre-release-benchmarks.sh --offline
```

The suite validates the authoritative capability inventory and feature matrix,
core/provider contracts, native host contracts, local-agent capabilities,
scripted conversation recovery, remote/git/worktree behavior, frontend
contracts, the skill journey, and the UI resilience sample. UTM lifecycle,
signed native Computer Use, and three-platform real-use receipts are optional
inputs with their own gates.

In the release workflow, the native host contracts and UI resilience sample
run as separate parallel jobs on clean native and browser runners. The
consolidated contract job uses `--delegate-native-host` and
`--delegate-ui-resilience` and records both families as `delegated`;
publication still requires all three jobs to pass. This keeps the Tauri host
link and Chromium profile/renderer storage isolated from the multi-crate Rust
build while preserving the same native contracts and eight release-blocking
fault cases. Local consolidated runs continue to execute both families
in-process unless those CI-only delegation flags are supplied.

The release workflow additionally runs `harness/webkit-smoke.mjs` and the
feature matrix runs `harness/attachment-smoke.mjs` against the production
frontend build. Both reserve an available loopback port and start their own
strict-port preview. The startup smoke requires Clark's signed-out and
authenticated text; the attachment smoke requires Clark's composer before
interaction. This prevents an unrelated server on a fixed port from producing
a false green result.

Its principal sub-harnesses are `harness/feature-matrix.mjs`,
`harness/utm-guest-benchmark.mjs`, and `harness/platform-real-use.mjs`; the
consolidator retains their status and receipt paths without collapsing their
different proof boundaries.

Important: the current script runs the paid lane by default and uses
`qwen/qwen3.7-flash`; `--offline` is the explicit no-network/no-credit mode.
This supersedes older prose that describes live as an opt-in `--live` flag.

Latest retained local receipts:

- `target/pre-release-benchmarks/20260806T201921Z-10902/`: the v0.1.130
  offline release receipt passed the full deterministic suite, including the
  required final-answer tool boundary, attachment smoke, and eight-case UI
  resilience sample; no live model calls were made.
- `target/pre-release-benchmarks/20260804-full-offline-r4/`: the current
  deterministic consolidated suite passed all release families (95 core,
  128 native, 613 local, 20 conversation, 5 remote, 614 frontend, and all
  feature/resilience/skill stages).
- `target/pre-release-benchmarks/20260804-full-paid-r1/`: the current paid
  consolidated lane passed all six bounded Qwen scenarios at `$0.004555`;
  the receipt records no API key.
- `target/pre-release-benchmarks/20260805-preflight-paid-r1/`: fresh paid
  consolidated validation passed all deterministic families plus the six
  bounded Clark Platform Qwen scenarios at `$0.005716`; the receipt records no
  API key and includes the environment-preflight receipt.

- `target/pre-release-benchmarks/v0.1.124-full-offline-20260802-r1/`: the
  consolidated offline pre-release suite passed after exercising the complete
  deterministic capability, Rust/native, frontend, skill, resilience, and
  integration set.
- `target/pre-release-benchmarks/v0.1.124-full-paid-20260802-r1/`: the same
  suite plus its bounded live lane passed against exact model
  `qwen/qwen3.7-flash`. The receipt records six live scenario contracts and
  `$0.004760` provider-reported cost without retaining the API key.
- `target/pre-release-benchmarks/release-v0.1.120-final-3/report.md`: passed in
  135 seconds; all nine deterministic feature lanes passed, including the
  self-owned WebKit and attachment previews, 517 frontend tests, the 16-stage
  skill journey, and the eight-case resilience sample. Paid, UTM,
  signed-native, and three-platform real-use lanes were not run.
- `target/pre-release-benchmarks/v0.1.115-local/report.md`: failed because the
  deterministic feature matrix and local capabilities failed.
- `target/pre-release-benchmarks/v0.1.115-local-rerun/report.md`: passed in
  102 seconds; inventory, feature matrix, core, native, local, conversation,
  remote, frontend, skill, and eight-case resilience families passed. Paid,
  UTM, native-smoke, and three-platform real-use lanes were not run.
- `target/pre-release-benchmarks/paid-manual-e146b01/report.md`: the isolated
  macOS `live_only` cheapest-paid chat/jobs lane passed with exact model
  `qwen/qwen3.7-flash`. This is a host live result, not a three-platform pass.

The consolidated receipt is `pre-release-receipt.json`; the human summary is
`report.md`. Supplying any guest real-use receipt makes the exact macOS,
Windows, and Ubuntu set release-blocking.

## Free-tier runtime, cancellation, and non-progress

Sources of truth:

- `crates/provider-local/examples/free_tier_stress/README.md`
- `crates/provider-local/examples/loop_sentinel/README.md`
- `crates/provider-local/tests/loop_termination.rs`
- `app/src/store/sessionStore.steer.spec.ts`

Production conversation `bf36da49-7925-4e4d-a315-e929b314907c` exposed a
host-loop failure, not a UI-only duplicate. At `2026-08-02T18:29:03.036Z` the
model delivered a valid final answer. Seven read-only Git inspections were
incorrectly registered as unresolved external mutations because the shell
classifier read Git's global `-C` option as the subcommand and split the
`2>&1` descriptor merge on `&`. The completion hook then insisted on canonical
verification while the deferred tool gate omitted `verify_effect`.

From that first false completion block through native cancellation, the
trajectory records 42 `clark-code:free` model responses, all resolving to
`deepseek/deepseek-v4-flash-0731` through DeepInfra. Forty returned unsolicited
plain-text `end_turn` output, two returned no content/tool call, and the final
in-flight request was cancelled. The exact user message `stop` entered the
active run as ordinary steering at `18:43:42.727Z`; the model acknowledged it,
but the unresolved-effect hook immediately re-prompted and produced 15 more
completed replies. The native Cmd+. cancellation aborted the request at
`18:45:35.973Z` and persisted `RunFinished(Cancelled)` five milliseconds later.

The bizarre prose itself was generated by the model, then amplified by the
host loop. Distinctive strings such as `3D-printed railgun`, `I-80 East`, and
`goatherd` do not occur in any earlier visible user, tool, or assistant item;
they first appear in these unsolicited assistant completions. Each completion
was then retained in the next model context, so the model began reacting to
and explaining its own corrupted text. The model's claims that the prose was
"injected" or "wasn't mine" are not forensic evidence. The trajectory proves
model-output degeneration plus recursive self-conditioning; it cannot prove
cross-request leakage inside an upstream provider.

The repaired contracts are narrow:

- exact `stop`, `/stop`, `cancel`, or `abort` during an active run invokes the
  provider cancellation boundary and never becomes queued/model-visible text;
- Git global options and descriptor merges preserve read-only classification;
- unresolved effect receipts automatically expose `verify_effect`; and
- deferred-tool discovery ignores connective/query boilerplate such as `and`,
  `with`, `tool`, and `capability`, preventing a multi-capability lookup from
  activating unrelated device, security, goal, and delegation schemas; and
- production stays iteration-uncapped. Only consecutive prose-only completion
  retries with an unresolved runtime obligation are bounded; any tool
  execution resets that non-progress counter.

The deterministic production composition test completes 160 consecutive
model/tool iterations, proving there is no 128-iteration product cutoff. Its
paired regression proves a genuinely unresolved external effect terminates as
`VerificationIncomplete` after bounded non-progress instead of printing
forever.

Retained live host evidence:

- `target/free-tier-stress/20260802-productkey-r12-c4/receipt.json` passed 72/72
  cases (12 each for exact response, hostile file-content injection, 12-read
  chains, mutation/read-back, typed goals, and native cancellation). It records
  142 one-attempt model responses, zero retries or route violations, 2,835,755
  input tokens, 12,884 output tokens, and zero provider-reported cost. AWS in
  the exact run window recorded 142 `/v1/chat/completions` requests, all 200,
  plus 15 stream-cancel warnings. Twelve cancellations were intentional test
  cases; gateway-only logs cannot safely attribute the other three, while the
  host receipts record no non-cancel case failure.
- The current seven-case harness additionally requires the model to stop after
  one expected missing-file error with exactly two model responses and no
  retry, search, mutation, or optional tool work. That stricter case is tracked
  but has no retained live receipt yet; the 72/72 result above remains evidence
  for the prior six-case contract only.
- Live feature-matrix, skill, forced-compaction, and steering/parallel-read
  tests passed on `clark-code:free`; the steering run continued across multiple
  compactions and verified its written artifact.
- `target/planning-eval-v3/free-stress-regional-audit-r2/` retained three
  frozen planners and five executors: 624 tool calls, 4,425,646 total tokens,
  no timeout, and five `RunFinished(Done)` outcomes. This is runtime endurance,
  not a quality pass: only 8/25 hidden behaviors passed, no lane completed all
  five, and one non-terminal proxy body-decode warning was printed after the
  fifth case. The five receipts themselves contain no trajectory error.
- A post-run contract audit found that the exact `pollUntilReady` surface,
  `audit-eu-west-1` resource name, and `api-v2-route` rollback label used by
  hidden checks were absent from the old task and assigned evidence. The
  regional-audit fixture now exposes those requirements across Project, Org,
  and Scout evidence and pins their observability in a deterministic test.
  This repairs future causal attribution; it does not retroactively improve or
  rescore the retained 8/25 sample.
- `target/free-tier-stress/20260802-pilot-r1/receipt.json` is an intentional
  failed control: a legacy Platform credential produced five 404
  `model_not_found` outcomes. It is credential/route evidence, not model
  quality.
- The experimental same-model lifecycle sentinel is quality-red and is not
  wired into production. It never edits or improves an outcome: one forced
  typed Free-model call judges a compact loop-state packet, while exact cancel
  and exhausted verification remain deterministic host stops. Productive
  controls are evaluated in shadow so model false stops remain observable.
  Failure count is deliberately excluded as a stop signal; a retained control
  has 24 unsuccessful turns that each add a new hypothesis or evidence item.
- `target/loop-sentinel/20260802-r5-c4/receipt.json` retained 35 two-field
  critic calls plus 10 host-only controls. It produced 14/15 accepted required
  stops, one raw false stop across 20 defer controls that the host validator
  rejected, one contradictory typed stop, and one output-limit non-decision.
  Exact routing was 35/35, average latency was 11,719 ms, maximum latency was
  44,630 ms, and provider-reported cost was zero. The gate failed.
- `target/loop-sentinel/20260802-atomic-r5-c4/receipt.json` replaced the
  contradictory action/status pair with one atomic decision enum and raised
  the bounded output ceiling. All 35 calls were strict, one-shot, route-valid,
  and non-timeout; all 20 defer controls stayed non-terminal, including five
  160-step productive packets and five 24-failure exploration packets. The raw
  lifecycle action stopped all 10 production-derived incident packets and four
  of five zero-novelty cycles. The stricter status validator accepted 10/15
  required stops, so the preregistered gate remained red. Average latency was
  10,119 ms, maximum latency was 39,945 ms, usage was 37,705 input plus 39,388
  output tokens, and provider-reported cost was zero.

The sentinel finding is narrower than a production recommendation. The same
Free model has useful independent stop signal on the observed terminal-reprompt
incident, but it is neither perfectly reliable nor fast enough to own run
lifecycle. Any future integration must remain occasional and advisory, enforce
stop eligibility from host facts, make at most one non-recursive request, and
fall back to deterministic cancellation/non-progress boundaries. Long or
unsuccessful exploration remains legal while it continues to add novelty.

AWS production logs corroborate the incident boundary. The full interval had
49 observed `/v1/chat/completions` status lines, all 200, while the trajectory
itself records 43 attempts, one transient retry, and two empty-output errors.
The gateway logged the long recovered stream cancellation at
`18:36:45.651Z` (419,903 ms) and the Cmd+. cancellation at
`18:45:35.987Z` (14,279 ms). After the typed stop, AWS accepted the remaining
1,192 trajectory events; after native cancel it accepted the terminal six
events with 47 ms ingest lag and synced the conversation to `idle`.

Claim boundary: the live results prove the current host/provider route sample.
They do not prove the packaged Stop interaction or macOS/Windows/Linux product
journeys. The planning sample is explicitly quality-red despite its runtime
endurance.

## Planning, plan adherence, and context

Source of truth:

- `crates/provider-local/examples/planning_eval/README.md`
- `crates/provider-local/examples/planning_eval/PREREGISTRATION_V3.md`
- `crates/provider-local/examples/planning_eval/WEAK_POINTS_V3.md`
- `crates/provider-local/examples/planning_eval/JUDGE_CONTRACT_V2.md`

The v3 harness has 12 synthetic multi-repository scenario families across
compliance, distributed systems, security, payments, data, configuration, and
client sync. Each family has a 60+ file workspace, an independent oracle, five
behavioral hidden checks, relevant and stale/conflicting evidence, and fixture
mutation/alternate-implementation gates. The full offline source/delivery
matrix contains 456 cases and 48 immutable plan-bank entries.

Treat offline runs only as fixture, schema, lane, oracle, and report validation:

```bash
cargo test -p provider-local --example planning_eval
cargo run -p provider-local --example planning_eval -- \
  --offline --repetitions 3 \
  --output target/planning-eval-v3/offline-gate
```

The retained `target/planning-eval*` offline history includes the legacy
`offline-full`/`offline-final` runs, v2 frozen/confirmatory runs, and the v3
`offline-gate`, 12-family, bank-handoff, delivery, causal, judge-contract, and
comparative-packet iterations. They record harness evolution and regression
gates. None is model-quality evidence, and the latest passing v3 contract
supersedes earlier offline schemas rather than averaging with them.

Live mode is fail-closed to authenticated `clark-code:free`. The current
2026-08-02 catalog sample resolved to
`deepseek/deepseek-v4-flash-0731`; older retained Qwen runs preserve their
historical model identity. The harness has bounded 429/502/503/504 retries and
no paid fallback. A confirmatory run must include typed handoff and
discarded-plan controls; Markdown prompt replay is only a delivery-mechanism
diagnostic.

### Planning run history

| Run family | State and conclusion |
| --- | --- |
| `target/planning-eval/live-representative-r2` | Pre-v2, 6-case diagnostic. Extra context looked directionally positive, but this used early host scores and is not current evidence. |
| `target/planning-eval-v2/live-pilot-r1` | Three-case pilot. It pasted rendered plan text and did not exercise typed `ProposedPlan` to `PlanDecision`; diagnostic only. |
| `target/planning-eval-v3/live-causal-pilot-r1` | 21 cases, 6 frozen plans. Plan delivery improved factual hidden checks by 0.2 in the narrow sample, but the causal classifier left 64 failed behaviors unresolved. |
| `live-knowledge-delivery-llm-r2` and `r3` | 18 cases each. Their reports remain `pending LLM trajectory review`; hidden-check deltas are evidence, not semantic conclusions. |
| `live-plan-enforcement-r1` | Typed plans were judged fully respected 6/9 and mostly 3/9, but completion honesty was false 9/9. The executor often followed incomplete plans. |
| `live-plan-enforcement-r2` | Private obligation audit increased judged satisfied behaviors from 5/45 to 11/45, but also made some wrong requirements more confidently wrong; no typed run satisfied all five behaviors. |
| `live-plan-enforcement-r3` | Clark hidden-plan artifact. Typed replay improved judged satisfaction/adherence, but separately judged identical plans received inconsistent coverage labels, so no plan-quality claim. |
| `live-plan-enforcement-r4` | Clean decomposed Qwen judge: 18/18 verdicts accepted; typed minus discarded was +18 satisfied and +41 followed across 45 behaviors, while plan coverage did not improve. |
| `live-progressive-context-r1` | Direct result was 8 typed-better, 3 ties, 1 worse, but the judge contradicted identical paired evidence and one 8,690-character plan was truncated to 6,000. It blocks a strong conclusion. |
| `live-progressive-context-r2` | Current best receipt after complete-plan repair and comparative Judge V2. Qwen final adjudication: `supported`/high; plan delivery `beneficial`/high; added context `beneficial`/high. |
| `free-stress-regional-audit-r2` | Current Free route endurance sample: three frozen planners and five executors all terminated normally after 624 tool calls, but only 8/25 hidden behaviors passed. No semantic judge was run; this is quality-red runtime evidence. |

The current best run is retained at:

- `target/planning-eval-v3/live-progressive-context-r2/`
- `target/planning-eval-v3/live-progressive-context-r2-judge-v2/`

It contains 24 trajectories, 12 immutable plans, 12 exact-byte typed handoffs,
12 discarded-plan controls, and three repetitions of one complex
`regional-audit-export` project. No plan was truncated and every typed SHA-256
and character count matched. The blinded pair judge classified plan delivery
as better in 10/12 pairs and equivalent in 2/12. All three no-context plans
were `mixed`/`partially_ready`; seven of nine context-assisted plans were
`good` or `excellent`.

Every accepted plan, pair, and final verdict passed a separate Qwen semantic
audit. Six plan candidates and eight pair candidates were rejected before the
final set; malformed schemas and two 429s caused whole-response retries. The
host did not post-fix semantic output. Provider telemetry was `$0.243555` for
planning/execution and `$1.155141` for judging; that is not necessarily the
user-billed amount.

The strongest defensible statement is limited: Qwen found complete plan
delivery and additional Project/Org/Scout context beneficial in this one
high-fidelity simulation. The planner, executor, candidate judge, audit judge,
and final adjudicator used the same model family; only one scenario family was
run live; no arm passed every hidden behavior. Cross-model, cross-project, and
real-repository replication remains open.

The deterministic engine journey
`plan_mode_journey_denies_edits_threads_feedback_and_builds_after_approval`
uses the current hidden `<proposed_plan>` response envelope, then exercises the
typed `ProposedPlanUpdated` and `PlanDecision` boundary through revision and
approval. It no longer relies on the retired model-visible `propose_plan` tool.

Judge commands:

```bash
cargo run -p provider-local --example planning_eval -- \
  --judge-input target/planning-eval-v3/live-run \
  --output target/planning-eval-v3/live-run-judge-v2

JUDGE_RUN_LABEL=live-run-qwen-v2 \
node crates/provider-local/examples/planning_eval/judge_qwen_v2.mjs \
  target/planning-eval-v3/live-run-judge-v2
```

## Memory and persistent-goal evals

`memory_eval` contains 18 live scenarios: three each for stale-memory handling,
correction, hallucination resistance, proactive memory, recall, and churn. It
uses scratch project/global memory stores and writes one JSONL result per
scenario.

```bash
CLARK_CODE_BASE_URL=https://api.clarkslabs.com/v1 \
CLARK_CODE_API_KEY=... \
cargo run -p provider-local --example memory_eval -- \
  --model clark-code --out target/memory-eval/results.jsonl \
  --concurrency 6
```

This is a legacy mixed grader. Some checks are deterministic, while its LLM
judge extracts JSON from surrounding prose and reduces checks to host pass
fractions. The judge now uses the exact model selected by `--model` and the
same production `LlmClient` as the agent, including HTTP/2 negotiation,
session affinity, idempotency, and bounded 429/transient retries; it can no
longer silently fall back to a hardcoded `clark-code` route. It still does not
meet the newer planning eval's exact-schema/no-post-fix standard.

The fresh retained sample is
`target/memory-eval/v0.1.124-qwen37-full-20260802-r2/results.jsonl`: 108 live
cases, 18 per dimension, exact model `qwen/qwen3.7-flash`, zero transport
errors, 79.55% mean score, `$0.247764` provider-reported cost, and 2,980.25
aggregate scenario-seconds. Dimension means were 88.89% churn, 81.48%
proactivity, 78.24% correction, 77.78% stale-memory, 75.93% hallucination,
and 75.00% recall. This is a model sample and a mixed-grader diagnostic, not a
release pass/fail claim.

The current rerun is
`target/memory-eval/20260804-qwen37-full-r1/results.jsonl`: all 108 scenarios
completed without transport errors, averaged 81.6%, and reported `$0.19`.

`goal_eval` gives the real local provider three autonomous deliverables:
Snake, a multi-page portfolio website, and a search-heavy Rust repository fix
with 1,500 ignored decoys. It grades resulting artifacts, goal state, tools,
tokens, cost, and wall time. It is live-only and has explicit scenario and cost
gates:

```bash
CLARK_CODE_LIVE=1 \
CLARK_CODE_API_KEY=... \
CLARK_CODE_MODEL=qwen/qwen3.7-flash \
GOAL_EVAL_SCENARIOS=snake,website,repo-tools \
GOAL_EVAL_MAX_COST_USD=0.50 \
GOAL_EVAL_RESULTS_JSON=target/goal-eval/results.json \
cargo run -p provider-local --example goal_eval
```

The first fresh full Qwen run exposed a real capability-boundary defect:
Snake and website passed, and the repository artifact scored 100/100, but the
repository case never created a goal. After explicit goal requests began
preactivating only the three lifecycle tools on the first turn, the targeted
rerun at
`target/goal-eval/v0.1.124-qwen37-repo-goal-fix-20260802-r5/results.json`
passed 100/100 with `create_goal` before edits, `update_goal`, a terminal
`complete` state, 13 tool calls, 82,510 input tokens, 1,510 output tokens,
`$0.001767` provider-reported cost, and 25 seconds wall time. This proves the
repaired repository case, not a post-fix rerun of all three deliverables.

The current full rerun is
`target/goal-eval/20260804-qwen37-full-r1/results.json`: Snake, website, and
repo-tools each scored 100/100 with an explicit completed goal; total reported
cost was `$0.026619`.

## Orchestration benchmarks

### Single repository

`orchestration_benchmark` compares single, planned-single, reader/writer,
reviewed, cheap-subagent, homogeneous-strong, Clark Cloud, and mixed ACP lanes.
Scripted mode is the default and validates permissions, writer leasing,
recovery, grading, and lifecycle replay. Live mode uses real synthetic Git
repositories and is explicitly gated by `ORCHESTRATION_BENCH_LIVE=1`, filters,
timeouts, tree budgets, run caps, and cost stops.

```bash
cargo run -p provider-local --example orchestration_benchmark -- --list
cargo run -p provider-local --example orchestration_benchmark -- \
  --out target/orchestration-benchmark/offline-full
```

Fresh evidence is retained at
`target/orchestration-benchmark/v0.1.124-qwen37-full-supported-20260802-r1/`.
It contains 196 live Qwen runs: 28 for each of seven lanes. Safety failures,
lifecycle-trace failures, and duplicate tool receipts were zero. Pass rates
were 85.7% for single, cheap-subagents, clark-cloud, and reviewed, and 89.3%
for planned-single, reader-writer, and homogeneous-strong. Every lane correctly
retained a `stop` decision because it missed the preregistered 90% threshold;
the result is therefore evidence of safe execution plus unresolved routing and
task-quality misses, not a winning orchestration configuration. Total
provider-reported cost across lanes was approximately `$0.525507`.

### Multi repository

`multi_repo_orchestration_benchmark` creates independent Git repositories and
grades explicit dependency graphs, pinned baselines, isolated content-addressed
patches, fresh replay, dirty-user-state preservation, targeted recovery, model
routing, review, cloud routing, and usage. Seven scenario families cover
contract propagation, rolling compatibility, generated clients, failure
recovery, cloud/local rollout, sequential anti-delegation, and baseline drift.

The deterministic `reference` adapter proves the benchmark is solvable.
`clark-current` is deliberately expected-red because Clark does not yet expose
the required multi-writer control plane. Only an external candidate with at
least three measured repetitions can cross the value gate.

### Work graph

`work_graph_orchestration_benchmark` adds typed dependencies, resource leases,
content-addressed artifacts, readiness wakeups, expiry, targeted recovery,
fresh verification, and zero-token waiting. Eleven scenarios include a wide
eight-writer/four-repository case and a sequential anti-case. Its reference
candidate proves mechanics; `clark-current` remains expected-red until it emits
authoritative production traces. A simulation can never unlock its value claim.

See the README beside each benchmark for external-candidate JSON protocols,
write containment, retained artifacts, and exact value gates.

## Scout trust, scale, and qualification

`scout_benchmark` is offline by default. It exercises skill resolution,
business-system cartography, fixed-point and negative controls, signed
append-only ledgers, authority, replay proofs, order-independent roots,
tenant-isolated ingestion, SQLite projection, affected-row work, high fan-in,
conflict corpora, and enterprise-scale graphs.

```bash
cargo run -p provider-local --example scout_benchmark -- \
  --out target/scout-benchmark/local \
  --host-label local --containment external
```

The 2026-08-02 local refresh passed the deterministic 25,000-service enterprise
graph, the one-million-object accumulator, 1,000/10,000/100,000 high-fan-in
sweep, one-million-event replay, and 100,000-task scheduler gates. The
million-event receipt took 231,752 ms and peaked at 3.776 GiB RSS. The
scheduler claimed its 100,000-task frontier in 1,102 ms and reproduced exact
reference, retry, and restart state. The high-fan-in receipt preserves 24
future-gate failures: equality and deterministic roots pass, but replay remains
O(N) and the index is not fixed width. These substantial memory/index costs
mean the monolithic reducer is still not an economical thousands-of-services
production architecture. The versioned receipts live under
`target/scout-benchmark/v0.1.124-*-20260802-r1/` and are ignored local evidence;
the checked-in source/report contract is the durable boundary.

Related boundaries:

- `scout-accumulator` `accumulator_scale`: authenticated object-root scale;
- `agent-orchestration` `scout_million_event_gate`: million-event replay;
- `scout-coordinator` `scheduler_scale`: fenced frontier-task scheduling;
- `harness/scout-utm-qualify.mjs`: byte-verified Ubuntu/Windows guest parity.

## Skill experience

`skill_experience_benchmark` runs a 10-stage local lifecycle journey from an
empty home: legacy discovery, managed install, collisions, exact binding,
provider-visible request, source update, stale-binding rejection, refresh,
restart, and uninstall. Remote coverage belongs to the durable worker
provider/project contracts, not a provider-local tunnel simulator. The
synthetic CI fixture is self-contained; a real Obra Superpowers checkout adds
compatibility evidence.

```bash
cargo test -p provider-local --example skill_experience_benchmark
cargo run -p provider-local --example skill_experience_benchmark -- \
  --synthetic --out target/skill-experience-benchmark/local
```

The retained 16-stage receipt at
`target/pre-release-benchmarks/v0.1.115-local-rerun/skill-experience/` predates
removal of the legacy tunnel lane and is historical evidence only.

## Security simulation

The tracked current summary is `docs/security-simulation-report.md`.
`harness/security-simulation.mjs --offline` runs deterministic finalizers,
adversarial Rust tests, frontend/history rendering, and UI smoke. The explicit
paid lane is locked to `qwen/qwen3.7-flash`:

```bash
node harness/security-simulation.mjs --offline
node harness/security-simulation.mjs --paid --live-only
```

The fresh 2026-08-04 paid receipt currently exists at
`/tmp/clark-security-simulation/receipt-paid.json` and passed the exact Qwen
lane: 14/14 vulnerable files matched, 3/3 protected controls recognized, and
0 protected-control false positives, with reported cost `$0.004371`. The receipt and `/tmp`
screenshots are disposable; the tracked report is the durable conclusion.

The smaller Rust `security_scan_simulation` and `security_diff_simulation`
examples exercise standard/diff sealing plus negative controls for incomplete
coverage, missing attack paths, invented/stale evidence, and changed targets.

## UI resilience and desktop/VM real use

The resilience benchmark enumerates six faults: rate limit, duplicate tool IDs,
event-stream disconnect, provider-process loss, cloud-sync delay, and user
cancel. Smoke runs baseline, each individual fault, and the combined case:

```bash
node harness/resilience-benchmark.mjs --smoke \
  --out=target/ui-resilience/current
```

The exhaustive 2026-08-02 simulated matrix at
`target/ui-resilience/v0.1.124-full-live-20260802-r1/report.json` passed all 64
fault masks, including the six-way combined case. A separately authorized
Free-route live control reached a typed provider failure and retained its
failure screenshot, but did not pass; do not combine that failure with the
simulated 64/64 result. `--live` and `--live-only` cross a materially different
provider boundary and require explicit authorization.

The complete desktop/VM architecture, current evidence matrix, privacy rules,
and exact sequence live in
`docs/clark-code-simulation-and-utm-qa-runbook.md`. The fresh Ubuntu ARM receipt
at `target/utm-product/v0.1.124-ubuntu-auth-20260802-r6/receipt.json` passed a
native Tauri build, atomic install, unprivileged launch, visible window, real
Clark-owned short-lived authentication, same-account provider-key binding,
project/model configuration, OCR-backed product controls, zero manual VM
actions, and erased transient auth transfer. The journey also found and fixed
a stale probe that still observed the retired global settings key after the
product moved to account-scoped settings; the matching Windows authenticated
probe and observer were repaired at the same boundary. This is a dirty-source
Ubuntu debug-product receipt, not current packaged Mac/Windows/Linux release
qualification. Reopen every mutable VM, binary, auth, pricing, and receipt
boundary before making a current three-platform claim.

## Durable remote worker

The reusable remote boundary is split into `code-host` (strict JSONL protocol,
plugin registry, cancellation, and trajectory records), `code-worker` (the
provider/plugin composition root), `code-remote` (SSH artifact deployment,
checksum verification, credential bootstrap, correlated streams, and
disconnect cleanup), and `provider-remote-worker` (the `agent-core::Provider`
translation used by a native host). Protocol v2 carries bounded, ordered
progress frames followed by exactly one terminal response; the transport
rejects sequence gaps per request and never attaches a late frame to another
request. `src-tauri` exposes the sole `remote_worker_connect` attach boundary through the account-partitioned
native runtime registry. Project inspection and coding
sessions use the same opaque worker handle; the former renderer-owned tunnel,
binary override, and tunnel tests have been deleted. The native registry owns
single-flight attachment and one bounded typed retry; the renderer has no
retry classifier. A live worker health receipt is bounded to 750 ms, and cold
process restart reuses checksum-verified, content-addressed worker and config
artifacts instead of uploading identical bytes. Cold bootstrap opens one
private mode-`0700`, process-owned SSH control socket and multiplexes every
probe, upload, and the worker channel through its non-persistent master. Unit
contracts pin the private permissions, short socket path, session reuse, and
`ControlPersist=no`; the packaged timings below close that transport claim.

Fresh non-model SSH evidence is retained under
`target/remote-reconnect-benchmark/20260804-control-master/`. Two consecutive
`nucleus` runs used the Linux worker extracted from the signed development
bundle and a deliberately fake provider key. Both proved worker version
`0.1.126`, `linux-x86_64`, `remote_worker` residency, the same verified binary
digest, `control_master`, ping/catalog, graceful shutdown, no credential in the
receipt, and `model_called=false`. Their measured connect times were 2,193 ms
and 2,275 ms; total start/catalog/shutdown lifecycles were 2,299 ms and 2,388
ms. Post-run local control-socket/master and remote worker process counts were
zero. The same directory's `registry-final.json` exercises the Desktop command
core and account-partitioned native registry against the final freshly rebuilt,
signature-verified bundle resource layout: the first connect completed in
2,237 ms, the second health-checked attach in 48 ms, both returned one identical
opaque handle, account worker count stayed
at one, and account teardown returned it to zero. The receipt contains no
account identity or credential and records `model_called=false`. This closes
the current transport, packaged-artifact, native warm-reconnect, stable-count,
and teardown receipts; the exact signed-in conversation-reopen visual remains
a separate UI claim.

Reconnect single-flight gates are weak native references and are reclaimed
after callers finish. A bounded 128-entry circuit opens for ten seconds after
three failed connect operations, while successful connection or account
teardown removes its state. This prevents repeated failures and arbitrary
worker specs from leaking registry coordination state.

Live `HostSession` entries, their providers, projections, and trajectory
clients now live in `RuntimeRegistry`; the parallel `AppState.sessions` map was
deleted. Session ownership is a typed `AccountKey` from the moment a native
credential enters provider configuration, rather than being attached later by
cloud-history setup. Account teardown unpublishes matching sessions before
awaiting provider shutdown, and the retained native test proves another
account's session remains registered. Global-memory preview no longer accepts
an account label from the renderer, and native provider preparation replaces
any renderer `memory_scope` with the server-validated account. The current
native run passed 120 tests with one explicitly gated live SSH test ignored;
its negative coverage includes missing authority and a forged cross-account
memory partition.

The no-secret-WebView boundary is now native-owned. Google sign-in, refresh,
Clark token exchange, retained authentication, current-account publication,
and sign-out run in Rust. Cloud, mobile, specialist, security, artifact, and
provider commands resolve the same atomic native account generation; their IPC
contracts contain no bearer, OAuth, refresh, or mobile claim token. The raw
Google token plugin permission and obsolete token-bearing commands were
deleted. Native tests pin descriptor-only auth responses, cross-account atomic
publication, account-and-host-bound mobile claims, and denial of raw Google
commands. All 614 frontend tests, typecheck, and production build pass; the count dropped only
because the deleted local-history migration tests no longer describe a product
path. The freshly rebuilt, strict-signature-verified macOS bundle is clean when
scanned for the retired provider, SSH, reconnect, auth, and Vite boundary names.
Its imported Security framework is used for platform TLS/certificate and code-
signing support; the binary imports no Keychain `SecItem`/`SecKeychain` secret
API symbols, and the workspace dependency graph contains no credential-vault
package. A
real Google sign-in remains a separate product receipt.

`remote_worker_connect` is the only attach/reconnect command. Its typed
receipt reports `started`, `reused`, or `replaced`, native elapsed
milliseconds, `control_master` transport residency, and the current account's
worker count without exposing the
account identity. Worker publication holds the native account lifecycle gate,
so sign-out cannot race a slow SSH bootstrap and leave an orphan worker. The
redundant renderer-addressable reconnect command has been deleted.

The registry keys live sessions by `(AccountKey, SessionKey)`, not raw IPC
strings. Identical conversation ids can coexist without cross-account lookup,
publication rejects a mismatched embedded account, and sign-out removes one
account partition under a single registry lock. Empty, control-bearing, and
unbounded session identities fail before lookup or publication. Registered
remote projects remain `ProjectKey` values inside the registry and are reduced
to protocol strings only at the worker-provider adapter.

The shared skill-catalog cache is no longer a second `AppState` authority.
`RuntimeRegistry` owns one service per native account partition, supplies the
same service to provider and UI callers, removes it during account teardown,
and reports the final cache count in the whole-app shutdown receipt.

The native `account_lifecycle` reader/writer gate admits same-account worker,
session, cloud, and projection work concurrently while sign-in/sign-out owns an
exclusive generation transition. The retired split lifecycle name and meaning
were removed. A `CloudAccess` value owns a shared lease for the full
authenticated request, replacing scattered post-request account comparisons.
Concurrency tests prove unrelated same-account requests do not queue behind one
another while sign-out still waits for every admitted lease before
unpublishing its generation.

The packaged journey exposed why a single exclusive request gate was not an
acceptable implementation: startup key provisioning, history, billing, and
conversation-open requests queued behind one another and remote admission did
not finish within the 30-second product bound. Replacing it with shared request
leases removed that head-of-line stall. The journey also activates the exact QA
window process by inventoried PID before semantic clicks and retains post-click
screens on failure, so an occluded-window input miss cannot masquerade as a
reconnect regression.

The packaged Desktop persists its retained Clark session, account-scoped Code
key, and MCP environment values in one app-owned ChaCha20-Poly1305 envelope
plus one random adjacent key under the private app-data directory. It does not
use an operating-system credential vault. WebView storage retains only
account-partitioned non-secret UI state, MCP server ids, and environment names;
the native account descriptor is held in renderer memory only. The obsolete WebView auth
record and v1 native credential envelope are deleted rather than migrated or
aliased; the user signs in cleanly. Legacy MCP values are imported once into
the encrypted file and removed. This deliberately
does not claim protection from another process already running as the same
laptop user.

The standalone Clark CLI uses the same storage policy: `auth.enc` is an
app-owned ChaCha20-Poly1305 envelope and `auth.key` is its adjacent random key
under `CLARK_HOME`, both restricted to the current OS user. The OS-vault
dependency and runtime fallback were deleted. The retired plaintext `auth.json`
is deleted rather than migrated or read. Four focused storage contracts prove
encrypted round-trip, plaintext absence, authenticated-tamper rejection, and
complete file cleanup. A direct module probe passes 4/4; the full CLI crate is
temporarily blocked by the concurrent `AgentEvent` migration in untouched
`tui/provider_events.rs`.

Sign-out is one tokenless native transaction (`clark_sign_out`), not separate
renderer auth/cache cleanup calls. Every remote session is account-bound at
atomic publication, local sessions acquire the validated owner when cloud
history attaches, and sign-out deletes the encrypted account credentials
before publishing in-memory teardown. It then removes only that account's
sessions and workers. If encrypted persistence fails, the old account remains
active in both native and renderer state instead of reporting a partial switch.

The isolated macOS product journey now seeds that same authenticated native
envelope directly. Its WebKit helper carries only nonsecret project/model
settings and a marker, asserts that credential-shaped fields are absent, and the
journey decrypts the disposable envelope afterward to prove authenticated
retention and plaintext absence. The harness no longer places a session token
in WebKit storage.

The renderer can no longer construct a generic provider connection. Its closed
`ProviderLaunchRequest` has no endpoint, header, bearer, executable override for
native providers, or account-partition field, and denies unknown fields. Native
account authority supplies those values after deserialization. Google OAuth
origin and client configuration are compile-time native `CLARK_*` settings;
the old `VITE_CLARK_*` connection/auth settings and their sign-in-screen copy
were deleted. Provisioning, MCP synchronization, provider open/reconfigure,
remote-worker publication, and late mobile-claim publication all cross the
same native account-generation gate. Sign-out holds its exclusive generation
transition across durable credential deletion and live teardown, while whole-
app exit unpublishes every session, worker, claim, gate, and circuit before
bounded provider/process shutdown.

The comparison target is intentionally simple. The sibling Codex checkout's
app-server contract initializes one transport, resumes a persisted thread by
id, and streams bounded protocol notifications instead of reconstructing a
runtime per conversation. A read-only process inspection of the installed
ChatGPT/Codex desktop harness showed one long-lived `ssh -T` process per remote
host running `codex app-server proxy`, with server keepalives and no per-chat
SSH process. Clark mirrors those useful ownership properties with one native
worker per account/host/spec boundary, opaque handles, atomic session open, and
warm health-checked reuse; it does not copy the implementation.

The packaged macOS native-credential profile tests pass 10/10. The current
signed, non-model remote-reconnect receipt is retained under
`target/macos-product-journey/20260804-runtime-registry-proof-16/`. It opened a
disposable existing conversation on the configured `nucleus` SSH host, switched
away and reattached in the same process, then gracefully stopped, relaunched,
and reopened the same conversation. Native evidence records exactly two cold
worker publications for the two processes (5,183 ms after a fresh packaged
worker build and 2,376 ms on restart), exactly one
account worker each time, `linux-x86_64` residency over `control_master`, no
additional connect for warm reselection, and one session plus one worker
retired at each shutdown. OCR-backed conversation-ready observations were 1,372
ms initially, 1,294 ms warm, and 2,302 ms after restart, with the reconnecting
surface absent.
The encrypted native envelope retained authentication with no plaintext,
unscoped settings and WebView credential fields were absent, one disposable desktop key and
conversation were created then removed, the profile/workspace were erased, all
67 personal-state entries were unchanged, and the prior normal app state was
restored. It made no model call and records no host/account/key identifiers in
the native runtime projection.

This run also corrected two invalid earlier harness assumptions. An optimistic
cached transcript is not connection readiness, so conversation selection now
waits on the native `remote_worker_connected` event. OpenSSH does not use a
synthetic `$HOME` to locate its user config, so the disposable SSH-alias path was
deleted; the product smoke uses the laptop's real configured host exactly as
Clark does. The no-model live CPU contract now invokes `session.open`, replays
the same durable request ID, rejects the same ID with different work, and then
checks ping/catalog. That exposed and removed the worker's hard dependency
on optional Bubblewrap: canonical project-root file containment remains active,
shell is permission-gated, and the command auto-allow list stays explicit.

The cross-platform
UTM contract suite passes 50/50 after deleting its WebView session/token seed:
Ubuntu and Windows now generate the exact v2 ChaCha20-Poly1305 files on the host,
write them into the guest's app-private data directory before launch, and seed
only account-partitioned project/model settings through WebView storage.

Deterministic evidence:

```bash
cargo test -p code-host -p code-worker -p code-remote -p provider-remote-worker
cargo clippy -p code-host -p code-worker -p code-remote -p provider-remote-worker --all-targets -- -D warnings
```

The `code-host` idempotency tests prove exact replay, conflict and ambiguity
refusal, and fixed-capacity admission without evicting old request identities.

The ignored transport receipt requires `CLARK_REMOTE_CPU_LIVE=1`,
`CLARK_REMOTE_CPU_WORKER`, `CLARK_REMOTE_CPU_CREDENTIAL_ENV`, a local value for
that environment variable, and the optional `CLARK_REMOTE_CPU_HOST`, root, and
trajectory variables. It proves SSH deployment, authenticated Clark access
reconciliation, remote-worker residency, a real coding `session.open`, catalog
correlation, durable replay without re-execution, conflicting-request refusal,
and shutdown, but makes no model call. The fresh receipt at
`target/code-remote/20260805-nucleus-transport-r1/receipt.json` passed against
worker v0.1.126 on `linux-x86_64` over the process-owned SSH control master:
connect was 3,390 ms, catalog was 54 ms, shutdown was 62 ms, and total runtime
was 3,685 ms. It recorded no credential and made no model call. The ignored paid
lane additionally requires `CLARK_REMOTE_CPU_PAID=1` and an explicitly named
`CLARK_REMOTE_CPU_MODEL`. It performs one bounded no-tool coding turn and
retains streamed agent events separately from the terminal response. The model,
provider route, cost, first failure, and receipt must be retained before
claiming paid success. A ping, process exit, or green transport test is not
evidence that a paid model turn succeeded. The fresh receipt at
`target/code-remote/20260805-nucleus-paid-r2/receipt.json` passed the explicit
paid lane against `qwen/qwen3.7-flash`: execution remained in the remote worker,
the exact response matched, no credential was recorded, and usage was 4,621
input tokens, 60 output tokens, and `$0.000176` provider-reported cost.

## Standalone Clark CLI

The human-facing `clark` executable is separate from the desktop GUI. With a
TTY and no subcommand it opens a workspace picker with Code first, followed by
Scout, Security, Scientist, and RSI. Code is included on Free; every specialist
is authorized from current server subscription state before its provider,
model, or native worker starts. Browser/device sign-in creates a product
credential with Code included on Free; an existing Clark Platform API key is
also a valid cloud credential and keeps Code on its metered API billing
boundary. Either key identifies the machine and carries server-assigned
permissions; neither proves paid entitlement. Clark does not currently expose
user-configurable `science:read` or `science:write` key scopes in the key UI,
so the CLI must not claim that users configure them. A single
eligible organization is inferred, multiple eligible organizations require an
explicit choice in the human TUI (or `--organization` in scripts), and no
organization environment variable is required. The TUI lock is explanatory,
not authoritative: the Clark service separately checks live paid coverage for
the exact organization on CLI authorization and on direct Scout, Security,
Scientist, and RSI cloud operations. Credential validity never substitutes for
that subscription check.

Deterministic evidence:

```bash
cargo test -p clark-cli -p security-cloud-sync
node --test harness/clark-cli-installer.spec.mjs
# In the sibling Clark service checkout:
cargo test -p clark-services --features artifacts platform_api::specialist_guard::tests --lib
```

These tests cover parser/auth/access contracts, the visible five-workspace
ordering, ambiguous-organization selection, direct specialist-request
organization extraction, Security's fail-closed cloud sync, paired installer
success, and checksum-tamper refusal. They prove the local deterministic
boundary, not a production subscription lookup, browser/device authentication,
signed/notarized binaries, or the public CDN; those require the release workflow
and deployed endpoints to be exercised.

### Clark terminal product contract

The active goal is to build Clark's minimal human and headless terminal as a
thin renderer over the same Clark application core used by Desktop. Browser,
device-code, and API-key authentication resolve account, organization, project,
and entitlements through Clark. Code plus Scientist, RSI, Scout, and Security
retain distinct workflows and typed output. Every paid specialist fails closed
when authentication, entitlement, required cloud preflight, or required journal,
evidence, experiment, and artifact synchronization cannot be verified.
Scientist and RSI preflight the account- and organization-scoped
`/v1/science/access` boundary before their native worker starts; a successful
turn still requires the worker's verified per-file/per-segment cloud receipt.

`harness/clark-tui-product.contract.json` encodes that product contract. It is
not a feature-parity or source-comparison scorecard. The only public commands
are `/attach`, `/clear`, `/goal`, `/init`, `/model`, `/permissions`, `/quit`, and
`/status`. File/editor input is intentionally one `/attach` workflow. Vim,
themes, pets, background-terminal management, review consoles, side chats,
generic agent commands, and local session-database commands are not part of the
Clark terminal product.

```bash
node --test harness/clark-tui-product.spec.mjs
node harness/clark-tui-product.mjs --out target/clark-tui-product/current.json
node harness/clark-tui-product.mjs --require-complete
```

The deterministic contract currently records all 10 capability groups as
implemented. `conversation-cloud` is the shared native client used by Desktop,
TUI, and headless Clark. Both authentication surfaces persist the same
account-scoped server rows and the same `agent-core::Snapshot`; the shared core
owns legacy migration, typed resume transcripts, goal/plan continuity, and title
derivation. The CLI has no local JSON session database. Its API-key route reuses
the service's existing Desktop conversation handlers, optimistic revisions,
specialist-binding validation, and deletion fences rather than creating a second
history authority.

The isolated Rust harness exercises the actual Clark composer, palette,
transcript, provider reducer, steering, permission, attachment, model, goal,
status, specialist-continuity, and workspace modules. Specialist continuity
checks cover all four paid products, server-resolved access, loud pre-execution
failure, and verified post-action synchronization receipts. The ownership probe
allows only generic `crossterm` and `ratatui` terminal libraries and rejects any
external TUI implementation marker in Clark CLI source.

## Focused contract simulations and microbenchmarks

### Quick Chat

Quick Chat deliberately separates repository-free work from project sessions
while retaining the normal Clark cloud conversation and snapshot path. Its
deterministic lanes are:

```bash
cd app && pnpm exec vitest run src/store/sessionStore.quickChat.spec.ts src/lib/projectSidebar.spec.ts
cargo test -p provider-local --test quick_chat_workspace
cargo test -p clark-desktop quick_chat_workspace_is_stable_and_confined --lib
```

These cover frontend allocation and binding without changing the remembered
project, UUID/path confinement and stable reopen, a provider session whose
checkout and document root are the same marked non-Git directory, Quick Chat
sidebar grouping, and recognition of the same conversation ID under a
different device home-directory prefix. They do not prove a model can use the
workspace or that a cloud-synced conversation has reopened on a second
packaged device.

The paid evaluation is ignored by default and hard-locked to the exact
`qwen/qwen3.7-flash` route:

```bash
CLARK_CODE_LIVE=1 \
CLARK_CODE_PROVIDER=clark-platform \
CLARK_CODE_BASE_URL=... \
CLARK_CODE_MODEL=qwen/qwen3.7-flash \
CLARK_CODE_API_KEY=... \
cargo test -p provider-local --test live_clark_code \
  live_qwen_37_flash_quick_chat_paid_evaluation -- --ignored --exact --nocapture
```

Run it only after the user authorizes that exact model and credential-bearing
call. It performs one bounded scenario with at most six model iterations: in a
marked workspace with no `.git`, the model must write an exact sentinel, read
it back, and return an exact final receipt. The grader independently checks the
file bytes, required read/write tools, clean run completion, absence of `.git`,
positive token usage, and positive provider-reported cost. The no-repository
fixture and exact final/file sentinels are negative controls against silently
falling back to a project or grading prose as execution. There is one attempt
per authorized sample; stop and retain the first provider, tool, cost, or
oracle failure. A green run proves only this model/route/scenario sample, not a
packaged Desktop journey, cloud cross-device reopen, or general model quality.
Retain stdout plus the test binary revision in a new versioned
`target/quick-chat-paid/<timestamp>/` receipt and promote any durable conclusion
back into this catalog.

| Harness | Command | What it proves |
| --- | --- | --- |
| Attachment boundary | `cargo test -p clark-desktop --test attachment_benchmark -- --nocapture` plus `cargo test -p code-worker -p provider-remote-worker` | Large-paste expansion, text/binary handling, two-image batching, worker event translation, and confined project execution. `harness/attachment-smoke.mjs` covers the product surface; packaged SSH remains a separate release receipt. |
| Lossless accepted media | `cd app && pnpm test --run src/lib/attachments.spec.ts` and `cargo test -p provider-local --lib attachments::tests:: -- --nocapture` | Accepted images retain their original filename, MIME type, size, and bytes instead of being resized or lossily re-encoded. Extracted PDF/DOCX and resumed text attachment content is complete. Oversize or unsupported inputs fail explicitly. |
| Document and PDF receipts | `cargo test -p provider-local --lib tools::document::tests:: -- --nocapture`, `cargo test -p clark-desktop markdown_export::tests::`, and `cargo test -p clark-desktop document_preview::tests::` | The pinned `libreoffice-pure` 0.5.4 engine produces tagged PDF/real DOCX outputs and office/PDF previews without system LibreOffice. Conversion results carry typed path, MIME, exact byte count, and SHA-256 receipts; configured size/page limits reject instead of truncating. |
| Compaction summary coverage | `cargo test -p provider-local --lib compaction:: -- --nocapture` | Summary-plus-raw-tail checkpointing includes every replaced source message and the complete readable reasoning text. Opaque signed/encrypted replay payloads remain excluded; a complete request rejected by the provider leaves raw history intact instead of installing a partial summary. |
| Free-pool retry and transport identity | `cargo test -p provider-local --lib llm::retry::tests::qwen_flash_survives_eight_consecutive_free_pool_rate_limits -- --exact` and `cargo test -p provider-local --lib llm::retry::tests::successful_turn_maps_cache_and_provider_request_identities -- --exact` | A Qwen Flash logical turn survives eight consecutive pre-output 429s, keeps one idempotency key and session ID, honors zero-delay test receipts, and records attempt/retry, HTTP, provider, generation, and cache identities without a live call. |
| Eval proxy affinity | `cargo test -p provider-local --example planning_eval gateway::proxy::tests::forward_preserves_turn_identity_and_response_receipts -- --exact` | The planning evidence proxy preserves `Idempotency-Key` and `x-session-id` upstream and returns provider/cache receipt headers to the production model client. |
| Unlimited batched turns | `cargo test -p provider-local --test batched_turns -- --nocapture` | Twelve mutating calls emitted in one assistant turn all execute; exclusive workspace tools remain sequential in model emission order and every result reaches the next model turn. |
| Lossless capability discovery | `cargo test -p provider-local --lib tools::deferred::tests:: -- --nocapture` | Deferred tool search returns every match beyond the former 12-tool activation ceiling and preserves complete tool descriptions. |
| Migration parity | `cargo test -p provider-local --test migration_eval -- --nocapture` | Claude and OpenAI instructions, MCP, and skills discover identically through local and remote executors. |
| Worktree safety | `cargo test -p provider-local --test worktree_simulation` | Linked/detached worktree identity, dirty-state isolation, hostile helper nonexecution, scoped checkpoints/edits, and scripted provider behavior. |
| Desktop Git journey simulation | `pnpm exec vitest run src/lib/fakeGitRepository.spec.ts` and `cargo test --manifest-path src-tauri/Cargo.toml project_worktree --lib` | Focused pure routing cases on each UI/native boundary, plus real temporary Git worktrees; proves dirty changes stay put, owned branches open their checkout, managed branches stay pinned, and detached commits require save-before-cleanup. |
| Sandbox benchmark | `cargo run -p exec-sandbox --example sandbox_benchmark -- --iterations 1000 --launch-iterations 20` | Policy compilation for macOS/Linux/Windows backends and, when available, native inside/outside-write containment plus launch latency. |
| Temporal Atlas wire scale | `cargo run -p scout-ingest-protocol --example temporal_atlas_benchmark -- 10000` | Deterministic six-membership-per-service overlay build, serialization, round trip, bytes, latency, and semantic SHA-256 with zero model calls. |

The 2026-08-01 compaction regression reproduced two independent losses before
the repair: a deliberately small compaction-request budget omitted the two
oldest source messages, and a 6,400-character readable reasoning record was
cut at the adapter's former 2,000-character excerpt limit. After the repair,
all ten deterministic `compaction::` tests passed. This proves request
construction, checkpoint reuse, raw-tail pairing, and manual-compaction
lifecycle locally; it is not a live-provider context-window result.

The same audit exercised the custom PDF stack at its package boundaries.
`pdfer_forms` 0.3.4 fixes recursive/indirect field trees, radio-state order,
flatten-name collisions, and duplicate reattachment; its 16-test suite and a
nine-document, 1,011-field corpus passed before publication. `pdfsink-rs`
0.2.16 fixes independent-subpath rendering and cubic-curve extrema; its
159-test post-merge suite passed before publication. Desktop continues to pin
`libreoffice-pure` 0.5.4, whose 42-test suite passed with collision-safe
parallel fixtures. The sibling Clark workspace pins the two new public crate
versions and its `clark-pdf` suite passed all 40 tests.

The 2026-08-01 browser journey additionally replayed a dirty source checkout
with the deterministic preview switch `?fakeGit=modified`. The retained
screenshots in `../clark/.test-ui-screenshots/dirty-fresh-main-04-visible-dirty-warning.png`,
`dirty-fresh-main-06-compact-confirm.png`, and
`dirty-fresh-main-07-post-start-receipt.png` show the visible working-tree
warning, explicit default-branch confirmation, sibling `clark/session-*`
identity, and the source-preservation receipt. This is browser-fixture
evidence only; it is not a paid model or real-SSH run.

## Adjacent Clark platform eval framework

The broad agent scenario corpus is owned by the sibling Clark repository, not
Clark Desktop. Its authority is `evals/README.md` in that repository (normally
`../clark/evals/README.md` when the checkouts are siblings).
It includes the Rust `clark-eval` runner, smoke and message-result gates,
long-horizon autoresearch, tool/task/memory expectations, LLM judging,
simulated SaaS/browser environments, public datasets, labor-index work, and
trajectory/replay artifacts. Do not copy its scenarios or report its live
results from this desktop catalog. Run and document them in that repository;
use this section only to keep ownership boundaries clear.

## Maintenance checklist

When adding or materially changing an eval:

1. Add or update its row in **Current map**.
2. State whether it is deterministic, scripted/reference, live provider,
   packaged product, or guest-VM evidence.
3. Document exact model/provider gates and cost/retry limits for live calls.
4. Record the scenario and lane catalog, hidden/public boundary, repetition
   rule, grader/judge contract, negative controls, and stop conditions.
5. Retain versioned machine receipts, raw trajectories, first failures, model
   identity, usage/cost telemetry, and source/fixture hashes.
6. Put durable conclusions in a tracked report; never rely solely on `target/`
   or `/tmp`.
7. Record known invalid runs and why they are invalid. Do not delete evidence
   that explains a changed protocol.
8. Update this file after the run, including limitations and the strongest
   claim the evidence does and does not support.
