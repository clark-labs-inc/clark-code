# Agent Desktop

Agent Desktop is an Apache-2.0 desktop foundation for agentic work. It combines
a Tauri 2 host, React 19 interface, provider-neutral event model, local coding
agent, ACP adapter, sandboxed execution, resumable sessions, and extension
contracts for branded distributions.

The repository intentionally contains no downstream billing policy, hosted-provider
transport, account entitlements, private research tools, specialist catalogs,
release authority, or branded updater configuration. Those concerns belong to
a separate product composition that consumes this foundation.

## Architecture

| Path | Responsibility |
| --- | --- |
| `crates/agent-core` | Backend-neutral domain model, provider contract, codecs, and pure projections |
| `crates/provider-acp` | Agent Client Protocol adapter |
| `crates/provider-local` | Sandboxed local coding agent and product-neutral tool-pack extension seam |
| `crates/exec-*` | Cross-platform process isolation and execution protocol |
| `src-tauri` | Native lifecycle, session registry, IPC, and `ProductIntegration` composition boundary |
| `app` | Reusable React shell and product module boundary |
| `harness` | Deterministic foundation checks |

The native product boundary is documented in
[`docs/product-boundary.md`](docs/product-boundary.md). A downstream product can
add providers, prepare native-only credentials and configuration, implement
opaque product requests, install tool packs, supply frontend slots and access
policy, and own its branding and packaging without placing those details in
this repository.

## Development

Rust checks:

```bash
cargo fmt --all --check
cargo clippy -p agent-core -p provider-acp -p provider-local -p devbridge --all-targets -- -D warnings
cargo test -p agent-core -p provider-acp -p provider-local
cargo check -p agent-core --target wasm32-unknown-unknown
```

Frontend checks:

```bash
cd app
pnpm install --frozen-lockfile
pnpm typecheck
pnpm test
pnpm build
```

Run the neutral development host on macOS with:

```bash
./script/build_and_run.sh
```

## Product boundary

Public code may define stable capabilities and extension interfaces. A branded
product must privately own:

- billing and entitlement decisions;
- hosted service URLs, credentials, and account policy;
- proprietary providers, research brokers, advisors, and specialist workflows;
- first-party catalogs and product-specific interface copy;
- bundle identifiers, deep links, signing, updater keys, release workflows,
  and deployment receipts.

The renderer receives only generic projections such as capability availability
and safe actions. It never reconstructs subscription rules or receives service
credentials.

## License

Apache-2.0. Downstream proprietary compositions remain separate works and are
not included in this repository.
