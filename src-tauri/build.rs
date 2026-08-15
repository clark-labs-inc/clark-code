use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=CLARK_BUILD_GIT_SHA");
    println!("cargo:rerun-if-env-changed=CLARK_BUILD_GIT_DIRTY");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");

    let git_sha = std::env::var("CLARK_BUILD_GIT_SHA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| git_output(&["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    let git_dirty = std::env::var("CLARK_BUILD_GIT_DIRTY")
        .ok()
        .filter(|value| matches!(value.as_str(), "true" | "false"))
        .unwrap_or_else(|| {
            git_output(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|output| !output.is_empty())
                .to_string()
        });
    println!("cargo:rustc-env=CLARK_BUILD_GIT_SHA={git_sha}");
    println!("cargo:rustc-env=CLARK_BUILD_GIT_DIRTY={git_dirty}");

    tauri_build::build()
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
}
