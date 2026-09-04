# Latest-build Clark Code QA report

- Date: 2026-08-21 (PDT)
- Foundation source: `main` at `43b23f0c4c1fd25ea659cfa1d818cc38620c2475` (clean)
- Product source observed: `/Users/stan/Documents/git/clark` at `5bab7904218b5ad3d3f9be3dc1b60bd625af068a` (dirty with concurrent changes; QA did not modify or stage it)
- Scope: latest local foundation build, deterministic feature matrix, UI/browser journeys, one bounded paid boundary run, simulated Git/worktree flows, and read-only SSH reachability for `scl`.
- Billing safety: no new account was created and no unlimited account was attached. The existing configured paid credential was used for one bounded first-turn run only; further paid calls were stopped after the account reported exhausted credits.

## Executive result

The foundation build and deterministic desktop matrix are healthy. The real paid path is not currently release-acceptable: preflight authorized the backend-owned `clark_max` tier, but the first fresh conversation was dispatched to `deepseek/deepseek-v4-pro-0813` through OpenRouter and then failed with exhausted credits. Current product-backend refresh is also blocked before startup because the product lockfile is stale under `--locked`. Consequently, paid specialists, paid quick chats, live GitHub OAuth, and an actual remote-agent run on `scl` remain unvalidated.

## Build and deterministic checks

| Area | Result | Receipt / command |
| --- | --- | --- |
| Rust format and clippy | PASS | `cargo fmt --all --check`; clippy for `agent-core`, `provider-acp`, `provider-local`, `devbridge` with `-D warnings` |
| Rust tests | PASS | `cargo nextest run -p agent-core -p provider-acp -p provider-local`: 759 passed, 8 skipped |
| Tauri project/worktree tests | PASS | `cargo nextest run --manifest-path src-tauri/Cargo.toml project_worktree --lib`: 27 passed, 2 leaky, 110 skipped |
| Frontend typecheck/build | PASS | `pnpm --dir app typecheck`; `pnpm --dir app build` (5,271 modules) |
| Frontend unit tests | PASS | `pnpm --dir app test`: 829 passed, 2 skipped, 1 skipped file |
| Targeted branch/Git/SSH tests | PASS | 5 files, 17 tests: quick chat, project branches, fake Git repository, SSH settings, branch picker |
| Product UI check/test/build | PASS | `pnpm --dir clark-ui check`; `test` (1,911 passed, 1 skipped); `build` |
| Full GUI mock-provider journey | PASS | `target/full-gui-smoke/20260821T183237Z-11366/`; goals, slash commands, artifacts, responsive layout |
| Specialist mock matrix | PASS | `target/specialist-matrix-smoke/20260821T183237Z-11408/`; Scout, Security, RSI catalog/access/start/settlement/detach/mobile gates |
| SSH settings UI | PASS | `target/ssh-settings-smoke/20260821T183237Z-11437/` |
| Model picker and branch/project drag-drop | PASS | `target/model-picker-smoke/20260821T183237Z-11394/`; `target/pragmatic-dnd-smoke/20260821T183237Z-11380/` |
| Resilience, attachments, WebKit, product boundary | PASS | `target/resilience-smoke/20260821T183458Z-14052/`; `/tmp/agent-desktop-attachment-smoke.png`; `test:webkit`; `test:product-boundary` 13/13 |

These are mock-provider or UI-only checks unless explicitly called out below; they do not prove hosted-model behavior.

## Paid-run protocol and result

The reusable paid protocol is:

1. Run the environment preflight and save its receipt:
   `node crates/clark-desktop-product/harness/release-environment-preflight.mjs --require clark --out target/qa-20260821/preflight-clark.json`
2. Pin the intended tier and provider in the run (`Clark Max`, `clark_max`, `clark-platform`); record the resolved model from the backend receipt rather than a product alias.
3. Start one fresh conversation per lane and send one bounded sentinel prompt. Suggested lanes are quick chat, new project, new session, branch/worktree mutation, simulated GitHub repository, each specialist (Scout/Security/RSI), and read-only SSH `scl`.
4. Before sending a second turn, verify the gateway receipt contains the expected provider/model, generation/run ID, usage/cost, and terminal state. Capture the conversation ID, canonical event inspection JSON, screenshot, console/network result, and provider receipt.
5. Stop the matrix on a route mismatch, credit/admission error, or missing terminal receipt; do not retry blindly.

Preflight passed and recorded the expected Clark route in `crates/clark-desktop-product/target/qa-20260821/preflight-clark.json/receipt.json`. The bounded continuity run was conversation `0856c658-fc91-4db9-ae28-4b98322967ce`; its canonical inspection showed 15 events, one tool call, no tool result, and a failed lifecycle. Gateway evidence showed the backend-resolved `deepseek/deepseek-v4-pro-0813` / OpenRouter Alibaba route, followed by `live billing exhausted credits` and `Insufficient credits. Add credits or start a paid plan to continue.` No further paid calls were made.

## Failure and blocker catalog

| ID | Issue | First broken boundary | Evidence / classification |
| --- | --- | --- | --- |
| QA-PAID-001 | Intended Clark route is not the route actually dispatched; existing paid account has no usable credits | Gateway routing/billing admission before provider completion | Conversation `0856c658-fc91-4db9-ae28-4b98322967ce`; provider logs; product/runtime or environment configuration blocker |
| QA-SETUP-001 | Refreshing the current product checkout fails under `--locked` because `Cargo.lock` cannot be updated | Product prebuilt export during `make test-ui-setup` | `/tmp/clark-test-ui/runs/20260821-113602-16390/services-rebuild.log`; source manifests and lockfile are out of sync |
| QA-MATRIX-001 | Offline product feature-matrix contract is stale | Capability-map validation before feature lanes | `target/qa-20260821/feature-matrix-offline`; unmapped provider commands, workspace crates, and current live-model policy |
| QA-SEC-001 | Product security simulation invokes paths/workspaces that no longer exist | Harness setup, before security steps | `/tmp/clark-security-simulation/receipt-offline.json`; provider-local is outside the product workspace, product `app` and referenced harness paths are missing |
| QA-SPECIALIST-001 | Product specialist preview fails at RSI welcome/example assertion | Product specialist composition/harness assertion | `crates/clark-desktop-product/target/specialist-ui-smoke/20260821T182954Z-94861/receipt.json`; Scout and Security previews pass; no paid calls |
| QA-DOC-001 | Product EVALS command points the fake-Git test at the wrong checkout | Documentation/test routing | Foundation `app/src/lib/fakeGitRepository.spec.ts` is the live test; corrected foundation command passes |
| QA-TOOLCHAIN-001 | Product UI commands emit Node-engine warnings | Toolchain compatibility | Current Node `v25.9.0`; product declares `^22.22.0 || ^24.0.0 || >=26.0.0` |

## Remote host and repository coverage

`ssh -G scl` resolved to user `ubuntu`, host `160.211.68.57`, port 22, and `~/.ssh/id_ed25519`. A read-only `BatchMode` probe returned `/home/ubuntu` and Git 2.34.1. This proves transport and a shell/Git binary only; no project checkout, agent loop, write, or paid model call was run on `scl`.

Branch management, worktree creation, quick-chat state, new-session clearing, artifact delivery, and project reordering were covered deterministically and in the mock GUI. Live GitHub OAuth/remote mutation and hosted specialist/quick-chat execution are explicitly **not tested** because the paid route failed at the first boundary.

## Required follow-up

1. Repair the product lockfile/manifests contract, then rerun `make test-ui-setup` against the current product HEAD.
2. Reconcile the Clark tier's provider/model selection with the gateway route and restore a funded, explicitly scoped test credential before any new paid call.
3. Refresh the product feature matrix, security harness paths, specialist RSI assertion, and EVALS test routing.
4. Re-run the one-turn-per-lane paid protocol and record terminal receipts before claiming hosted specialists, GitHub operations, or `scl` agent work are release-ready.

No repository commits, pushes, releases, account creation, or secret changes were made by this QA pass.

## Post-fix verification

The issues above were traced to their owning contracts and corrected locally on
2026-08-21. The baseline observations remain preserved above; this section is
the authoritative post-fix status.

| ID | Post-fix status | Evidence |
| --- | --- | --- |
| QA-PAID-001 | Source contract fixed; live acceptance still blocked by billing | Clark Code no longer owns a model catalog. The compatibility selector resolves through the backend-owned `clark_max` tier, whose model, reasoning, and fallback order come from the Clark/DynamoDB snapshot. Core model-profile tests, 18 gateway contract tests, 17 provider-routing tests, and the specialist/provider contracts pass. No new paid call was made because the existing credential is exhausted. |
| QA-SETUP-001 | Resolved for current lockfile | Product `cargo metadata --locked --no-deps --format-version 1` succeeds; the product workspace can compile the paid-eval test target under `--locked`. |
| QA-MATRIX-001 | Resolved | Offline consolidated matrix: 9 passed, 0 failed, 1 explicitly skipped paid lane. Receipt: `crates/clark-desktop-product/target/qa-20260821/feature-matrix-after-contract-fix-v3/report.json`. |
| QA-SEC-001 | Resolved | Security simulation now uses the product workspace for product evals and the foundation workspace for foundation lanes; offline receipt is `/tmp/clark-security-simulation/receipt-offline.json` (`passed`). |
| QA-SPECIALIST-001 | Resolved | RSI is conversation-native and no longer incorrectly required to expose the Scout/Security insights canvas; example metadata has a stable QA selector. Receipt: `crates/clark-desktop-product/target/specialist-ui-smoke/20260821T185639Z-74566/receipt.json` (`passed`, no paid calls). |
| QA-DOC-001 | Resolved | Product `EVALS.md` now invokes the foundation fake-Git test from the correct checkout. |
| QA-TOOLCHAIN-001 | Open, non-blocking | Node 25 still emits the product's declared-engine warning; use the declared Node 22/24/26 lanes for release CI. |

The stale resilience lane was also corrected at the manifest boundary:
`benchmark:resilience:smoke` became the foundation's actual
`test:resilience` script. The lane passes all six recovery/cancellation cases;
receipt: `crates/clark-desktop-product/target/qa-20260821/resilience-contract-after-fix/report.json`.

Remaining release boundary: hosted paid specialists, live GitHub OAuth/remote
mutation, and a real agent run on `scl` still need one funded, explicitly
scoped acceptance credential. The source and offline contracts are now green;
they must not be represented as proof of hosted-provider success.
