# Update and relaunch durability receipt — 2026-08-20

## Scope

The original updater warning meant the renderer had not completed a remote
conversation PUT before the update deadline. That made network latency look
like local data loss risk even though the updater needed a process-restart
guarantee, not a synchronous cloud acknowledgement.

The repaired boundary commits the exact pending snapshot, mutation identity,
and generated Markdown bytes to the native account-partitioned SQLite journal
before restart. Product cloud remains the cross-device authority and receives
the same mutation ID during asynchronous replay.

Foundation HEAD was `f5f851c981efa1d9dfcd7757a56db34d4a0b5343`. The bounded
relevant-file patch fingerprint used for the final package was
`7d5a7659a2eae407095f28aacb6062a5f0cf855a9bd196017668564972f08102`.
The worktree contained unrelated concurrent changes; they were preserved and
no commit was created.

## Deterministic gates

At `2026-08-21T06:39:48Z`:

- `cargo test -p desktop-foundation --lib -- --nocapture`: 137 passed.
- `pnpm --dir app test`: 829 passed, 2 skipped.
- `pnpm --dir app typecheck`: passed.
- `pnpm --dir app build`: passed.
- `cargo fmt --all --check`: passed.
- `cargo clippy -p desktop-foundation --all-targets -- -D warnings`: blocked by
  nine existing Rust 1.97 lints, including unrelated worktree, diagnostics,
  project-context, SSH, and Windows-smoke files plus longstanding outbox and
  recovery lints. No new durability-path Clippy diagnostic was reported.

Those focused contracts cover native FULL-synchronous database reopen, stable
mutation replay after an uncertain PUT response, a newer coalesced snapshot
while an older cloud PUT hangs, restart-safe artifact staging while upload
hangs, same-path final-byte revalidation, and SHA-fenced superseded uploads.

At `2026-08-21T06:52:08Z`, a follow-up stale-code cleanup replaced the old
unconditional checkpoint-plus-clear sequence with one atomic, mutation-fenced
publication acknowledgement. The regression proves that a late acknowledgement
cannot replace or clear a newer pending snapshot. The cleanup also removed the
unused detail-level `syncPending` wire field and the obsolete checkpoint/clear
APIs. The full native and frontend counts remained 137 passed and 829 passed
with 2 skipped; typecheck, build, formatting, and diff checks passed. Strict
Clippy remained blocked by eight pre-existing Rust 1.97 diagnostics outside the
changed publication path.

## Packaged macOS boundary

The package hashes below bind the original durability repair before that
follow-up cleanup. They prove the packaged restart boundary and schema, but do
not claim byte identity with the later atomic-ack source; that exact source is
currently covered by the deterministic gates above.

The downstream Clark product composition at
`8a251625ba45e459837ff14a9e27b9583ded4478` built the final foundation source
with the product frontend and exact development Tauri configuration. The
result was:

- bundle: `/Users/stan/Documents/git/clark/target/debug/bundle/macos/Clark Code Dev.app`
- main-binary SHA-256: `67f7e5160366fc970732909ba585c21ee486c716f701a810764cbdaebb2972d9`
- ad-hoc local signature: `codesign --sign -`; deep strict verification passed
- runtime: the Dev bundle launched as its own `clark-code` process
- database: the real `com.clark.desktop.dev` database opened in WAL mode and
  contained `snapshot_pending`, `pending_mutation_id`, and `artifact_stage`
- cleanup: only the Dev process was stopped; the installed Clark Code process
  was left untouched

The canonical all-in-one launcher was separately blocked before this app build
by the concurrent downstream Scientist migration: its `code-host` construction
does not yet supply the new `SessionOptions.session_id`. The main product app
was therefore built directly from the same product configuration with the
already staged sidecars. This receipt does not claim a fresh Scientist worker
self-test.

## Bounded paid Clark conversation

One user-authorized Clark Max conversation was submitted through the real local
Clark UI, gateway, queue, worker, sandbox, and hosted-provider boundary.

- conversation: `a2d0c88e-88aa-43cd-a0af-1d36634c9279`
- job: `53b36763-9bb0-4f13-9d5b-fb73a9977f73`
- requested route: `openrouter` / `deepseek/deepseek-v4-pro-0813`
- resolved provider response: requested and actual model both
  `deepseek/deepseek-v4-pro-0813`; upstream provider `GMICloud`
- retained event history: 15 events, one capsule snapshot, one attempted tool
  call, zero tool results, 7.1 seconds, and no event-sequence anomaly
- terminal result: `insufficient_credits` after provider-response sequence 1
- billing ledger: no usage-outbox row was recorded for the job, so this receipt
  makes no token or cost claim
- artifact result: no Markdown file was created or published because billing
  stopped execution before the tool call ran

The browser was then soft-reloaded and fully stopped/recreated at the exact
conversation URL. Both the submitted prompt and the terminal insufficient-credit
state reappeared after each reopen. The terminal screenshot remains at
`/Users/stan/Documents/git/clark/update-durability-paid-terminal.png` with
SHA-256 `9b53c8c06cf72d35b4597eba26478dd939b29308c056b779ad6c2ee2db86fd50`.

This is positive evidence for the real paid route, provider resolution, durable
terminal conversation history, and cold-browser replay. It is deliberately not
a successful paid artifact-generation or post-update artifact-continuity claim.
A funded test account is required to close that final live gate; repeating the
same paid request against an exhausted account would add cost risk without new
evidence.
