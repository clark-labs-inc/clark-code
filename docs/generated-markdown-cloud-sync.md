# Generated Markdown cloud sync

Clark Code automatically persists every generated Markdown artifact from
`~/.clark/workspace/<conversation>/` to Clark's private artifact bucket. This
behavior is mandatory for signed-in users: it is enabled by default and there
is no preference, feature toggle, or per-document opt-out.

## Product contract

- Local generation and preview do not wait on the network.
- An authenticated client always schedules generated Markdown for cloud sync.
- Offline or temporarily unauthenticated clients retain a workspace-relative
  retry intent and retry with bounded exponential backoff.
- A cloud snapshot never contains an absolute host path in an artifact ID or
  URI.
- The client does not claim a cloud artifact URI until the backend has verified
  object size, content type, and SHA-256.
- Reads require the owning Clark account. Public conversation shares have
  private and legacy-local artifact URIs removed.
- Rewriting a file creates an immutable content version. Repeating the same
  logical ID and SHA-256 is idempotent.
- Conversation deletion and account-data deletion revoke metadata access and
  best-effort delete the corresponding private S3 objects.

## Runtime flow

1. The local provider emits a Markdown artifact with a workspace-relative
   identity and a local-only absolute URI for immediate preview.
2. Before snapshot persistence, the WebView projects the artifact to either:
   - `clark-workspace://<conversation>/<relative-path>` while pending, or
   - `/api/desktop/conversations/<conversation>/artifacts/<artifact-version>`
     after completion.
3. The native host canonicalizes the source beneath the exact conversation
   workspace, rejects traversal/symlinks outside the workspace and non-Markdown
   files, enforces the 8 MiB limit, reads the bytes, and computes SHA-256.
4. The authenticated Clark API creates or reuses an immutable metadata row and
   returns a short-lived private-bucket PUT lease.
5. Rust uploads the exact bytes without sending the Clark bearer to S3, then
   asks Clark to complete the version.
6. Clark reads the uploaded object and verifies size, `text/markdown`, and
   SHA-256, then copies those bytes to a server-owned immutable prefix before
   marking it uploaded. Abandoned or replayed temporary uploads expire after
   one day.
7. The next coalesced snapshot contains only the authenticated Clark API URI.

## Simulated user journeys

| Journey | Expected behavior | Receipt |
| --- | --- | --- |
| Generate online | Local preview appears immediately; safe pending snapshot is created; upload completes; cloud URI replaces pending URI. | `cloudArtifacts.spec.ts` completion journey |
| Generate offline | Local preview remains usable; cloud snapshot contains only `clark-workspace://`; retry continues up to 30-second intervals. | Retry scheduler and safe-projection test |
| Quit while pending | The cloud snapshot retains the conversation-bound relative URI; reopening on the originating device rediscovers and uploads it. | Native pending-URI parser test |
| Open on another device while pending | The artifact remains visibly unavailable because that device lacks the local bytes; no path or false cloud-safe claim is exposed. | `desktopArtifactCanPreview` mobile test |
| Rewrite the same file | A later producing tool call schedules a new hash/version; an identical hash reuses the existing version. | Immutable rewrite journey test plus backend unique key |
| Open on mobile after upload | Mobile recognizes the relative `/api/` route and fetches Markdown with its Clark bearer. | Existing authenticated `fetchArtifactText` path plus route test |
| Share conversation publicly | The shared transcript remains visible, but private, pending, and legacy local artifact URIs are removed. | `desktop_share` redaction test |
| Delete conversation/account | Database access is fenced/cascaded before object cleanup; stale clients cannot recreate a tombstoned conversation. | Tombstone transaction plus object-key return path |
| Switch accounts | Artifact jobs, completed mappings, and retry timers are cleared with the cloud-history account boundary. | Shared reset epoch in `cloudHistory.ts` |

## Edge-case policy

- Empty Markdown is valid; files over 8 MiB are rejected.
- Unsupported extensions, directories, invalid UTF-8 filenames/content,
  malformed hashes, control characters, path traversal, and cross-conversation
  workspace URIs are rejected.
- A hash collision with a different declared size is rejected.
- Failed integrity completion marks the version failed and removes the bad
  object before a retry can reuse the row.
- Upload quotas cap abuse at 1,000 versions per conversation and 1 GiB of
  declared artifact bytes per user.
- S3 leases are short-lived and never persisted in snapshots or exposed to the
  WebView.
- Backend rollout must precede Desktop rollout. Older backends return an error;
  Desktop keeps the artifact pending and retries without losing the local file.
- Existing historical local-path snapshots migrate lazily the next time the
  originating Desktop opens or updates that conversation. Public share
  redaction protects those legacy paths immediately after backend rollout.

## Primary implementation paths

- `app/src/lib/cloudArtifacts.ts`
- `app/src/lib/cloudHistory.ts`
- `src-tauri/src/commands/desktop_artifacts.rs`
- `crates/provider-local/src/agent_adapter/translate.rs`
- `clark/crates/clark-services/src/rest/desktop_artifacts.rs`
- `clark/crates/clark-services/migrations/zz_20260728c_desktop_artifact_versions.sql`
