use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Default, Serialize)]
pub(super) struct RssMetrics {
    source: &'static str,
    samples_bytes: BTreeMap<String, u64>,
    peak_sampled_bytes: Option<u64>,
}

impl RssMetrics {
    pub(super) fn new() -> Self {
        Self {
            source: rss_source(),
            ..Self::default()
        }
    }

    pub(super) fn sample(&mut self, phase: &str) {
        let Some(bytes) = resident_set_bytes() else {
            return;
        };
        self.samples_bytes.insert(phase.to_owned(), bytes);
        self.peak_sampled_bytes = Some(
            self.peak_sampled_bytes
                .map_or(bytes, |current| current.max(bytes)),
        );
    }
}

#[cfg(target_os = "linux")]
fn rss_source() -> &'static str {
    "linux_proc_self_status_vmrss"
}

#[cfg(all(unix, not(target_os = "linux")))]
fn rss_source() -> &'static str {
    "unix_ps_rss_sample"
}

#[cfg(not(unix))]
fn rss_source() -> &'static str {
    "unavailable"
}

#[cfg(target_os = "linux")]
fn resident_set_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kibibytes = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    kibibytes.checked_mul(1024)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn resident_set_bytes() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let kibibytes = String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    kibibytes.checked_mul(1024)
}

#[cfg(not(unix))]
fn resident_set_bytes() -> Option<u64> {
    None
}
