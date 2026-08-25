# Sandbox architecture

The open local provider routes command execution through `exec-sandbox`. The
policy layer is platform-neutral; platform adapters implement macOS Seatbelt,
Linux bubblewrap, and Windows restricted-token execution.

Core invariants:

- the selected project root is the only writable project capability, plus
  explicitly granted absolute roots from `.agent/settings.json`
  (`sandbox_write_roots`, for shared machine-level build caches reached through
  in-project symlinks); they never apply in Plan Mode's read-only preset;
- read-before-edit remains enforced above process isolation;
- explicit read roots do not imply write access;
- environment and working-directory inputs are validated before launch;
- cancellation terminates the owned process tree;
- unavailable required isolation fails closed;
- extension tools retain their own authorization boundaries and cannot weaken
  the local sandbox.

Deterministic contracts live beside `exec-sandbox` and `provider-local`.
Packaged releases verify helper placement, signing, bundled binaries, and
platform behavior separately.
