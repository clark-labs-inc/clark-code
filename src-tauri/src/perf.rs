//! Owner-local performance measurement for the snapshot emit path.
//!
//! Compiled only under the `perf-profiling` feature. Unlike `diagnostics`, this
//! is deliberately release-compatible: the jitter it exists to measure only
//! appears at `opt-level = 3` inside the real platform WebView, so a debug
//! build cannot observe it.
//!
//! Privacy boundary, identical to `diagnostics`: conversation content,
//! credentials, and session identifiers never reach these records. Session
//! identity is a per-process salted hash so one run's rows can be grouped
//! without the id being recoverable.

#![cfg(feature = "perf-profiling")]

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use agent_core::Snapshot;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Directory selector, mirroring `AGENT_DESKTOP_LOGS`. A run writes nothing
/// unless this names an absolute, real directory.
const PERF_DIR_ENV: &str = "CLARK_PERF_DIR";
/// Companion event carrying just the emit's shape, so the WebView can time the
/// crossing without parsing the snapshot itself.
const EMIT_TICK_EVENT: &str = "perf-emit-tick";
/// Flush cadence for the emit log. Large enough that the hot path is not doing
/// syscalls, small enough that a crash keeps almost everything.
const FLUSH_EVERY: u64 = 200;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static WRITTEN: AtomicU64 = AtomicU64::new(0);
static EMIT_LOG: OnceLock<Option<Mutex<BufWriter<File>>>> = OnceLock::new();
static SESSION_SALT: OnceLock<u64> = OnceLock::new();

/// One row per snapshot emit. Field names are stable — `harness/perf-compare`
/// and the summary reducer read them.
#[derive(Serialize)]
struct EmitRecord {
    seq: u64,
    /// Wall clock at emit, for pairing with the WebView's arrival timestamp.
    emit_unix_us: u128,
    /// Bytes actually placed on the wire.
    bytes: usize,
    serialize_us: u128,
    emit_us: u128,
    timeline_len: usize,
    tool_calls_len: usize,
    runs_len: usize,
    /// Salted hash of the session id — never the id itself.
    session: u64,
}

/// The companion tick. Deliberately tiny (~80 bytes) so its own transport cost
/// is negligible next to the snapshot it announces.
#[derive(Clone, Serialize)]
struct EmitTick {
    seq: u64,
    emit_unix_us: u128,
    bytes: usize,
    timeline_len: usize,
    tool_calls_len: usize,
}

fn now_unix_us() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
}

fn salt() -> u64 {
    *SESSION_SALT.get_or_init(|| {
        let mut hasher = DefaultHasher::new();
        now_unix_us().hash(&mut hasher);
        std::process::id().hash(&mut hasher);
        hasher.finish()
    })
}

/// Group rows by session without retaining the identifier.
fn session_digest(snapshot: &Snapshot) -> u64 {
    let mut hasher = DefaultHasher::new();
    salt().hash(&mut hasher);
    match snapshot.session.as_ref() {
        Some(id) => id.as_str().hash(&mut hasher),
        None => 0u8.hash(&mut hasher),
    }
    hasher.finish()
}

/// Validate the configured directory with the same discipline as
/// `AGENT_DESKTOP_LOGS`: absolute, real, not a symlink.
pub(crate) fn perf_dir() -> Option<PathBuf> {
    let raw = std::env::var_os(PERF_DIR_ENV).filter(|value| !value.is_empty())?;
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        tracing::warn!("{PERF_DIR_ENV} must be an absolute directory; perf records disabled");
        return None;
    }
    if fs::create_dir_all(&path).is_err() {
        tracing::warn!("could not create {PERF_DIR_ENV}; perf records disabled");
        return None;
    }
    let metadata = fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        tracing::warn!("{PERF_DIR_ENV} must be a real directory, not a symlink");
        return None;
    }
    Some(path)
}

fn emit_log() -> Option<&'static Mutex<BufWriter<File>>> {
    EMIT_LOG
        .get_or_init(|| {
            let path = perf_dir()?.join("native-emit.jsonl");
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()?;
            Some(Mutex::new(BufWriter::new(file)))
        })
        .as_ref()
}

fn record(row: &EmitRecord) {
    let Some(log) = emit_log() else { return };
    let Ok(line) = serde_json::to_string(row) else {
        return;
    };
    let Ok(mut writer) = log.lock() else { return };
    let _ = writeln!(writer, "{line}");
    if WRITTEN
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(FLUSH_EVERY)
    {
        let _ = writer.flush();
    }
}

/// Flush the emit log. Called from the app's exit handler so the tail of a run
/// is never lost.
pub(crate) fn flush() {
    if let Some(Ok(mut writer)) = emit_log().map(|log| log.lock()) {
        let _ = writer.flush();
    }
}

/// Serialize the snapshot exactly once, publish those same bytes, and record
/// what each step cost.
///
/// Measuring the payload with a second `to_vec` would double the serde cost on
/// the hot path and corrupt the number it was meant to produce. `RawValue`
/// writes through verbatim under the `to_string` Tauri itself calls, so the
/// wire bytes stay byte-identical to an uninstrumented build.
pub(crate) fn emit_snapshot_instrumented(app: &AppHandle, snapshot: &Snapshot) {
    let started = Instant::now();
    let json = match serde_json::to_string(snapshot) {
        Ok(json) => json,
        Err(error) => {
            tracing::warn!(%error, "snapshot did not serialize; emitting through the plain path");
            let _ = app.emit(crate::snapshot_emit::SNAPSHOT_EVENT, snapshot);
            return;
        }
    };
    let serialize_us = started.elapsed().as_micros();
    let bytes = json.len();
    let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let emit_unix_us = now_unix_us();

    let tick = EmitTick {
        seq,
        emit_unix_us,
        bytes,
        timeline_len: snapshot.timeline.len(),
        tool_calls_len: snapshot.tool_calls.len(),
    };
    // All evals funnel through one event-loop queue, so the tick's arrival in
    // the WebView bounds the snapshot's. Send it first.
    //
    // This is itself one extra eval per snapshot — a known perturbation of the
    // measurement, but an ~80 byte payload against a transcript-sized one.
    let _ = app.emit(EMIT_TICK_EVENT, tick);

    // SAFETY: `json` came straight from `serde_json::to_string` on a Snapshot,
    // so it is a single well-formed JSON value with no surrounding whitespace —
    // exactly the contract `from_string_unchecked` requires.
    //
    // The checked `from_string` would re-parse the whole payload to validate it,
    // which on a megabyte transcript costs more than the serialization this
    // meter exists to measure. (A debug build still re-parses under
    // `debug_assert!`; the release profiling build, which is the one worth
    // measuring, does not.)
    let raw = unsafe { serde_json::value::RawValue::from_string_unchecked(json) };
    let emit_started = Instant::now();
    let _ = app.emit(crate::snapshot_emit::SNAPSHOT_EVENT, &raw);
    let emit_us = emit_started.elapsed().as_micros();

    record(&EmitRecord {
        seq,
        emit_unix_us,
        bytes,
        serialize_us,
        emit_us,
        timeline_len: snapshot.timeline.len(),
        tool_calls_len: snapshot.tool_calls.len(),
        runs_len: snapshot.runs.len(),
        session: session_digest(snapshot),
    });
}

/// A report name must not be able to escape the run directory or collide with
/// the records this module owns.
fn safe_report_name(name: &str) -> Result<String, String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        && !name.starts_with(['.', '-'])
        && !name.contains("..");
    if ok {
        Ok(format!("{name}.json"))
    } else {
        Err("report name must be lowercase alphanumeric with . _ -".into())
    }
}

fn write_in_perf_dir(dir: &Path, file: &str, contents: &str) -> Result<(), String> {
    fs::write(dir.join(file), contents).map_err(|error| format!("write perf report: {error}"))
}

/// Persist one recorder report into the active run directory.
#[tauri::command]
pub(crate) fn perf_write_report(name: String, json: String) -> Result<(), String> {
    let dir = perf_dir().ok_or("CLARK_PERF_DIR is not set to a usable directory")?;
    let file = safe_report_name(&name)?;
    write_in_perf_dir(&dir, &file, &json)
}

/// Round-trip probe so the WebView can bound its clock offset against the host
/// rather than assuming the two agree.
#[tauri::command]
pub(crate) fn perf_clock_probe() -> u128 {
    now_unix_us()
}

#[cfg(test)]
mod tests {
    use super::safe_report_name;

    #[test]
    fn report_names_stay_inside_the_run_directory() {
        assert_eq!(safe_report_name("frames").unwrap(), "frames.json");
        assert_eq!(safe_report_name("cold-open.1").unwrap(), "cold-open.1.json");
        for rejected in [
            "",
            ".hidden",
            "-leading",
            "../escape",
            "with/slash",
            "Upper",
            "with space",
        ] {
            assert!(
                safe_report_name(rejected).is_err(),
                "{rejected:?} should be rejected"
            );
        }
    }
}
