# Sandbox architecture

The open local provider routes command execution through `exec-sandbox`. The
policy layer is platform-neutral; platform adapters implement macOS Seatbelt,
Linux bubblewrap, and Windows restricted-token execution.

Core invariants:

- the selected project root is the only writable project capability;
- read-before-edit remains enforced above process isolation;
- explicit read roots do not imply write access;
- environment and working-directory inputs are validated before launch;
- cancellation terminates the owned process tree;
- unavailable required isolation fails closed;
- product tools retain their own authorization boundaries and cannot weaken
  the local sandbox.

Deterministic contracts live beside `exec-sandbox` and `provider-local`.
Branded products separately own helper placement, signing, entitlements,
bundled binaries, VM journeys, and packaged runtime receipts.
