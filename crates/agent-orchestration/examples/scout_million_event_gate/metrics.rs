use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;

#[derive(Default)]
pub struct ByteCounter {
    bytes: u64,
}

impl ByteCounter {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Write for ByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(buffer.len() as u64)
            .ok_or_else(|| io::Error::other("serialized byte count overflow"))?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn serialized_size(value: &impl Serialize) -> Result<u64, String> {
    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, value).map_err(|error| error.to_string())?;
    Ok(counter.bytes())
}

#[derive(Default, Serialize)]
pub struct RssMetrics {
    pub source: &'static str,
    pub samples_bytes: BTreeMap<String, u64>,
    pub peak_sampled_bytes: Option<u64>,
}

impl RssMetrics {
    pub fn new() -> Self {
        Self {
            source: rss_source(),
            ..Self::default()
        }
    }

    pub fn sample(&mut self, phase: &str) {
        let Some(bytes) = resident_set_bytes() else {
            return;
        };
        self.samples_bytes.insert(phase.to_string(), bytes);
        self.peak_sampled_bytes = Some(
            self.peak_sampled_bytes
                .map_or(bytes, |current| current.max(bytes)),
        );
    }
}

pub fn write_receipt(path: &Path, receipt: &impl Serialize) -> Result<(), String> {
    if path.exists() {
        return Err(format!(
            "refusing to overwrite existing benchmark receipt {}",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "benchmark receipt has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut body = serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?;
    body.push(b'\n');
    std::fs::write(path, body).map_err(|error| error.to_string())
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
