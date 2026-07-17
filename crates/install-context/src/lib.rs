//! Discovers helper executables shipped beside Clark Code.
//!
//! Tauri external binaries are installed next to the application executable.
//! PATH-visible tools are prepended before the desktop runtime starts; private
//! helpers remain addressable by absolute path without shadowing system tools.

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
    pub exposure: ToolExposure,
}

pub const RIPGREP: BundledTool = BundledTool {
    name: if cfg!(windows) { "rg.exe" } else { "rg" },
    exposure: ToolExposure::Path,
};

/// Every helper the package layout knows about. New private runtime helpers
/// belong here even when they should not be exposed to model-issued commands.
pub const BUNDLED_TOOLS: &[BundledTool] = &[RIPGREP];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallContext {
    executable_dir: Option<PathBuf>,
}

impl InstallContext {
    pub fn from_exe(current_exe: Option<&Path>) -> Self {
        let executable_dir = current_exe
            .and_then(Path::parent)
            .map(|path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
        Self { executable_dir }
    }

    pub fn current() -> &'static Self {
        INSTALL_CONTEXT.get_or_init(|| Self::from_exe(std::env::current_exe().ok().as_deref()))
    }

    pub fn bundled_tool(&self, tool: BundledTool) -> Option<PathBuf> {
        let path = self.executable_dir.as_ref()?.join(tool.name);
        path.is_file().then_some(path)
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
        let Some(dir) = self.executable_dir.as_ref() else {
            return Ok(None);
        };
        if !BUNDLED_TOOLS
            .iter()
            .any(|tool| tool.exposure == ToolExposure::Path && self.bundled_tool(*tool).is_some())
        {
            return Ok(None);
        }

        let mut entries = vec![dir.clone()];
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
    std::env::set_var("PATH", path);
    Ok(context.executable_dir.clone())
}

pub fn rg_command() -> PathBuf {
    InstallContext::current().rg_command()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_rg_is_resolved_and_prepended() -> std::io::Result<()> {
        let package = tempfile::tempdir()?;
        let bin_dir = package.path().join("bin");
        std::fs::create_dir_all(&bin_dir)?;
        let app = bin_dir.join(if cfg!(windows) {
            "clark-desktop.exe"
        } else {
            "clark-desktop"
        });
        let rg = bin_dir.join(RIPGREP.name);
        std::fs::write(&app, b"app")?;
        std::fs::write(&rg, b"rg")?;

        let context = InstallContext::from_exe(Some(&app));
        assert_eq!(context.rg_command(), rg.canonicalize()?);
        let existing = std::env::join_paths([package.path().join("system")]).unwrap();
        let updated = context
            .path_with_bundled_tools(Some(existing))
            .unwrap()
            .expect("bundled rg should update PATH");
        let entries = std::env::split_paths(&updated).collect::<Vec<_>>();
        assert_eq!(entries[0], bin_dir.canonicalize()?);
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
