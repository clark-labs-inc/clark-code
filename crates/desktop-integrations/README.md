# Native integrations

The native integration catalog lives in **Settings → Integrations**. Adapters
are compiled Rust implementations of the Integration trait; the app does not
load downloaded integration code or model-authored manifests. The host
registers trusted adapters, the catalog discovers them, and a ToolPack exposes
their bounded model tools to the local provider.

The first adapter is a read-only iMessage prototype. There is no draft, send,
Apple Events, Automation permission, background inbox polling, or “text Clark
Code to start a task” path.

## Current boundary

The model sees one tool named **read_imessage_selection**. It has an empty
argument schema. It cannot accept a handle, phone number, conversation ID,
message ID, query, SQL, time range, or future-message subscription.

Access requires these explicit steps:

1. Open a Clark Code task and connect iMessage from Settings.
2. Approve the native read dialog for that task.
3. Choose one eligible self-conversation.
4. Select 1–20 exact messages and enable them for the read tool.
5. Let the task call **read_imessage_selection**.

The native grant binds the active account, task ID, account generation, and
live session instance. One task at a time owns the adapter. Grants expire after
15 minutes and disappear on restart. Sleep, lock, session changes, reconnect,
conversation reselection, selection edits, and revoke invalidate or clear
access. Every tool read reopens the database read-only and checks that each
enabled message still has the exact text shown when selected. Results are
prefixed as untrusted quoted data.

The adapter opens **~/Library/Messages/chat.db** read-only. It lists at most 30
one-to-one iMessage chats whose address appears in local outgoing-message
metadata. Group chats, SMS, arbitrary recipients, attachments, attributed-body
decoding, and inbox polling are excluded. The selected chat exposes at most 50
recent records, capped at 4,000 characters per record. A tool selection is
limited to 20 records and 32 KB total. Messages schema incompatibility fails
closed.

## macOS limitation

Full Disk Access belongs to the entire Clark Code app. macOS does not grant it
per task or per conversation. Clark's sandboxed file tools and sandboxed child
commands deny Messages paths after path and symlink resolution, but Full Access
commands, MCP servers, external agents, terminals, computer use, another
process running as the user, or a compromised renderer are not comprehensively
confined by this integration grant.

The Settings panel and native dialog disclose this. Do not grant Full Disk
Access when untrusted host tools are active or when this boundary is
insufficient. A separately identified authenticated broker remains the path to
strong app-level isolation.

## Local acceptance

Start the unsigned app with **./script/build_and_run.sh**. Do not select,
unlock, or use an Apple signing identity. Use a disposable macOS test account
and a self-conversation containing only test text. If the permission owner
shown by macOS is Terminal or Codex instead of the actual Clark Code app, stop
and treat that as a harness defect.

| Check | Procedure | Acceptance evidence |
| --- | --- | --- |
| Deny | Open a task, choose Connect, cancel the native dialog | No conversation metadata or text returned |
| Grant | Grant Full Disk Access to the exact test app, restart, connect | Permission owner is the app; only eligible self-conversations listed |
| Read tool | Choose one chat, select one test message, enable the tool, ask Clark to read it | Tool returns only that exact text, labeled as quoted data |
| Selection change | Enable one message, change any checkbox, call the tool | Tool refuses until the new selection is explicitly enabled |
| Revoke | Revoke in Clark, then remove Full Disk Access in System Settings | Old task tool and selection refuse |
| Sleep/lock | Enable text, sleep or lock, unlock, call the tool | Old grant refuses and requires reconnect |
| Restart | Enable text, restart Clark, call the tool | No persisted grant or selected text |
| Other task | Enable text in task A, call the tool from task B | Native scope rejects task B |
| Alternate tools | Probe file, shell, Full Access, MCP, external-agent, terminal, and computer-use paths in an isolated test account | Record each boundary separately; deterministic tests cover only sandboxed file/process paths |

Never put real addresses, private message bodies, or macOS permission-database
contents in logs or receipts. Record build identity, macOS version, permission
owner, action, counts, and outcome.

“Text Clark Code to start a task” remains a separate inbound-channel design. It
needs authenticated identity, an explicit allowlist, durable message GUID
deduplication, an inbound cursor, origin tagging, loop prevention, and task
ownership. It must not be added as polling on top of this read tool.
