//! Discovers helper executables shipped with Clark Code.
//!
//! The package has two intentionally disjoint surfaces:
//! `clark-path/` contains commands exposed to agent processes, while
//! `clark-resources/` contains implementation helpers that are resolved only by
//! absolute path. macOS keeps supporting Tauri's signed sidecar location for
//! PATH tools, but private helpers never fall back to that shared directory.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static INSTALL_CONTEXT: OnceLock<InstallContext> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExposure {
    Path,
    Private,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BundledTool {
    pub name: &'static str,
    pub relative_path: &'static str,
    pub exposure: ToolExposure,
}

pub const PATH_DIR: &str = "clark-path";
pub const RESOURCES_DIR: &str = "clark-resources";
#[cfg(target_os = "linux")]
const PRODUCT_NAME: &str = "Clark Code";

pub const RIPGREP: BundledTool = BundledTool {
    name: if cfg!(windows) { "rg.exe" } else { "rg" },
    relative_path: if cfg!(windows) { "rg.exe" } else { "rg" },
    exposure: ToolExposure::Path,
};

pub const BUBBLEWRAP: BundledTool = BundledTool {
    name: "bwrap",
    relative_path: "sandbox/linux/bwrap",
    exposure: ToolExposure::Private,
};

pub const WINDOWS_SANDBOX_RUNNER: BundledTool = BundledTool {
    name: "clark-command-runner.exe",
    relative_path: "sandbox/windows/clark-command-runner.exe",
    exposure: ToolExposure::Private,
};

pub const WINDOWS_SANDBOX_SETUP: BundledTool = BundledTool {
    name: "clark-windows-sandbox-setup.exe",
    relative_path: "sandbox/windows/clark-windows-sandbox-setup.exe",
    exposure: ToolExposure::Private,
};

/// Every helper the package layout knows about. New private runtime helpers
/// belong here even when they should not be exposed to model-issued commands.
pub const BUNDLED_TOOLS: &[BundledTool] = &[
    RIPGREP,
    BUBBLEWRAP,
    WINDOWS_SANDBOX_RUNNER,
    WINDOWS_SANDBOX_SETUP,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallContext {
    executable_dir: Option<PathBuf>,
    resource_dir: Option<PathBuf>,
}

impl InstallContext {
    pub fn from_exe(current_exe: Option<&Path>) -> Self {
        let executable_dir = current_exe
            .and_then(Path::parent)
            .map(|path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
        let resource_dir = executable_dir.as_deref().map(default_resource_dir);
        Self {
            executable_dir,
            resource_dir,
        }
    }

    /// Construct a context with an explicit Tauri resource directory. Hosts
    /// other than the desktop app can use the same package contract without
    /// reproducing platform bundle discovery.
    pub fn from_layout(current_exe: Option<&Path>, resource_dir: Option<&Path>) -> Self {
        let executable_dir = current_exe
            .and_then(Path::parent)
            .map(|path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
        let resource_dir =
            resource_dir.map(|path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
        Self {
            executable_dir,
            resource_dir,
        }
    }

    pub fn current() -> &'static Self {
        INSTALL_CONTEXT.get_or_init(|| Self::from_exe(std::env::current_exe().ok().as_deref()))
    }

    pub fn bundled_tool(&self, tool: BundledTool) -> Option<PathBuf> {
        match tool.exposure {
            ToolExposure::Path => {
                let packaged = self
                    .resource_dir
                    .as_ref()
                    .map(|root| root.join(PATH_DIR).join(tool.relative_path));
                if let Some(path) = packaged.filter(|path| path.is_file()) {
                    return Some(path);
                }

                // Tauri signs macOS external binaries in Contents/MacOS. This
                // compatibility path is safe for public tools only.
                let sidecar = self.executable_dir.as_ref()?.join(tool.name);
                sidecar.is_file().then_some(sidecar)
            }
            ToolExposure::Private => {
                let path = self
                    .resource_dir
                    .as_ref()?
                    .join(RESOURCES_DIR)
                    .join(tool.relative_path);
                path.is_file().then_some(path)
            }
        }
    }

    pub fn rg_command(&self) -> PathBuf {
        self.bundled_tool(RIPGREP)
            .unwrap_or_else(|| PathBuf::from(RIPGREP.name))
    }

    /// Return PATH with the package tool directory first, preserving all user
    /// entries. No update is needed for source builds without bundled tools.
    pub fn path_with_bundled_tools(
        &self,
        existing_path: Option<OsString>,
    ) -> Result<Option<OsString>, std::env::JoinPathsError> {
        let Some(public_tool) = BUNDLED_TOOLS.iter().find_map(|tool| {
            (tool.exposure == ToolExposure::Path)
                .then(|| self.bundled_tool(*tool))
                .flatten()
        }) else {
            return Ok(None);
        };
        let dir = public_tool
            .parent()
            .expect("bundled PATH tool must have a parent directory");

        let mut entries = vec![dir.to_path_buf()];
        if let Some(path) = existing_path {
            entries.extend(std::env::split_paths(&path).filter(|entry| entry != dir));
        }
        std::env::join_paths(entries).map(Some)
    }
}

/// Activate package-visible tools before Tauri or Tokio starts worker threads.
/// Returns the directory added to PATH, when running from a packaged layout.
pub fn activate_bundled_path() -> Result<Option<PathBuf>, std::env::JoinPathsError> {
    let context = InstallContext::current();
    let Some(path) = context.path_with_bundled_tools(std::env::var_os("PATH"))? else {
        return Ok(None);
    };
    let activated = std::env::split_paths(&path).next();
    std::env::set_var("PATH", path);
    Ok(activated)
}

pub fn rg_command() -> PathBuf {
    InstallContext::current().rg_command()
}

fn default_resource_dir(executable_dir: &Path) -> PathBuf {
    if let Some(override_dir) = std::env::var_os("CLARK_RESOURCE_DIR") {
        return PathBuf::from(override_dir);
    }

    #[cfg(target_os = "macos")]
    {
        let candidate = executable_dir.join("../Resources");
        candidate.canonicalize().unwrap_or(candidate)
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(app_dir) = std::env::var_os("APPDIR") {
            return PathBuf::from(app_dir).join("usr/lib").join(PRODUCT_NAME);
        }
        let adjacent = executable_dir.join("../lib").join(PRODUCT_NAME);
        if adjacent.exists() {
            return adjacent.canonicalize().unwrap_or(adjacent);
        }
        let installed = PathBuf::from("/usr/lib").join(PRODUCT_NAME);
        if installed.exists() {
            return installed;
        }
        return executable_dir.to_path_buf();
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        executable_dir.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_rg_is_resolved_and_prepended() -> std::io::Result<()> {
        let package = tempfile::tempdir()?;
        let bin_dir = package.path().join("bin");
        let resources = package.path().join("resources");
        let path_dir = resources.join(PATH_DIR);
        std::fs::create_dir_all(&bin_dir)?;
        std::fs::create_dir_all(&path_dir)?;
        let app = bin_dir.join(if cfg!(windows) {
            "clark-desktop.exe"
        } else {
            "clark-desktop"
        });
        let rg = path_dir.join(RIPGREP.name);
        std::fs::write(&app, b"app")?;
        std::fs::write(&rg, b"rg")?;

        let context = InstallContext::from_layout(Some(&app), Some(&resources));
        assert_eq!(context.rg_command(), rg.canonicalize()?);
        let existing = std::env::join_paths([package.path().join("system")]).unwrap();
        let updated = context
            .path_with_bundled_tools(Some(existing))
            .unwrap()
            .expect("bundled rg should update PATH");
        let entries = std::env::split_paths(&updated).collect::<Vec<_>>();
        assert_eq!(entries[0], path_dir.canonicalize()?);
        assert_eq!(entries[1], package.path().join("system"));
        Ok(())
    }

    #[test]
    fn source_build_falls_back_to_command_name() -> std::io::Result<()> {
        let package = tempfile::tempdir()?;
        let app = package.path().join("clark-desktop");
        std::fs::write(&app, b"app")?;
        let context = InstallContext::from_exe(Some(&app));

        assert_eq!(context.rg_command(), PathBuf::from(RIPGREP.name));
        assert_eq!(context.path_with_bundled_tools(None).unwrap(), None);
        Ok(())
    }

    #[test]
    fn private_sandbox_helpers_are_resolved_without_entering_path() -> std::io::Result<()> {
        let package = tempfile::tempdir()?;
        let bin_dir = package.path().join("bin");
        let resources = package.path().join("resources");
        let path_dir = resources.join(PATH_DIR);
        let private_dir = resources.join(RESOURCES_DIR).join("sandbox/windows");
        std::fs::create_dir_all(&bin_dir)?;
        std::fs::create_dir_all(&path_dir)?;
        std::fs::create_dir_all(&private_dir)?;
        let app = bin_dir.join("clark-desktop");
        let rg = path_dir.join(RIPGREP.name);
        let runner = private_dir.join(WINDOWS_SANDBOX_RUNNER.name);
        let setup = private_dir.join(WINDOWS_SANDBOX_SETUP.name);
        std::fs::write(&app, b"app")?;
        std::fs::write(&rg, b"rg")?;
        std::fs::write(&runner, b"runner")?;
        std::fs::write(&setup, b"setup")?;
        let context = InstallContext::from_layout(Some(&app), Some(&resources));

        assert_eq!(
            context.bundled_tool(WINDOWS_SANDBOX_RUNNER),
            Some(runner.canonicalize()?)
        );
        assert_eq!(
            context.bundled_tool(WINDOWS_SANDBOX_SETUP),
            Some(setup.canonicalize()?)
        );
        let activated = context
            .path_with_bundled_tools(None)
            .unwrap()
            .expect("public tools should activate PATH");
        assert_eq!(
            std::env::split_paths(&activated).collect::<Vec<_>>(),
            vec![path_dir.canonicalize()?]
        );
        assert!(!path_dir.join(WINDOWS_SANDBOX_RUNNER.name).exists());
        assert!(!path_dir.join(WINDOWS_SANDBOX_SETUP.name).exists());
        Ok(())
    }

    #[test]
    fn existing_package_path_is_not_duplicated() -> std::io::Result<()> {
        let package = tempfile::tempdir()?;
        let app = package.path().join("clark-desktop");
        std::fs::write(&app, b"app")?;
        std::fs::write(package.path().join(RIPGREP.name), b"rg")?;
        let context = InstallContext::from_exe(Some(&app));
        let dir = package.path().canonicalize()?;
        let existing = std::env::join_paths([dir.clone(), PathBuf::from("/system")]).unwrap();

        let updated = context
            .path_with_bundled_tools(Some(existing))
            .unwrap()
            .unwrap();
        assert_eq!(
            std::env::split_paths(&updated).collect::<Vec<_>>(),
            vec![dir, PathBuf::from("/system")]
        );
        Ok(())
    }
}
