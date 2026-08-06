# AGENTS.md

Guidance for agents (and humans) working in this repo.

## What this is

Clark Desktop — a Tauri 2 + React 19 + Rust desktop client for agentic work.
One UI talks to many agent backends through a single `Provider` trait in
`agent-core`. Clean-room: no code from the main Clark repository is copied in;
shared behavior is reimplemented against provider contracts.

## Critical safety rules (concurrent agents)

Multiple agents operate on this repo simultaneously. Violating these rules
destroys other agents' work with no recovery path.

- Work on the current branch. **Never** create branches or worktrees.
- **Never use `git stash`.** Commit WIP directly to the current branch when
  asked. No safe invocation exists (not `--quiet`, not inside `$()`, not with
  `; echo skipped`). Same applies to `git clean`, `git reflog expire`, and
  `git gc --prune`.
- **Never revert working-tree files** — no `git checkout`/`git restore`/
  `git reset --hard` over files you didn't change. Other agents' uncommitted
  changes are intentional in-progress work.
- **Never restore old code over a concurrent migration.** If another agent has
  moved a file, interface, or flow to a new shape, do not put the old version
  back to unstick your task. Stop, surface the conflict, and ask.
- **Never commit unless explicitly asked.** When asked, stage only the
  specific files you changed — never `git add -A` or `git commit -a`.
- **No write-mode formatters across the tree** (`cargo fmt --all`,
  `prettier --write`, `eslint --fix`) — repo-wide reformat diffs collide with
  concurrent work. Format only the files you touched. The check-only
  `cargo fmt --all --check` in Commands is fine.
- **If compilation breaks in files you didn't modify**: don't touch them.
  Build only the crate you need, or ask — another agent is mid-refactor.
- **Trust Edit/Write results — don't revert "to verify" or "establish a
  baseline".** Use Read for disk state and read-only `git diff` for changes.
  A failing test means fix the code, fix/delete the test, or confirm it's
  pre-existing — never stash-and-rerun.
- **Live-model tests cost real money.** `live_clark_code` (and anything else
  that calls a provider) is env-gated and ignored by default; run it only when
  the user explicitly asks, with the model and key they name.
- **When in doubt, ask instead of acting.** Pausing costs seconds; a
  destructive command costs hours of recovery.

## Engineering judgment

Instructions encode an intent; serve the intent, not the literal command past
its premise.

- When the data says the intent is unreachable (a hung build, an eval pinned
  at 0%, an empty query), the instruction is moot — stop, report, ask.
- Surface bad news early. A predictable failure at a 10% sample is a finding
  now; it does not get more useful at 100%.
- Question your own actions as they run: if three iterations confirm the same
  thing, the work is done — pivot or stop.
- Match scope to the actual problem. A bug fix doesn't need a refactor; a
  one-line fix doesn't need new abstractions.
- Delete or rename stale concepts at each boundary instead of leaving
  compatibility aliases. When an authority or flow is replaced, remove its
  obsolete names, contracts, tests, harnesses, and documentation as callers
  migrate so the repository describes one current architecture.
- Cost-awareness is part of the job: model calls, long contexts, and user
  attention are engineering constraints, not free resources.
- When you ask, ask with a recommendation ("X looks broken — kill and dig in,
  or let it finish?") — own the judgment call you're best positioned to make.

## Subagent reports are leads, not sources

A subagent's return text is a condensed summary written from memory — treat it
like a colleague's verbal description of code: useful for orientation, never
authority. Don't quote, claim, or act on its description of file contents
without opening the file yourself; if you can't point to a Read result you
produced this session, you're paraphrasing. When a file read contradicts a
subagent summary, the file wins — retract and rebuild from the file. Subagents
are for breadth (locating files, surveying conventions); read the specific
files you're about to make claims about yourself.

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
| `EVALS.md` | Repository-wide eval/simulation catalog, current retained evidence, commands, claim boundaries, and known invalid or incomplete runs. |

## Evaluations and simulations

Read `EVALS.md` before designing, running, changing, or interpreting an eval.
It is the routing source for planning/context, memory, goals, orchestration,
Scout, skills, security, resilience, VM/product, and focused contract
simulations. Keep scripted/reference, live-model, packaged-product, and guest-VM
evidence separate. Never promote ignored `target/` or disposable `/tmp`
artifacts into durable claims without a tracked conclusion, and update
`EVALS.md` whenever an eval contract or authoritative result changes.

For Clark Desktop release requests, "full evals", "full sims", and similar
language means the release-relevant lanes cataloged in this repository's
`EVALS.md`, including its explicitly configured paid release benchmark. It does
not authorize running the main Clark repository's broad scenario corpus or any
other sibling repository's eval suite. Those cross-repository, high-volume, or
otherwise non-release-blocking paid runs require a separate explicit request;
they must not delay or gate a Desktop release.

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
./script/build_and_run.sh
```

On macOS, always use this launcher instead of opening a raw debug bundle. It
assigns the separate `Clark Code Dev` identity and applies a stable development
signature so TCC privacy grants survive rebuilds.

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
- **Prefer refactoring** existing code over adapter shims or compatibility
  layers. Update callers when changing signatures — don't add optional params
  or wrappers to preserve old call sites. Delete dead code rather than
  commenting it out. Fix all call sites when touching shared interfaces
  (grep for usages).
- **File size: soft limit 500 lines, hard limit 800.** At 500, split before
  adding code; at 800, extract a submodule first. Rust and TypeScript.
- **Order is part of the prompt.** Tool-call schemas in `provider-local` are
  consumed autoregressively — the model generates arguments in the advertised
  property order — so schemas are authored locate-before-payload (`edit_file`:
  `path → old_string → new_string`), decide-before-write (`memory`:
  `action → scope → title → content`), and rationale-first (`update_plan`:
  `explanation → plan`). serde_json's `preserve_order` feature in the workspace
  `Cargo.toml` is load-bearing (without it schemas alphabetize on the wire) and
  `schema_property_order_survives_serialization` in `tools/mod.rs` pins the
  order. When adding a tool, order properties by what the model should commit
  to first, and add the tool to that test if the order carries semantics. The
  same thinking applies to the system prompt (`prompt.rs`): hard rules go
  first (primacy), volatile per-turn facts go in the turn message (recency).

## Root-cause debugging

When debugging provider, agent-loop, or UI-stall behavior, identify the first
contract break before patching the visible symptom.

- Reconstruct the timeline from records — persisted `AgentEvent`s, tool
  results, provider request/response logs — to find the first bad transition.
  Don't infer root cause from the last visible UI state alone.
- Fix the broken boundary, not the echo. Separate symptom mitigation from the
  root fix, say which one you changed, and prove it with the smallest targeted
  test or replay.
- Use current upstream contracts: for Tauri, provider APIs, or SDK behavior,
  verify live docs or local source over remembered parameter support. If docs
  conflict with observed behavior, the observed failing path is authoritative —
  document the mismatch.
