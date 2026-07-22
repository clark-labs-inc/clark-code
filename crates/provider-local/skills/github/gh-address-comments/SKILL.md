---
name: gh-address-comments
description: Inspect and address actionable GitHub pull-request review feedback, including unresolved threads, requested changes, inline comments, fixes, verification, and explicitly requested replies or thread resolution.
---

# Address pull-request review feedback

## Establish scope

Resolve the active repository, branch, and pull request. Read repository instructions and current local changes. Preserve unrelated work and do not switch branches or rewrite the tree to recreate a clean baseline.

Fetch review state with a connected GitHub tool discovered through `tool_search`, or use `gh` through `bash`. Prefer thread-aware GraphQL data for inline review threads because ordinary PR comments do not reliably represent resolution state. Gather:

- review decision and requested-change reviews
- unresolved review threads with file, line, author, and comment text
- issue-level comments that request code changes
- the current head SHA and check state

## Triage before editing

Group duplicate comments by root cause. Classify each item as actionable, already fixed, obsolete because the code moved, a question, or a disagreement requiring explanation. Verify every classification against the current file; never infer it from the comment alone.

If the user asked only to inspect or summarize, stop after reporting the triage. If they asked to address feedback, implement the smallest coherent fixes, update all affected call sites when a shared contract changes, and run targeted checks. Do not modify unrelated local changes.

## External effects

Posting replies, resolving threads, requesting re-review, committing, and pushing are separate external effects. Perform only those the user requested. Before replying, refresh the thread and head SHA so the response cannot describe stale code. Keep replies concrete: what changed, where, and what verification passed. Do not resolve a thread whose concern remains open.

Conclude with addressed items, deferred or disputed items and why, checks run, and whether any GitHub state was changed.
