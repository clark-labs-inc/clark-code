# Clark Code

<p align="center">
  <img src="docs/clark-desktop-demo.gif" alt="Clark Desktop running an agent task end to end: web research, a live plan, file edits, and tool calls that build and publish a site" width="820">
  <br/>
  <em>One Clark run in the desktop app: web research → a live plan → file edits → tool calls that build and publish a site.</em>
</p>

Private, cross-platform desktop client for agentic work. One UI that talks to
many agent backends through a single provider abstraction — **ACP** local CLI
agents and the **Clark** runtime — with the
performance-critical engine written once in Rust and shared across desktop,
mobile, and web.

## Why

- **Most performant.** Tauri 2 (native WebView, ~10MB binaries, ~50% less RAM
  than Electron) + a Rust core. No bundled Chromium.
- **One engine, three targets.** Transport codec, event projection, and run
  lifecycle live once in the `agent-core` Rust crate and compile to native
  (desktop/mobile) and WASM (web) — instead of being re-implemented per platform.
- **Agent-agnostic, Clark-focused.** A trait-based `Provider` abstraction fronts
  every backend. ACP-first; the Clark adapter rides the same trait.
- **Beautiful & complete.** React 19 + Tailwind v4 UI: streaming chat, tool-call
  timeline, plan, permission gates, and an agent "computer" surface.

## Architecture at a glance

```
React (Tauri WebView)  ──invoke/emit──►  agent-core (Rust, native + WASM)
   surfaces, store            │              domain · projection · Provider trait
                              ▼
                  provider-acp (JSON-RPC/stdio)   provider-clark (HTTP cmds + SSE)
                  local CLI agents via sidecar     remote Clark runtime
```

## Autoregressive prompt & schema design

LLMs generate left-to-right, so *order is part of the prompt*. The local coding
agent (`crates/provider-local`) is designed around that:

- **Tool schemas are ordered prompts.** The model emits tool-call arguments in
  the property order the schema advertises, so every schema is authored
  locate-before-payload (`edit_file`: `path → old_string → new_string`),
  decide-before-write (`memory`: `action → scope → title → content`), and
  rationale-first (`update_plan`: `explanation → plan`). The workspace enables
  serde_json's `preserve_order` feature so authored order actually reaches the
  wire — stock serde_json alphabetizes `json!{}` maps, which had silently
  reversed `edit_file` into `new_string → old_string → path`. A regression
  test pins the wire order of the sensitive schemas.
- **The system prompt is position-aware.** Hard rules (the shared-worktree git
  rules) sit in the primacy slot directly after the identity line; volatile
  per-turn facts (a fresh `git status` snapshot, output style) are injected at
  the recency end of each turn message — which also keeps the cached system
  prompt prefix stable for prompt caching.

## Layout

| Path | What |
| --- | --- |
| `crates/agent-core` | Domain model, projection reducers, `Provider` trait, codecs. Native + WASM. |
| `crates/provider-acp` | Agent Client Protocol adapter (JSON-RPC over stdio). |
| `crates/provider-clark` | Clark runtime adapter: HTTP command writes (`/api/conversation-sync/commands`) + resumable SSE event stream, with a WS for realtime session binding. |
| `crates/provider-local` | Local coding loop with brokered Clark Cloud tools and a default-on native command sandbox. |
| `crates/exec-sandbox*` | Cross-platform policy, Seatbelt/bubblewrap adapters, and the Windows restricted-token privilege boundary. See [sandbox design](docs/sandboxing.md). |
| `crates/devbridge` | Dev-only WebSocket bridge that drives the real providers + projection from a browser (headless UI testing, video capture). Not shipped. |
| `src-tauri` | Tauri 2 host: commands, event bridge, sidecar, state. |
| `app` | Vite + React + TS + Tailwind v4 frontend. |
| `harness` | Playwright scripts for local smoke runs, diagnostics, and screen capture against the running app. |

## Develop

Prerequisites: Rust (stable), Node 24+, pnpm 10+, and the
[Tauri 2 system deps](https://v2.tauri.app/start/prerequisites/).

```bash
# Rust engine: test + lint
cargo test -p agent-core -p provider-acp -p provider-clark
cargo clippy -p agent-core -p provider-acp -p provider-clark -p devbridge --all-targets -- -D warnings
cargo fmt --all --check

# Frontend (browser preview uses a mock provider)
cd app && pnpm install && pnpm dev      # http://localhost:1420
pnpm test                               # vitest
pnpm typecheck

# Run the desktop app (spawns the Vite dev server automatically)
cargo tauri dev
```

In a plain browser the UI runs against a **mock provider** that plays a scripted
streaming run, so every surface is demonstrable without the native host.

## Resilience benchmark

The Playwright resilience benchmark drives the real Clark Code conversation UI
through every combination of six independent conditions: rate limiting,
duplicated provider tool-call IDs, event-stream disconnects, provider-process
loss, delayed cloud-history acknowledgment, and explicit user cancellation.
That is a deterministic 64-case power set, not 64 paid model calls. Each case
asserts that the conversation stays rendered, internal tool-call IDs remain
hidden, incident state settles, and interrupted/cancelled work can continue
from saved progress. Screenshots and a JSON receipt are written to a temporary
artifact directory printed at the end of the run.

```bash
cd harness
node resilience-benchmark.mjs
```

A separate control drives the real local provider through `devbridge` using
the Clark-managed `clark-code:deepseek_v4_pro` route. It is intentionally
opt-in because it spends live model credits. Set `CLARK_CODE_API_KEY` (or keep
it in the repository's gitignored `.env`) and run:

```bash
cd harness
node resilience-benchmark.mjs --live-only
```

The shared contract in
`app/src/core-bridge/resilienceBenchmark.json` pins the matrix dimensions,
provider, and model for both the TypeScript tests and Playwright harness so the
reported configuration cannot drift from the simulated one.

## Repository Status

Private Clark repo. Clean-room: no code from the main Clark repository is
copied into this client; shared behavior is reimplemented against provider
contracts.
