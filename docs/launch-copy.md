# Clark Code — open-source launch copy

Four surfaces, one fact base. Every claim here is verifiable in the public repo
(model IDs in `crates/provider-local/src/config.rs` and `app/src/lib/localAgent.ts`,
sandboxing in `crates/exec-sandbox/`, SSH remote in `crates/code-remote/`,
importer in `crates/provider-local/src/external_import/`).

---

## 1. X/Twitter

Clark Code is now fully open source (Apache-2.0).

A desktop coding agent in the spirit of Claude Code — except it's a native app with the engine written in Rust, and the default model tier is free.

• Runs on your machine — your files, your shell — or on any Linux box over SSH
• Free tier: DeepSeek V4 Flash (400k context, weekly allowance, no card). Paid: GLM 5.2 and Kimi K3 with 1M context
• Default-on OS sandboxing (Seatbelt / bubblewrap / restricted tokens), every edit shown as a diff before it lands, git checkpoint + undo
• Migrating from Claude Code takes one click: your MCP servers, skills, CLAUDE.md and slash commands just work
• macOS, Windows, Linux — signed builds, auto-update

https://github.com/clark-labs-inc/clark-code

*(Single post; if splitting into a thread, break after the bullets.)*

---

## 2. Show HN

**Title:** Show HN: Clark Code – open-source desktop coding agent (Rust engine, free DeepSeek tier)

Hi HN — we just open-sourced Clark Code (Apache-2.0), a desktop coding agent in the same family as Claude Code or Codex, but shipped as a native app with the agent engine written in Rust (Tauri 2 + React on top).

What it does:

- Runs the agent loop locally against your files and shell, or against a remote machine over SSH — it uploads a static musl worker binary over a multiplexed SSH connection, sha256-verified, credentials passed over stdin and never argv.
- Models: the free tier is DeepSeek V4 Flash (400k context) with a weekly included allowance — no card. Paid tiers are GLM 5.2 and Kimi K3, both 1M context.
- Safety is the part we're proudest of: OS-level sandboxing is on by default (Seatbelt on macOS, bubblewrap on Linux, restricted tokens on Windows), shell commands are risk-classified with a hard "blocked" floor that even full-auto mode can't cross, every file edit is shown as a diff in the approval gate, and there's git checkpoint/undo that never touches your real index.
- The engine handles the long-session problems: automatic context compaction, context-overflow recovery, steering the agent mid-run, parallel tool execution, and resumable streams that survive disconnects.
- MCP servers (stdio), sub-agents, skills, plan mode, and a goal mode where the agent runs autonomously against a token/time budget it cannot raise itself.
- If you use Claude Code today: the importer reads your .mcp.json, .claude settings, skills, and CLAUDE.md, and .claude/commands slash commands work with zero migration.
- There's a machine-checked feature map in the repo (harness/feature-matrix.mjs) — every documented tool and capability is derived from source and CI fails if anything is unmapped. Same spirit in EVALS.md: scripted successes can't be reported as live results.

Honest limitations, so you don't have to dig for them: the app currently requires sign-in, and models route through our API (which fronts OpenRouter) — there's no BYO-API-key or local-inference path in the shipped app yet. The git history is squashed to the public release. A paid plan adds specialist workspaces, including a security one that produces verified findings with sandboxed proof-of-concept execution.

Repo: https://github.com/clark-labs-inc/clark-code — happy to answer anything about the Rust engine, the sandboxing, or the SSH remote design.

---

## 3. Reddit (r/LocalLLaMA or r/opensource)

**Title:** Clark Code is now fully open source — desktop coding agent running DeepSeek V4 Flash (free tier), GLM 5.2 and Kimi K3

We open-sourced our desktop coding agent under Apache-2.0: https://github.com/clark-labs-inc/clark-code

It's in the Claude Code / Codex category, but a native desktop app (Rust engine, Tauri shell) built around open-weight models:

- **Free tier:** DeepSeek V4 Flash, 400k context, weekly included allowance, no card required
- **Paid:** GLM 5.2 and Kimi K3, both with 1M context
- Works on your local machine or drives a remote Linux box over SSH (uploads a static worker binary, checksum-verified)
- Default-on OS sandboxing, command risk classification with a hard block floor, diff approval on every edit, git checkpoint/undo
- stdio MCP support, sub-agents, skills, memory, plan mode, budgeted autonomous goal mode
- One-click migration from Claude Code (MCP servers, skills, CLAUDE.md, slash commands)

Being upfront since this sub will ask: **inference is not local.** The models route through our API today — no Ollama/BYO-key path in the shipped app yet. What's open is the entire client and agent engine: the loop orchestration, sandboxing, tool layer, SSH remote, compaction — all of it, Apache-2.0. If you want to see how a production agent harness handles context compaction, overflow recovery, parallel tools, or mid-run steering, it's all readable Rust.

Happy to answer questions.

---

## 4. README hero / release notes blurb

**Clark Code** — an open-source desktop coding agent (Apache-2.0). A native app in the spirit of Claude Code, with the agent engine written in Rust.

- **Local or remote.** Runs against your files and shell, or any Linux host over SSH via a checksum-verified static worker.
- **Open-weight models.** DeepSeek V4 Flash free with a weekly included allowance (400k context); GLM 5.2 and Kimi K3 with 1M context on paid plans.
- **Safe by default.** OS sandboxing on by default, risk-classified shell commands with a hard block floor, diff approval on edits, git checkpoint/undo.
- **A real engine.** Context compaction, overflow recovery, mid-run steering, parallel tools, resumable streams, concurrent sessions.
- **Meets you where you are.** stdio MCP servers, skills, sub-agents, plan & goal modes, and one-click migration from Claude Code.
- **Cross-platform.** Signed and notarized builds for macOS, Windows, and Linux with auto-update.

---

## Pre-publish checklist

1. ~~README pricing contradiction~~ — **fixed**: all price rates removed from README, `harness/clark-code-feature-map.json`, and the QA runbook.
2. ~~Free-lane billing contradiction~~ — **fixed**: README now says the free route consumes the included weekly allowance, matching `CreditBanner.tsx`.
3. ~~`clark-agent-compaction` visibility~~ — **verified public** (HTTP 200).
4. ~~"Most performant" / "ACP-first"~~ — **fixed**: README now says "Lean by construction" (no unreceipted numbers) and drops the ACP-first ordering claim.
5. **Open:** sign-in screen still reads "Private beta · Clark Code" (`app/src/surfaces/SignInScreen.tsx`).
6. Reminder: don't claim "tool-result truncation" anywhere — bash output is captured completely by design.
