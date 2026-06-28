# Clark Platform API — passthrough tool-capable model (spec)

**Why:** clark-desktop's "Clark Code" mode runs the agent loop *locally* (file/shell
tools on the user's laptop) and uses Clark only as the model. That needs the
Platform API to accept OpenAI `tools` and stream back native `tool_calls`.
Today `POST /v1/chat/completions` rejects `tools` unconditionally
(`crates/clark-services/src/platform_api.rs:197`) because every model runs Clark's
internal sandbox agent loop and returns final output only.

This spec adds **one new model id** (`clark-code`) that is *passthrough*: when the
client sends `tools`, Clark forwards `messages + tools + tool_choice` straight to
the underlying provider (OpenRouter) and streams the provider's chunks back
(text + `tool_calls`) — no internal sandbox loop. The client runs its own tool
loop. Auth stays the same `ck_live_` Platform key; everything stays on `/v1`.

The underlying function-calling is **already implemented** at the bridge layer:
`crates/clark-agent-bridge/src/openrouter_request.rs` already builds requests with
`tools`, `OpenRouterMessage::Assistant { tool_calls }`, and
`OpenRouterMessage::Tool { tool_call_id, content }`, and
`openrouter_stream.rs` already parses streamed `tool_calls`. The work is exposing
a passthrough path at the **Platform API layer** that bypasses the agentic
dispatcher.

## Client contract (what clark-desktop sends / expects)

clark-desktop already speaks standard OpenAI streaming, so the contract is just
"normal OpenAI tool-calling":

Request — `POST https://api.clarkslabs.com/v1/chat/completions`, `Authorization:
Bearer ck_live_…`:
```jsonc
{
  "model": "clark-code",
  "stream": true,
  "messages": [
    {"role":"system","content":"…"},
    {"role":"user","content":"…"},
    {"role":"assistant","content":null,
     "tool_calls":[{"id":"call_1","type":"function",
                    "function":{"name":"read_file","arguments":"{\"path\":\"a.rs\"}"}}]},
    {"role":"tool","tool_call_id":"call_1","content":"…file contents…"}
  ],
  "tools": [{"type":"function","function":{"name":"read_file","description":"…","parameters":{…}}}],
  "tool_choice": "auto"
}
```

Response (stream) — standard `chat.completion.chunk` SSE, with `tool_calls` deltas:
```
data: {"choices":[{"delta":{"content":"Reading…"}}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_2","function":{"name":"edit_file","arguments":""}}]}}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}
data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}
data: [DONE]
```
clark-desktop's accumulator (`provider-local/src/llm.rs`) reassembles these by
`index` already — no client change needed beyond pointing `model` at `clark-code`.

## Server changes (the integration map)

After model selection in `create_chat_completion`, branch to a passthrough path
when the model is flagged passthrough; otherwise unchanged.

1. **`clark-core/src/runtime_core/model_profiles.rs`** — add `is_passthrough: bool`
   to `ProfileModel`; add a `clark-code` entry to `PRODUCTION_MODELS`
   (`is_passthrough: true`, provider `openrouter`, model = a tool-capable model,
   e.g. `z-ai/glm-5.2`). Optionally add `features` for the catalog.

2. **`clark-services/src/platform_api.rs`**
   - Relax the guard at ~`197-204`: allow `tools`/`tool_choice` **only** when the
     selected model `is_passthrough` (still reject for agentic models).
   - After model selection (~`206`): `if selection.is_passthrough { return
     passthrough_chat_completion(state, auth, body).await; }` — bypass
     `dispatch_platform_response` / `dispatch_run` entirely.

3. **`clark-services/src/platform_api/types.rs`** — extend `ChatMessage` with
   `tool_calls: Option<Vec<ToolCall>>` (assistant) and `tool_call_id:
   Option<String>` (role `tool`); add `ToolCall { id, type, function:
   FunctionCall { name, arguments } }`; add `tool_calls` to the response message
   type. (These map 1:1 onto the existing `OpenRouterMessage` variants.)

4. **`clark-services/src/platform_api/request.rs`** — add
   `full_message_history(messages) -> Vec<OpenRouterMessage>` that preserves ALL
   turns incl. assistant `tool_calls` and `tool` results, instead of
   `last_user_message_text` (which keeps only the final user text — lossy, and the
   reason passthrough must NOT reuse the dispatcher path).

5. **New `passthrough_chat_completion` (in platform_api.rs or a submodule)** —
   reuse `clark-agent-bridge::openrouter_request::build_request_body` (or its
   pieces) to construct the OpenRouter call from the full history + tools, call
   `providers/openrouter/http.rs`, and:
   - stream: forward provider chunks as OpenAI `chat.completion.chunk`s (text +
     tool_calls) — most can pass through unchanged; `openrouter_stream.rs` already
     parses them. This skips the "final-answer-only" projection in
     `platform_api/streaming.rs`.
   - non-stream: assemble one `chat.completion` with `message.tool_calls`.

6. **`clark-services/src/platform_api/models.rs`** — `capabilities_for_model`
   takes `is_passthrough`; for `clark-code` advertise
   `["native_tool_calls","streaming"]` (drop `agent_tools`/`artifacts`/`background`/
   `conversation_memory`, which are sandbox-only).

7. **Billing/idempotency** — auth (`authenticate_platform_request`) and the
   model-access check stay. But the passthrough path creates **no `agent_job`
   row**: read token usage from the OpenRouter response (usage / `x-or-*` headers)
   and record the billing + a `platform_api_response` row (add a
   `passthrough: true` flag) for idempotency.

## Properties / constraints
- **Stateless from Clark's view**: passthrough sends only what the client
  provides each call (no server memory/workspace injection) — correct, because the
  client owns the transcript and tools.
- **No artifacts / sandbox tools** for `clark-code` — the client's tools are
  authoritative; Clark must NOT inject its own tools in passthrough.
- **Agentic models unchanged** — `clark`, `clark_max`, `openrouter` keep rejecting
  client tools and running the internal loop. clark-desktop already uses an
  agentic model (`clark`) for `clark_research`/memory (no tools), so those keep
  working with the same key.

## clark-desktop side
No code change required beyond setting the default coding model to `clark-code`
(`provider-local/src/config.rs::DEFAULT_MODEL`, and the StartCard model
placeholder). The loop already emits `tools` and parses `tool_calls`. Research +
memory stay on the agentic `clark` model (`DEFAULT_RESEARCH_MODEL`).
