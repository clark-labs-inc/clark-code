# `code-remote`

`code-remote` is the host-side SSH transport for a complete headless Clark
Code worker. It does not execute provider calls itself: it deploys and starts
the configured worker binary on the target, then carries the strict
`code-host` JSONL protocol over the SSH stdio channel. The same transport can
launch the Scientist worker from `clark-scientist`: that worker exposes the
protocol-compatible `scientist`/`turn` Invoke lane while retaining the richer
`specialist_turn` protocol for the native Scientist provider.

The lifecycle is deliberately bounded:

1. Validate the project/trajectory roots, worker config, credential variable,
   and shell-safe host/path inputs.
2. Open one process-owned SSH master on a private mode-`0700` control socket.
   Every probe, upload, and worker channel multiplexes over that connection;
   neither the socket nor the master persists after the worker lifecycle.
3. Probe the remote OS and home, create only the registered roots and Clark
   artifact directories, and select a versioned architecture-specific binary.
4. Upload the binary through a random partial path, verify SHA-256 remotely,
   and atomically install it mode `0700`. Install the credential-free,
   content-addressed worker config mode `0600` only when its digest is absent.
5. Start the worker with a multiplexed `ssh -T` channel and, when configured,
   send one credential
   line through encrypted SSH stdin. The credential is not in argv, config,
   trajectory, or responses.
6. Correlate concurrent requests by `request_id`; validate each progress
   sequence, bound per-request buffering, and require exactly one terminal
   response. The worker durably reserves mutating IDs before execution and
   replays their bounded ordered progress and terminal response after a lost
   connection or process restart; an incomplete or conflicting receipt fails
   closed. Cancellation and shutdown remain first-class protocol messages. A ping must prove
   `execution_residency=remote_worker` before connect succeeds.
7. On disconnect, cancel active work and tear down both the worker channel and
   its SSH master. The
   credential-free, content-addressed config remains as an immutable cache for
   the next process start.

Protocol v2 is intentionally incompatible with stale workers that cannot
stream ordered progress. The startup ping rejects an old protocol before it can
own a conversation.

The Tauri `remote_worker_connect` command is the sole attach/reconnect
boundary. It health-checks an existing account/project worker, reuses it when
healthy, or atomically replaces it after a fresh bootstrap while keeping the
opaque handle stable for attached sessions. It never persists the bootstrap
secret. Completed request retries replay from the worker's owner-private
receipt; incomplete or mismatched retries are rejected without re-execution.

Use `RemoteWorkerSpec` from a trusted native caller or the sole
`remote_worker_connect` Tauri command. Clark Desktop has no executor-only remote
mode: its worker owns the provider loop, tools, project primitives, and
trajectory together.

For a Scientist worker, set `remote_binary` to the verified
`clark-code-headless` release binary, keep `execution_residency` as
`remote_worker`, and send an `invoke` request with `plugin: "scientist"` and
`operation: "turn"`. The response includes the Scientist projection and the
worker's private trajectory remains on the remote host.

For the trusted GPU autoresearch lane, the remote worker config may explicitly
set `provider.permission_mode` to `allow_anything` together with
`provider.sandbox_mode` `disabled` or `danger-full-access`. The worker then
auto-answers tool permission requests for both its Scientist and Engineer
effects. This profile is valid only with `execution_residency:
"remote_worker"`; local workers must keep the default prompt/permission path.
The transport still validates the registered project and remote roots, and the
worker retains its budget, residency, and terminal-receipt checks.

The ignored live tests in `tests/live_cpu.rs` separate a transport/catalog
receipt from the explicitly paid model-turn receipt. Neither is part of normal
CI. Set `CLARK_REMOTE_CPU_RECEIPT` to an absolute path when retaining the
transport receipt; Cargo may run the test with the crate rather than repository
root as its working directory.
