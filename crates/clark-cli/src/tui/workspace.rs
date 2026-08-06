use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const AGENTS_GUIDANCE: &str = "# AGENTS.md\n\n## Working in this project\n\n- Read this repository's documentation and tests before changing behavior.\n- Preserve unrelated user changes and report conflicts instead of overwriting them.\n- Keep credentials and generated secrets out of tracked files.\n- Verify focused changes with the smallest relevant tests before reporting completion.\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceInspection {
    AlreadyExists(PathBuf),
    Preview(WorkspaceInitialization),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceInitialization {
    pub(crate) path: PathBuf,
    content: String,
}

impl WorkspaceInitialization {
    pub(crate) fn inspect(root: &Path) -> Result<WorkspaceInspection, String> {
        let path = root.join("AGENTS.md");
        match fs::symlink_metadata(&path) {
            Ok(_) => Ok(WorkspaceInspection::AlreadyExists(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(WorkspaceInspection::Preview(Self {
                    path,
                    content: AGENTS_GUIDANCE.into(),
                }))
            }
            Err(error) => Err(format!(
                "could not inspect project guidance {}: {error}",
                path.display()
            )),
        }
    }

    pub(crate) fn preview_lines(&self, limit: usize) -> Vec<&str> {
        self.content.lines().take(limit).collect()
    }

    pub(crate) fn desired_height(&self) -> u16 {
        u16::try_from(self.preview_lines(6).len().saturating_add(4))
            .unwrap_or(10)
            .min(12)
    }

    pub(crate) fn confirm(&self) -> Result<String, String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    format!(
                        "Clark refused to overwrite existing project guidance {}",
                        self.path.display()
                    )
                } else {
                    format!(
                        "could not create project guidance {}: {error}",
                        self.path.display()
                    )
                }
            })?;
        file.write_all(self.content.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                format!(
                    "could not finish project guidance {}: {error}",
                    self.path.display()
                )
            })?;
        Ok(format!("Created project guidance {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn test_root() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "clark-tui-workspace-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn preview_names_the_exact_project_local_operation() {
        let root = Path::new("/workspace/example");
        let initialization = WorkspaceInitialization {
            path: root.join("AGENTS.md"),
            content: AGENTS_GUIDANCE.into(),
        };
        assert_eq!(initialization.path, root.join("AGENTS.md"));
        assert_eq!(initialization.preview_lines(2)[0], "# AGENTS.md");
        assert!(initialization
            .content
            .contains("Preserve unrelated user changes"));
    }

    #[test]
    fn confirmation_creates_once_and_never_overwrites() {
        let root = test_root();
        fs::create_dir(&root).expect("create test root");
        let WorkspaceInspection::Preview(initialization) =
            WorkspaceInitialization::inspect(&root).expect("inspect")
        else {
            panic!("new root should produce preview")
        };
        let receipt = initialization.confirm().expect("confirm");
        assert!(receipt.contains("AGENTS.md"));
        assert_eq!(
            fs::read_to_string(root.join("AGENTS.md")).expect("read guidance"),
            AGENTS_GUIDANCE
        );
        assert!(matches!(
            WorkspaceInitialization::inspect(&root).expect("inspect again"),
            WorkspaceInspection::AlreadyExists(_)
        ));
        assert!(initialization
            .confirm()
            .unwrap_err()
            .contains("refused to overwrite"));
        fs::remove_dir_all(&root).expect("remove owned test root");
    }
}
