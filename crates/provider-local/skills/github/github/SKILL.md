---
name: github
description: Orient and route GitHub repository, issue, pull request, review, CI, and publishing work. Use for general GitHub requests or when a more specific GitHub skill should be selected.
---

# GitHub work in the host

Use this skill as a router. Load the narrow skill instead when the request is specifically about:

- review feedback: `github:gh-address-comments`
- failing GitHub Actions checks: `github:gh-fix-ci`
- committing, pushing, and opening a draft pull request: `github:yeet`

## Operating contract

1. Read the repository's current instructions and git state before proposing or changing anything.
2. Match the user's verb: an ask to inspect, explain, review, or diagnose is read-only; an ask to fix or publish includes the corresponding in-scope changes.
3. Prefer a connected GitHub capability when one exists. Use `tool_search` to find it. Otherwise use the local `gh` CLI through `bash`, first checking `gh auth status` and never printing tokens.
4. Resolve the repository and pull request from the current checkout when possible. If no checkout context exists, require an explicit `OWNER/REPO` and issue or pull-request identifier.
5. Treat local source, GitHub state, and CI logs as separate evidence. Re-read externally mutable state before the final answer.
6. Skills do not override the host permissions, Plan mode, repository instructions, or the user's scope. Do not create commits, push, open or edit issues/PRs, post comments, resolve threads, merge, or rerun workflows unless the request authorizes that effect.

Return the outcome first, then the smallest useful evidence: repository/PR identity, the relevant state, and any remaining blocker.
