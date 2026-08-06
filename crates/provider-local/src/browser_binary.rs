//! Download/cache clark-browser (a stealth Chromium fork,
//! github.com/clark-labs-inc/clark-browser) — never bundled in the installer
//! (135-320MB per platform, Alpha status, ad-hoc-signed on macOS). Lazily
//! downloaded on first use of the opt-in `browser` tool, cached under
//! `~/.clark/bin/`, mirroring the URL/layout the project's own Python
//! launcher (`clarkbrowser/config.py`) uses so a version bump here just means
//! bumping the same two constants.

use std::path::{Path, PathBuf};

const CHROMIUM_VERSION: &str = "148.0.7778.96";
const RELEASE_TAG: &str = "chromium-v148.0.7778.96-stealth5";
const DOWNLOAD_BASE_URL: &str = "https://github.com/clark-labs-inc/clark-browser/releases/download";

/// This machine's clark-browser platform tag, or `Err` if unsupported.
fn platform_tag() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x64"),
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("macos", "x86_64") => Ok("darwin-x64"),
        ("windows", "x86_64") => Ok("windows-x64"),
        (os, arch) => Err(format!("clark-browser isn't available for {os}/{arch}")),
    }
}

fn archive_ext(tag: &str) -> &'static str {
    if tag == "windows-x64" {
        ".zip"
    } else {
        ".tar.gz"
    }
}

fn download_url(tag: &str) -> String {
    format!(
        "{DOWNLOAD_BASE_URL}/{RELEASE_TAG}/clark-browser-{tag}{}",
        archive_ext(tag)
    )
}

fn cache_root() -> Result<PathBuf, String> {
    let home = dirs_home()?;
    Ok(home
        .join(".clark/bin")
        .join(format!("clark-browser-{CHROMIUM_VERSION}")))
}

/// The `HOME` (or `USERPROFILE` on Windows) directory. No `dirs` crate
/// dependency needed for this one lookup.
fn dirs_home() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let var = "USERPROFILE";
    #[cfg(not(windows))]
    let var = "HOME";
    std::env::var_os(var)
        .map(PathBuf::from)
        .ok_or_else(|| format!("${var} is not set"))
}

/// Where the extracted binary lives inside `cache_dir`, per-platform.
fn binary_path_in(cache_dir: &Path, tag: &str) -> PathBuf {
    match tag {
        "darwin-arm64" | "darwin-x64" => cache_dir.join("Chromium.app/Contents/MacOS/Chromium"),
        "windows-x64" => cache_dir.join("chrome.exe"),
        _ => {
            let chrome = cache_dir.join("chrome");
            if chrome.exists() {
                chrome
            } else {
                cache_dir.join("headless_shell")
            }
        }
    }
}

pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// Return the path to a locally cached clark-browser binary, downloading and
/// extracting it first if it isn't already present. `on_progress` is called
/// periodically during the download (used to stream progress into the tool
/// call's content, since a 135-320MB fetch is not instant).
pub async fn ensure_binary(
    on_progress: impl Fn(DownloadProgress) + Send + Sync,
) -> Result<PathBuf, String> {
    let tag = platform_tag()?;
    let cache_dir = cache_root()?;
    let binary = binary_path_in(&cache_dir, tag);
    if binary.exists() {
        return Ok(binary);
    }

    std::fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
    let archive_bytes = download(&download_url(tag), &on_progress).await?;
    extract(&archive_bytes, &cache_dir, tag)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if binary.exists() {
            let mut perms = std::fs::metadata(&binary)
                .map_err(|e| e.to_string())?
                .permissions();
            perms.set_mode(perms.mode() | 0o111);
            std::fs::set_permissions(&binary, perms).map_err(|e| e.to_string())?;
        }
    }

    if !binary.exists() {
        return Err(format!(
            "clark-browser download completed but no binary found at {}",
            binary.display()
        ));
    }
    Ok(binary)
}

async fn download(
    url: &str,
    on_progress: &(impl Fn(DownloadProgress) + Send + Sync),
) -> Result<Vec<u8>, String> {
    use futures::StreamExt;
    let client = clark_http::build_client(clark_http::ClientOptions {
        request_timeout: Some(std::time::Duration::from_secs(600)),
        redirect_policy: clark_http::RedirectPolicy::Limited(10),
        ..Default::default()
    })
    .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("downloading clark-browser: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "clark-browser download failed: HTTP {}",
            resp.status()
        ));
    }
    let total = resp.content_length();
    let mut buf = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        buf.extend_from_slice(&chunk);
        on_progress(DownloadProgress {
            downloaded: buf.len() as u64,
            total,
        });
    }
    Ok(buf)
}

fn extract(bytes: &[u8], dest: &Path, tag: &str) -> Result<(), String> {
    if archive_ext(tag) == ".zip" {
        extract_zip(bytes, dest)
    } else {
        extract_tar_gz(bytes, dest)
    }
}

fn extract_tar_gz(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let gz = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(gz);
    archive
        .unpack(dest)
        .map_err(|e| format!("extracting clark-browser archive: {e}"))
}

fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("reading clark-browser archive: {e}"))?;
    archive
        .extract(dest)
        .map_err(|e| format!("extracting clark-browser archive: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_tag_matches_this_machine_or_reports_unsupported() {
        // Just confirm it doesn't panic and returns something plausible on
        // every OS/arch this app actually ships for.
        let result = platform_tag();
        if matches!(
            (std::env::consts::OS, std::env::consts::ARCH),
            ("linux", "x86_64")
                | ("macos", "aarch64")
                | ("macos", "x86_64")
                | ("windows", "x86_64")
        ) {
            assert!(result.is_ok());
        }
    }

    #[test]
    fn download_url_uses_zip_only_for_windows() {
        assert!(download_url("windows-x64").ends_with(".zip"));
        assert!(download_url("linux-x64").ends_with(".tar.gz"));
        assert!(download_url("darwin-arm64").ends_with(".tar.gz"));
        assert!(download_url("linux-x64").contains(RELEASE_TAG));
    }

    #[test]
    fn binary_path_matches_platform_layout() {
        let dir = Path::new("/cache");
        assert_eq!(
            binary_path_in(dir, "darwin-arm64"),
            dir.join("Chromium.app/Contents/MacOS/Chromium")
        );
        assert_eq!(binary_path_in(dir, "windows-x64"), dir.join("chrome.exe"));
    }

    #[test]
    fn extract_tar_gz_round_trips_a_fake_binary() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("chrome"), b"fake binary").unwrap();

        // Build a tar.gz in memory the same way the real archives are shaped.
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            builder.append_dir_all(".", &src).unwrap();
            builder.finish().unwrap();
        }
        let mut gz_bytes = Vec::new();
        {
            use std::io::Write;
            let mut enc = flate2::write::GzEncoder::new(&mut gz_bytes, flate2::Compression::fast());
            enc.write_all(&tar_bytes).unwrap();
            enc.finish().unwrap();
        }

        let dest = dir.path().join("dest");
        extract_tar_gz(&gz_bytes, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("chrome")).unwrap(), b"fake binary");
    }
}
