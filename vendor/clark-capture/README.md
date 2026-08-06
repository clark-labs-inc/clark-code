# `clark-capture`
Local-first Sentry-style capture for Rust. Set `CLARK_CAPTURED_LOGS`, construct `CaptureClient` with a project name, and capture events into the shared Clark Capture v1 NDJSON format. The crate has no daemon, credentials, or network transport.

Enable the optional `tracing` feature for `CaptureLayer`. See the [repository documentation](https://github.com/clark-labs-inc/clark-capture) for integrations, plugins, and schema details.
