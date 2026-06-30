//! Fast project file listing — backs the desktop `@`-mention file picker.
//!
//! Walks the project root with the same ignore rules the file tools use
//! (`.git`, `node_modules`, `target`, …) and returns project-relative paths.
//! Read-only; never reads file contents.

use std::path::Path;

use exec_core::is_ignored;
use walkdir::WalkDir;

/// Cap the listing so a huge monorepo can't balloon the IPC payload. The picker
/// fuzzy-filters client-side, so this is a breadth bound on the walk, not a cap
/// on what the user can find once the list is loaded.
const MAX_FILES: usize = 5000;

/// Project-relative file paths under `root` (forward-slashed, sorted), skipping
/// ignored directories and capping at [`MAX_FILES`].
pub fn list_project_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_ignored(e.path()))
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
            if out.len() >= MAX_FILES {
                break;
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_files_and_skips_ignored_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "").unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("node_modules/x/y.js"), "").unwrap();

        let files = list_project_files(root);
        assert!(files.contains(&"a.rs".to_string()));
        assert!(files.contains(&"src/main.rs".to_string()));
        assert!(!files.iter().any(|f| f.contains("node_modules")));
        // Sorted output.
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted);
    }
}
