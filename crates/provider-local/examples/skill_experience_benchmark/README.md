# Clark skill experience benchmark

This benchmark reproduces Read's Superpowers journey from two isolated,
initially empty user homes. It exercises production Clark catalog, executor,
managed-pack, typed-history, instruction, and provider boundaries. It does not
call a live model.

## What it proves

The local lane performs the following ordered journey:

1. Start with empty personal and project skill roots.
2. Reproduce legacy directory-symlink discovery.
3. Load personal, project, and nested instruction provenance.
4. Install the real Superpowers tree into Clark-managed user storage.
5. Preserve an intentionally-created project skill name collision.
6. Serialize and deserialize an exact typed skill binding.
7. Send that binding through `LocalAgentProvider` to a scripted
   OpenAI-compatible endpoint and inspect the actual outbound request.
8. Modify the source pack and install the new content revision.
9. Prove the active provider refreshes before the next run and rejects the old
   revision before making a model request.
10. Restart the provider and prove the updated skill body is loaded.
11. Uninstall the pack and prove its last binding is rejected while the
    independent colliding skill remains.

The remote lane repeats install, discovery, exact provider loading, update,
stale rejection, reconnect/restart, and uninstall through a real
`RemoteExecutor` connected to an in-process `clark-exec-server`. The server has
an explicit fake target home, which catches accidental desktop-home reads.

## Run against the real Superpowers repository

```bash
cargo run -p provider-local --example skill_experience_benchmark -- \
  --superpowers /path/to/obra/superpowers
```

The path can also be supplied through `CLARK_SUPERPOWERS_FIXTURE`. When neither
is provided, the runner checks the normal Codex plugin cache.

Use `--out /new/path` to choose the artifact directory. The runner refuses to
overwrite an existing directory. By default, it creates a unique directory
under `target/skill-experience-benchmark/`.

Each run writes:

- `receipt.json`: versioned machine-readable result, per-stage duration, and
  exact evidence such as catalog/pack/skill revisions.
- `report.md`: compact human-readable summary.
- `fake-empty-user/` and `fake-empty-remote-user/`: inspectable synthetic home
  directories after the journey.
- Local/remote project and copied source fixtures used by the run.

The benchmark exits nonzero on the first broken contract but still writes a
partial receipt identifying that boundary.

## CI-safe regression

The example contains a synthetic 12-skill fixture so CI does not depend on an
external checkout:

```bash
cargo test -p provider-local --example skill_experience_benchmark
```

This uses the same 16-stage journey as the real repository run. Passing it
proves benchmark mechanics and Clark contract integration; the separate real
run proves compatibility with the current Obra repository layout.

The standalone runner can also emit a complete receipt from that fixture:

```bash
cargo run -p provider-local --example skill_experience_benchmark -- \
  --synthetic --out target/skill-experience-benchmark/local
```

## Pre-release suite

The repository-level entrypoint runs this deep journey as one family inside a
broader fast sample of core/provider contracts, local capabilities, scripted
conversations, remote/git/worktree behavior, frontend contracts, and UI
resilience:

```bash
./scripts/run-pre-release-benchmarks.sh \
  --superpowers /path/to/obra/superpowers
```

Add `--live` to include a representative Clark Platform model sample: managed
skill/resource use, basic response, read/search tools, permissioned mutation,
and memory. That lane has no implicit provider or model and fails closed unless
all four values are set:

```bash
CLARK_CODE_PROVIDER=clark-platform \
CLARK_CODE_BASE_URL=https://api.clarkslabs.com/v1 \
CLARK_CODE_MODEL='clark-code:YOUR_EXPLICIT_TIER' \
CLARK_CODE_API_KEY=ck_live_... \
  ./scripts/run-pre-release-benchmarks.sh \
    --superpowers /path/to/obra/superpowers \
    --live
```

The live credential is never stored in the logs or receipts. The release
workflow runs this suite before any native build. It uses a pinned Superpowers
revision and enables the paid lane only when
`CLARK_CODE_PRERELEASE_LIVE=1` is configured at the repository level.
