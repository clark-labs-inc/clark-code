---
name: yeet
description: Publish an authorized local change set to GitHub by confirming scope, checking repository rules, committing intentionally, pushing the current branch, and opening or updating a draft pull request.
---

# Publish local changes to GitHub

Use only when the user explicitly asks to commit and publish, push and open a PR, or invokes this skill. A request to edit code alone does not authorize publication.

## Preflight

1. Read repository instructions. They may forbid branches, commits, broad staging, or particular verification commands.
2. Inspect `git status --short --branch`, the relevant diff, current branch, upstream, and remotes. Never discard, stash, reset, clean, or overwrite unrelated work to make the tree look clean.
3. Identify the exact files that belong to the requested change. If concurrent or unrelated edits overlap those files and cannot be separated safely, stop and explain the conflict.
4. Run proportionate checks before publication. Do not run paid or live-provider tests unless the user explicitly authorized them.

## Commit and push

Stage only the confirmed paths. Do not use broad staging such as `git add -A`, `git commit -a`, or a workspace-wide formatter unless repository instructions and the user's scope explicitly call for the whole tree. Review the staged diff and secret-sensitive filenames before committing.

Write a concise commit message that describes the outcome. Add attribution only when repository settings require it. Push the current branch to its configured remote; do not force-push, create a new branch, or change upstream topology unless explicitly authorized and allowed by repository instructions.

## Draft pull request

Use a connected GitHub capability discovered through `tool_search`, or `gh pr create --draft` through `bash`. Reuse an existing pull request for the branch instead of creating a duplicate. The title should state the outcome. The body should include:

- what changed and why
- verification actually run
- notable risk or follow-up

Do not merge, mark ready, request reviewers, or modify labels unless requested.

Verify the final commit SHA, pushed ref, and draft PR URL from GitHub. Report them along with checks and any files intentionally left uncommitted.
