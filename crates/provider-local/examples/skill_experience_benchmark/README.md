# Agent skill experience benchmark

This benchmark reproduces a Superpowers journey from two isolated, initially
empty user homes. It exercises the foundation catalog, executor, managed-pack,
typed-history, instruction, and provider boundaries. It does not call a live
model.

## What it proves

The local lane performs the following ordered journey:

1. Start with empty personal and project skill roots.
2. Reproduce legacy directory-symlink discovery.
3. Load personal, project, and nested instruction provenance.
4. Install the real Superpowers tree into agent-managed user storage.
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

Remote skill execution is covered at the durable worker boundary. The retired
provider-local tunnel simulator is intentionally not retained as a second
runtime.

## Run against the real Superpowers repository

```bash
cargo run -p provider-local --example skill_experience_benchmark -- \
  --superpowers /path/to/obra/superpowers
```

When no path is provided, the runner checks the normal Codex plugin cache.

Use `--out /new/path` to choose the artifact directory. The runner refuses to
overwrite an existing directory. By default, it creates a unique directory
under `target/skill-experience-benchmark/`.

Each run writes:

- `receipt.json`: versioned machine-readable result, per-stage duration, and
  exact evidence such as catalog/pack/skill revisions.
- `report.md`: compact human-readable summary.
- `fake-empty-user/`: inspectable synthetic home after the journey.
- Local project and copied source fixtures used by the run.

The benchmark exits nonzero on the first broken contract but still writes a
partial receipt identifying that boundary.

## CI-safe regression

The example contains a synthetic 12-skill fixture so CI does not depend on an
external checkout:

```bash
cargo nextest run -p provider-local --example skill_experience_benchmark
```

This uses the same 10-stage journey as the real repository run. Passing it
proves benchmark mechanics and foundation contract integration; the separate real
run proves compatibility with the current Obra repository layout.

The standalone runner can also emit a complete receipt from that fixture:

```bash
cargo run -p provider-local --example skill_experience_benchmark -- \
  --synthetic --out target/skill-experience-benchmark/local
```

Product-specific pre-release orchestration and paid model lanes belong to the
downstream distribution. This public benchmark remains deterministic and
credential-free.
