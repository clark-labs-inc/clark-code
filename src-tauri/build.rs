use sha2::{Digest, Sha256};
use std::env;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

fn main() {
    let worker_digest = worker_digest().unwrap_or_else(|error| {
        if env::var("PROFILE").as_deref() == Ok("release") {
            panic!("Clark Scientist release sidecar is invalid: {error}");
        }
        println!("cargo:warning=Clark Scientist development sidecar is unavailable: {error}");
        String::new()
    });
    println!("cargo:rustc-env=CLARK_SCIENTIST_WORKER_SHA256={worker_digest}");
    tauri_build::build();
}

fn worker_digest() -> Result<String, String> {
    let manifest = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR is unavailable")?,
    );
    let target = env::var("TARGET").map_err(|_| "TARGET is unavailable")?;
    let suffix = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let binaries = manifest.join("binaries");
    // Clark's macOS release is universal. Both architecture slices of the
    // signed host must pin the final universal sidecar rather than a temporary
    // single-architecture input used during Tauri's merge.
    let universal = binaries.join("clark-code-headless-universal-apple-darwin");
    let target_worker = binaries.join(format!("clark-code-headless-{target}{suffix}"));
    let worker = if target.ends_with("apple-darwin") && universal.is_file() {
        universal
    } else {
        target_worker
    };
    println!("cargo:rerun-if-changed={}", worker.display());
    sha256_file(&worker)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    // Keep the digest buffer off the build-script stack. Windows build scripts
    // have a smaller default stack than their Unix counterparts, and a 1 MiB
    // local array can terminate the process with STATUS_STACK_OVERFLOW before
    // Tauri packaging starts.
    let mut chunk = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
