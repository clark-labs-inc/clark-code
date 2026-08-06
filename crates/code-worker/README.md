# `clark-code-worker`

`clark-code-worker` is a quiet, standalone JSONL service. It composes the
existing `agent_core::Provider` contract with `provider-local::LocalAgentProvider`
through the `coding` plugin:

```text
stdin JSONL -> code-host -> coding plugin -> Provider -> local executor
stdout JSONL <- response    trajectory.jsonl <- host receipt
```

Start it with a trusted configuration containing canonical project
registrations and an absolute trajectory directory:

```json
{
  "schema_version": 1,
  "worker_name": "gpu-worker",
  "projects": [{"id": "omnivoicegraph", "root": "/workspace/omnivoicegraph"}],
  "trajectory_root": "/workspace/.clark/trajectory",
  "execution_residency": "remote_worker",
  "enabled_plugins": ["coding"],
  "provider": {
    "base_url": "https://api.clarkslabs.com/v1",
    "model": "clark-code:free",
    "api_key_env": "CLARK_CODE_API_KEY",
    "allowed_tools": ["bash", "write_file", "edit_file"],
    "allowed_command_prefixes": ["git ", "cargo test"]
  }
}
```

The credential is read from the worker environment; it is never placed in the
config file or returned by `ping`. Permission decisions are fail-closed: file
tools must be named explicitly, and shell actions additionally require a
configured command prefix. The worker intentionally has no desktop UI and no
stdout logging besides protocol responses.

The first-party `coding` operations are:

- `session.open` with `{ "session_id": "...", "options": {...} }` and a
  registered `project_id`; the worker rejects a `cwd` outside that registration;
- `session.prompt` with `{ "session_id": "...", "input": PromptInput }`,
  retaining typed content blocks and attachments end to end;
- `session.close` with `{ "session_id": "..." }`.

Other capabilities should be implemented as `HeadlessPlugin` crates and
registered by a worker composition root. This keeps research, evaluation,
simulation, and future GPU-specific plugins separate from the coding loop.

For a worker resident on a CPU/GPU host, set `execution_residency` to
`remote_worker` and launch it through [`code-remote`](../code-remote/README.md).
The launcher creates the registered roots, uploads a versioned worker binary
to `$HOME/.clark/bin`, verifies its SHA-256 before an atomic install, writes a
mode-0600 content-addressed, credential-free config, and reuses both artifacts
by verified digest. A configured
credential is sent once over encrypted SSH stdin and is never included in argv
or JSON.

`remote_worker_connect` attaches one account/project worker, so provider
calls, tools, project inspection, and trajectory receipts stay on the remote
host. The desktop remains only the native account, capability, projection, and
cancellation control plane.

The remote launcher requires a `remote_worker` ping receipt. A local or
executor-only process therefore cannot be mistaken for a fully resident worker.
