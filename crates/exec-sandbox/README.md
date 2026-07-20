# Clark execution sandbox

`exec-sandbox` is the policy and platform-adapter layer between agent tools and
`exec-core`. It has no provider, UI, HTTP, or Tauri dependency.

The contracts are deliberately separate:

- `SandboxPolicy` is a resolved, symlink-aware filesystem and process-network
  policy.
- `SandboxPreset` maps product modes to read-only, workspace-write, or explicit
  full access.
- `SandboxBackend` compiles a policy into one platform launch request. Built-in
  adapters target macOS Seatbelt, Linux bubblewrap, and the versioned Windows
  runner protocol.
- `SandboxManager` owns backend readiness and compilation, not process I/O.
- `SandboxedExecutor` applies one policy to both direct filesystem primitives
  and every process launched through `Executor::prepare_process`.

Clark Cloud is a brokered host capability and does not run inside a child
process. It remains enabled by default for signed-in users while arbitrary
networking from local shell processes is denied. Direct external tools such as
`web_fetch`, browsers, and connected services have their own consent class.

## Platform readiness

| Platform | Backend | Runtime contract |
| --- | --- | --- |
| macOS | `/usr/bin/sandbox-exec` + Seatbelt | Fixed system path; no setup |
| Linux | bubblewrap | Pinned private build preferred; probed distro helper is a fallback |
| Windows | restricted-token runner | Private signed `clark-command-runner.exe`; missing helper reports `SetupRequired` |

The Windows runner is intentionally a separate executable boundary. Its setup
service can own elevation, durable capability identities, ACL reconciliation,
and consent without giving the desktop or model process those privileges. The
desktop passes it a bounded, UTF-16-safe, versioned request; it never searches
`PATH` for this private helper. Setup readiness requires an attestation marker
that pins the runner digest, protocol versions, offline identity SID, and WFP
network enforcement. The manager exposes elevation only until that one-time
bootstrap is valid; later owned-workspace actions are explicitly user-mode.

`exec-sandbox-windows` separates elevated bootstrap, user-mode enrollment, and
the unprivileged command runner. Bootstrap provisions a DPAPI-sealed offline
identity, verifies SID-scoped outbound firewall rules, and installs only a
stable device capability. Enrollment uses the caller's existing `WRITE_DAC` to
add a distinct root capability for each owned workspace; protected roots have
an explicit elevated fallback. The runner logs on as the offline identity,
re-verifies the firewall rules, creates a `WRITE_RESTRICTED` token, and keeps
the worker tree in a kill-on-close job. Missing or partial setup still reports
`SetupRequired` rather than silently running with partial containment.

Linux releases compile bubblewrap 0.11.2 from its digest-pinned upstream source
archive and ship it under the private `clark-resources/sandbox/linux` tree. The
bundle includes the LGPL notices and the exact verified source archive. The
build retains upstream's warning policy except that GCC's optimization-only
`format-overflow` false-positive is not promoted to an error. Debian and RPM
metadata also declare the distro `bubblewrap` dependency. Runtime probing
tries an explicit test override, the bundled build, and the standard distro
locations in order, selecting the first candidate that can actually create a
user namespace. This gives AppImage installs a private first choice on normal
hosts without masking a working setuid distro helper on hardened hosts.

## Verification

```bash
cargo test -p exec-sandbox
cargo run -p exec-sandbox --example sandbox_benchmark -- \
  --iterations 5000 --launch-iterations 30
```

The integration simulation verifies direct and process writes, protected Git
metadata, PTY execution, and loopback network denial on the native backend.
Compiler simulations exercise all three platform adapters on every host. The
benchmark emits JSON and exits non-zero if native containment fails.

`.github/workflows/sandbox.yml` makes all three native backends mandatory. The
Windows lane provisions the ephemeral hosted runner and exercises inside/outside
writes, child processes, Git metadata, junction escape, loopback networking, and
orphan-process cleanup through the packaged helper boundary.

The paid model receipt is separately ignored and cost-capped:

```bash
CLARK_CODE_API_KEY=... \
CLARK_SANDBOX_E2E_MODEL=clark-code:kimi_k27_code \
cargo test -p provider-local --test sandbox_live \
  paid_cheapest_model_cannot_escape_workspace -- --ignored --exact --nocapture
```
