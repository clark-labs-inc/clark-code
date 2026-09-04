# Latest-build developer dogfood journal

- Date: 2026-08-19 (PDT)
- Source: `main` at `b9c60c10a0d005b2df274b6649b6afdc252cfe8d`
- Freshness: `origin/main` resolved to the same commit
- Routing: no `FEATURES.md` exists in this foundation checkout; coverage was derived from `EVALS.md`, `README.md`, current source, and the repository harness catalog.
- Scope: build the current checkout, run deterministic repository gates, launch the unsigned local desktop app, and exercise the discoverable UI/browser harness workflows.

## Receipts

| Check | Result | Evidence / notes |
| --- | --- | --- |
| Dependency install | PASS | `pnpm install --frozen-lockfile` in `app/` and `harness/` |
| Frontend production build | PASS | `pnpm build` in `app/`; Vite built 5,271 modules |
| Frontend unit suite | PASS | `pnpm test -- --reporter=dot`; 824 passed, 2 skipped, 1 skipped file |
| Rust formatting | PASS | `cargo fmt --all --check` |
| Rust lint | PASS | `cargo clippy -p agent-core -p provider-acp -p provider-local -p devbridge --all-targets -- -D warnings` |
| WASM boundary | PASS | `cargo check -p agent-core --target wasm32-unknown-unknown` |
| Open-source boundary | PASS | `node --test harness/product-boundary.spec.mjs`; 13/13 |
| Full deterministic Rust workspace | PASS | `cargo nextest run --workspace -E 'not test(attachment_benchmark_local)' --no-fail-fast`; nextest run `15c5ec98-3d55-4828-88dd-7bf1101ed051`; 1,373 passed, 9 skipped |
| Unsigned Tauri dev build | PASS | `./script/build_and_run.sh`; compiled and ran `target/debug/clark-code` with `debug-diagnostics`; stopped cleanly after smoke validation |

## Browser and native dogfood

All browser journeys used the repository's mock-provider or UI-only harnesses. No paid or hosted-model calls were made.

| Journey | Result | Evidence / coverage |
| --- | --- | --- |
| WebKit startup | PASS | Empty and restored profiles; `pnpm test:webkit` |
| Attachments | PASS | 11 checks: image admission, large paste, atomic clear, send/settle; `/tmp/agent-desktop-attachment-smoke.png` |
| Model picker | PASS | Portal, compact bounds, pointer selection in both directions; `target/model-picker-smoke/20260820T011148Z-6936/` |
| SSH settings | PASS | Save host without folder, edit/focus, persistence, execution-target selection; `target/ssh-settings-smoke/20260820T011148Z-6918/` |
| Drag/drop and file picker | PASS | Pointer reorder, keyboard/menu reorder, focus restore, external drop, picker alternative; `target/pragmatic-dnd-smoke/20260820T011148Z-6942/` |
| Artifact delivery | PASS | Inline SVG, save-copy download, workspace, real PDF rasterization and PDF download; `target/artifact-delivery-smoke/receipt.json` |
| Full GUI | PASS | Multi-turn chat, permission, goals/steering/completion/clear, 12 slash commands, terminal/MCP/memory/compact, side question, artifacts, mobile overflow; `target/full-gui-smoke/20260820T011300Z-10560/` |
| Resilience matrix | PASS | Clean, recoverable transport faults, all recoverable faults, provider loss pause, upstream+process pause, explicit cancel; 6/6 cases; `target/resilience-smoke/20260820T011204Z-7354/` |
| Specialist matrix | PASS | Scout, Security, RSI catalog/canvas/start-failure/running/settlement/detach-reattach/mobile/access gates; 23 checks; `target/specialist-matrix-smoke/20260820T011204Z-7366/` |
| Sidebar resize | PASS | Drag, keyboard bounds, reload persistence, collapse/expand, double-click reset; probe output ended `sidebar-resize-probe: PASS` |
| Text selection clearing | PASS | Chromium mid-stream and settled selection cleared on empty-area clicks; `selection-repro.mjs chromium` |
| Chat-switch profile | PASS (exploratory) | 4 warm switches at 17–65 ms with 10 heavy transcript turns; `/tmp/agent-profile/results.json` |

## Failed or blocked journeys

| Journey | Classification | First failing boundary / evidence |
| --- | --- | --- |
| Motion probe, reduced mode | Harness contract mismatch | Full-motion Chromium/WebKit rows pass. Reduced rows report failure because the probe requires a 0 ms toast transition, while the current source CSS and unit contract intentionally use a 120 ms opacity-only reduced-motion fade. Permission-gate fade and no-spatial-motion checks otherwise pass. |

The first concurrent run of artifact and full-GUI journeys also saw Vite `504 Outdated Optimize Dep` responses before React mounted. Sequential retries removed that dependency-optimizer contention and both passed.

## Findings

- The current source is buildable and the broad deterministic and browser mock-provider surface is healthy.
- The reduced-motion probe should be aligned with the current low-motion contract (`opacity` fade at the fast duration) instead of asserting zero transition duration.

## Cleanup

- Stopped the unsigned Tauri dev process with Ctrl-C; verified no task-owned `target/debug/clark-code` or app Vite process remained.
- Browser contexts and temporary Vite servers were closed by each harness.
- No source files other than this journal were changed; no commit or push was performed.
