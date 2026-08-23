//! The single boundary where a session projection crosses into the WebView.
//!
//! Every snapshot the UI renders passes through `emit_snapshot`. Routing all
//! emit sites through one function buys two things that were previously
//! impossible: one place to measure what the wire actually costs (see `perf`),
//! and one place to replace the whole-snapshot payload with a delta without
//! auditing a dozen call sites again.
//!
//! Tauri's event transport serializes the payload to JSON and then embeds that
//! JSON in a JavaScript *source string* for `evaluateJavaScript:`, so the cost
//! here scales with the whole conversation and is paid on the platform's main
//! thread. Keep this function's body cheap and keep new work out of it.

use agent_core::Snapshot;
use tauri::AppHandle;

pub(crate) const SNAPSHOT_EVENT: &str = "snapshot";

/// Publish one session projection to the WebView.
#[cfg(not(feature = "perf-profiling"))]
pub(crate) fn emit_snapshot(app: &AppHandle, snapshot: &Snapshot) {
    use tauri::Emitter;

    let _ = app.emit(SNAPSHOT_EVENT, snapshot);
}

/// Publish one session projection, recording what it cost to do so.
#[cfg(feature = "perf-profiling")]
pub(crate) fn emit_snapshot(app: &AppHandle, snapshot: &Snapshot) {
    crate::perf::emit_snapshot_instrumented(app, snapshot);
}
