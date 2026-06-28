# AGENTS.md

Guidance for agents (and humans) working in this repo.

## What this is

Clark Desktop — a Tauri 2 + React 19 + Rust desktop client for agentic work.
One UI talks to many agent backends through a single `Provider` trait in
`agent-core`. Clean-room: no code from the main Clark repository is copied in;
shared behavior is reimplemented against provider contracts.

## Layout

| Path | What |
| --- | --- |
| `crates/agent-core` | Domain model, projection reducers, `Provider` trait, codecs. Native + WASM. |
| `crates/provider-acp` | Agent Client Protocol adapter (JSON-RPC over stdio). |
| `crates/provider-clark` | Clark runtime adapter: HTTP command writes + resumable SSE event stream, WS for realtime session binding. |
| `crates/provider-local` | Local coding agent (OpenCode-style): an OpenAI-compatible tool-calling loop that runs file/shell tools locally (read-before-edit invariant, project-root sandbox), delegates research to Clark's sandbox via a `clark_research` tool, and keeps a per-repo memory under `<root>/.clark/memory/` that Clark can auto-extract. |
| `crates/devbridge` | Dev-only WebSocket bridge driving real providers from a browser. Not shipped. |
| `src-tauri` | Tauri 2 host: commands, event bridge, sidecar, state. |
| `app` | Vite + React + TS + Tailwind v4 frontend. |
| `harness` | Playwright scripts for local smoke runs, diagnostics, screen capture. |

## Commands

Run these before considering work done.

### Rust

```bash
cargo fmt --all --check
cargo clippy -p agent-core -p provider-acp -p provider-clark -p provider-local -p devbridge --all-targets -- -D warnings
cargo test -p agent-core -p provider-acp -p provider-clark -p provider-local
```

`provider-clark` live tests are ignored unless `CLARK_WS_URL` is set; they still
compile under `--all-targets`.

`agent-core` also builds for WASM:

```bash
cargo check -p agent-core --target wasm32-unknown-unknown
```

### Frontend (run inside `app/`)

```bash
pnpm install --frozen-lockfile
pnpm typecheck
pnpm test        # vitest
pnpm build
```

### Run the desktop app

```bash
cargo tauri dev   # spawns the Vite dev server automatically
```

## Conventions

- **Provider abstraction.** Every backend implements `agent_core::Provider`.
  New backends go in `crates/provider-*` and are wired through `devbridge` and
  `src-tauri` state. Keep `agent-core` backend-agnostic and WASM-clean.
- **Projection is pure.** `agent_core::projection::apply` is a pure reducer
  over `AgentEvent`. No I/O, no async. Add reducer tests for new event shapes.
- **Tests required.** New translate/projection behavior gets a unit test
  (mirror the existing `mod tests` blocks). CI enforces fmt + clippy
  (`-D warnings`) + tests for the crates above.
- **Secrets/env.** `.env`, `.env.*`, and `*.local` are gitignored. Never commit
  tokens; only `.env.example` templates are tracked.

## Workflow rules

- Work on the current branch. **Never** create branches or worktrees.
- **Never** commit unless explicitly asked. Commit WIP directly to the current
  branch when asked; do not use stash or branches to fragment shared state.
- Match existing code style; do not add comments unless asked.
