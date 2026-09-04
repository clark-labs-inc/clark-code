# Security access recovery verification

Verified at `2026-09-03T13:51:07Z` against foundation HEAD
`42fd6e12cf189a719155977b7e167533325e9842` with the scoped worktree changes
listed below. The repository was dirty before this work and also contains
unrelated concurrent changes; no branch, worktree, stash, reset, commit,
package publication, deployment, or release action was performed.

## Conclusion

The original Clark Security screen was not waiting on a slow entitlement
service. Its first authenticated access request received an expired-credential
response, while the renderer had no refresh-and-replay contract. A separate
native trajectory upload recovered after refreshing the same session, but that
did not cause the one-shot access hook to retry. The failed request therefore
left the access projection unknown, and the Security gate rendered that unknown
state as an indefinite access check.

The current source fixes both broken boundaries:

- authenticated product calls share one refresh flight and replay the exact
  rejected operation once;
- refresh operations themselves cannot recurse, and a second unauthorized
  response is terminal;
- native auth-expiry events, concurrent renderer requests, and manual reconnect
  use the same refresh boundary;
- refreshed credentials are published only while the initiating account still
  owns both the visible store and the hidden auth cache;
- access results are generation- and owner-fenced across sign-out and account
  switches;
- a failed, still-indeterminate Security access check becomes an explicit
  retryable state instead of continuing to say that access is being checked.

## Production incident evidence

Conversation `7edefe84-01c4-4587-b178-ad13c87e88b4` was re-fetched from the
private production trajectory store with:

```text
AWS_DEFAULT_REGION=us-west-2 ./scripts/get_debug_trajectory.sh \
  7edefe84-01c4-4587-b178-ad13c87e88b4 <temporary-output>/trajectory.jsonl
python3 .../clark-prod-trajectory-forensics/scripts/summarize_trajectory.py \
  <temporary-output>/trajectory.jsonl
```

Content-free receipt:

- schema version: 2
- environment/source: production desktop
- records: 1 metadata, 1 conversation, 25 trajectory events, 1 completion
- completion boundary: change revision `3393703`, event sequence `8582962`
- malformed lines: 0
- validation warnings: 0
- client/app version: `0.1.134`
- embedded foundation revision:
  `4f1f4ef79d0cf7719f22f63b484776d66cbb2e2f`, dirty product composition

Bounded CloudWatch inspection from `2026-09-03T12:06:00Z` through
`2026-09-03T12:16:00Z` established this content-free sequence:

1. `/api/desktop/access` succeeded at `12:07:12.821Z`.
2. The next access request failed at `12:08:03.427Z`; paired request/error
   records classify it as HTTP 401 caused by expired JWT validation.
3. The trajectory endpoint failed for the same auth class at `12:08:03.422Z`.
4. The native trajectory path resumed successfully at `12:08:13.941Z` and
   continued publishing successful records.
5. There was no later `/api/desktop/access` request in the bounded window.

The durable trajectory also records a later provider failure for the remote
run. That failure is secondary: it occurred after the access request had
already failed and cannot explain why the Security gate remained on its access
checking screen. Private message bodies, tool arguments, and raw provider
errors are intentionally omitted from this receipt.

The pre-fix foundation source at `4f1f4ef...` confirms the first contract break:
`productRequest` invoked native IPC exactly once, `useProductAccess` latched its
single attempt, and the Security projection did not consume the hook's error as
a terminal state.

## Deterministic verification

All commands below passed against the same scoped source inventory, whose
ordered SHA-256 manifest digest is
`2de524fa672134d566165d03bdc8abfb5bd23898de82b91794e9e4cbd9c95d85`.

| Layer | Command or receipt | Result |
| --- | --- | --- |
| Focused renderer contracts | `pnpm --dir app exec vitest run src/product/productBridge.spec.ts src/store/sessionStore.cloudLifecycle.spec.ts src/lib/authPersistence.spec.ts src/lib/useProductAccess.spec.ts src/lib/specialists.readRoots.spec.ts src/surfaces/specialists/SpecialistAccessGate.spec.ts` | 6 files, 37/37 passed |
| Full frontend suite | `pnpm --dir app test` | 175 passed files, 2 skipped; 832 passed tests, 5 skipped |
| TypeScript | `pnpm --dir app typecheck` | passed |
| Production frontend bundle | `pnpm --dir app build` | passed; 5,329 modules transformed |
| Native auth/outbox recovery | `cargo nextest run -p desktop-foundation expired_token_refreshes_and_replays_the_durable_outbox_end_to_end` | nextest `287d923d-637e-453a-806a-a97d08b07113`, 1/1 passed, 145 skipped |
| Specialist desktop/mobile matrix | `pnpm --dir harness test:specialists` | passed in mock/no-paid-call mode; 23 checks; no browser console errors; no failed requests |
| Specialist matrix artifacts | typed receipt | `target/specialist-matrix-smoke/20260903T134524Z-46548` |
| Scoped diff hygiene | `git diff --check -- <scoped files>` | passed |

The focused contracts exercise exact replay, concurrent single-flight refresh,
new requests waiting behind native-event recovery, bounded second-401 behavior,
refresh failure without replay, refresh-operation non-recursion, unrelated
failure passthrough, successful store credential publication, failed-refresh
reconnect state, account-switch rejection, manual reconnect-before-sync,
hidden-cache ownership, immediate old-account result hiding, late-result
rejection, and the retryable Security failure UI/action.

The current optimized bundle contains both the central recovery failure string
and the retryable specialist access copy. Relevant generated asset hashes were:

- `app/dist/assets/index-D8sv3fKd.js`:
  `c1902bb6fe0d5bad583f04b9a4aead600784baa3a3543622a2e8300421038d71`
- `app/dist/assets/specialists-Bt-csqHJ.js`:
  `180d592a89d25f7396cb9a450bac140d8f75a6deabb413b8eb19bda0a5cb0eb7`

The supported unsigned `./script/build_and_run.sh` path also compiled and
launched `target/debug/clark-code` successfully with debug diagnostics. It was
observed running for five seconds without a runtime failure and then stopped;
the already-running installed Clark Code process was left untouched. This is a
native Tauri development smoke, not a packaged branded-product receipt.

## Package and live-product boundary

The public macOS updater was checked directly and still served Clark Code
`0.1.173`, published `2026-08-26T15:54:38Z`, with product source revision
`c57eb8cc8324b63a69ce0a49a0753ec4f924f0c4`. The installed application is also
`0.1.173`; its executable SHA-256 is
`e25b55f1944909a2d9fcae5274a80550f1c667b92b30e200cc1a8b15cec19a42`, and its
embedded foundation revision is the pre-fix `4f1f4ef...` source reconstructed
above.

Therefore neither the installed application nor the current public release
contains this fix. They are valid negative provenance evidence, not proof that
the repair works in a packaged WebView or against the live identity provider.
A positive packaged/live claim requires a new release built from these exact
changes, followed by an expired-session Security canary that observes access
401, one successful refresh, one exact access replay, and a settled Security
state. Credentialed packaging, release publication, deployment, and mutation
of a real account session were outside this verification's authority and were
not attempted.

## Scoped files

- `app/src/product/productBridge.ts`
- `app/src/store/sessionStore.appActions.ts`
- `app/src/lib/auth.ts`
- `app/src/lib/useProductAccess.ts`
- `app/src/lib/specialists.ts`
- `app/src/surfaces/MobileRemoteAgent.tsx`
- `app/src/surfaces/ProfileMenu.tsx`
- `app/src/surfaces/Settings.tsx`
- `app/src/surfaces/specialists/SpecialistWorkspace.tsx`
- the six focused frontend test files named in the command above
- `EVALS.md`
- this receipt

Final diff inspection found pre-existing concurrent specialist-catalog work in
`app/src/lib/specialists.ts`, `app/src/lib/specialists.readRoots.spec.ts`, and
`app/src/surfaces/specialists/SpecialistWorkspace.tsx`, plus a separate
specialist-matrix wording change in `EVALS.md`. The access-recovery hunks are
distinct from those edits. None of the unrelated changes or deleted assets in
the wider dirty worktree were modified, staged, reverted, or committed.
