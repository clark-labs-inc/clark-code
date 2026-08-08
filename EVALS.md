# Foundation evaluations, benchmarks, and simulations

Last source-and-artifact audit: 2026-08-07.

This public catalog covers only the open desktop foundation. Branded products
own provider routes, paid models, billing, service topology, product catalogs,
release identities, and retained production receipts in their composition
repositories.

## Evidence rules

1. Unit and integration tests prove only their named contracts.
2. Scripted simulations prove harness mechanics, not model quality or live use.
3. Live provider tests require explicit authorization for the exact model and
   route; no live provider test is part of the default foundation gate.
4. Host checks do not prove a packaged product or guest-VM journey.
5. Ignored `target/` and disposable `/tmp` artifacts are not durable evidence.
6. Keep typed receipts and first failures; never infer success from UI state or
   a process exit alone.

## Current map

| Surface | Entrypoint | Evidence class | Claim boundary |
| --- | --- | --- | --- |
| Public/private boundary | `node --test harness/product-boundary.spec.mjs` | deterministic repository-wide text, dependency, and metadata contract | Proves no downstream provider/CLI package, billing policy, first-party specialist catalog/command surface, release authority, hosted transport, product model policy, or private research/advisor implementation remains in project-controlled public source, tests, documentation, fixtures, scripts, or configuration |
| Core domain and providers | `cargo test -p agent-core -p provider-acp -p provider-local` | deterministic Rust contracts | Proves provider-neutral projection, ACP translation, and local loop/tool behavior |
| Native host | `cargo test -p desktop-foundation --lib` | deterministic native command contracts | Does not prove a packaged branded product |
| WASM core | `cargo check -p agent-core --target wasm32-unknown-unknown` | compile contract | Proves the domain crate remains WASM-clean |
| Frontend | `pnpm --dir app typecheck`, `pnpm --dir app test`, `pnpm --dir app build` | deterministic TypeScript, component, and bundle contracts | Neutral build only; product entries have their own build receipt |
| Local sandbox | `cargo test -p exec-sandbox` | deterministic policy and platform-adapter contracts | A packaged/signature receipt remains product-owned |
| Durable worker | `cargo test -p code-host -p code-worker -p code-remote -p provider-remote-worker` | deterministic protocol and confinement contracts | Ignored live SSH lanes require explicit authorization |
| Orchestration | provider-local orchestration benchmark tests | deterministic fixture contracts | Does not claim live-model quality |
| Memory and goals | provider-local memory and goal eval tests | deterministic fixture contracts | Live-model quality is separate and opt-in |

## Product boundary

The allowed dependency direction and extension contracts are documented in
`docs/product-boundary.md`. A branded product must independently verify:

- its pinned foundation revision;
- provider and tool-pack adapters;
- server-authored product access projections;
- branded frontend entry and Tauri context;
- signing, updater, deep-link, sidecar, and packaged runtime receipts.

Changing a foundation eval contract or authoritative result requires updating
this file. Product-only evidence must not be copied into this public catalog.
