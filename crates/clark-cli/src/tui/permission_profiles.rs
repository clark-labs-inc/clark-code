use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PermissionProfile {
    #[default]
    Prompt,
    ReadOnly,
    WorkspaceWrite,
}

impl PermissionProfile {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "prompt" | "ask" => Some(Self::Prompt),
            "read-only" | "readonly" => Some(Self::ReadOnly),
            "workspace-write" | "workspace" => Some(Self::WorkspaceWrite),
            _ => None,
        }
    }

    pub(crate) fn mode_for(self, tool: &str) -> &'static str {
        match (self, tool) {
            (Self::Prompt, _) => "ask",
            (Self::ReadOnly, _) => "deny",
            (Self::WorkspaceWrite, "write_file" | "edit_file") => "allow",
            (Self::WorkspaceWrite, _) => "ask",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PermissionProfileState {
    profile: PermissionProfile,
    sandbox_required: bool,
    read_roots: Vec<PathBuf>,
}

impl Default for PermissionProfileState {
    fn default() -> Self {
        Self {
            profile: PermissionProfile::Prompt,
            sandbox_required: true,
            read_roots: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PermissionProfileEffect {
    pub(crate) status: String,
    pub(crate) transcript: String,
    pub(crate) changed: bool,
}

impl PermissionProfileState {
    pub(crate) fn path(cwd: &Path) -> PathBuf {
        directories::BaseDirs::new().map_or_else(
            || cwd.join(".clark/permissions.conf"),
            |directories| directories.config_dir().join("clark/permissions.conf"),
        )
    }

    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(contents) => Self::decode(&contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!(
                "could not read Clark permission profiles {}: {error}",
                path.display()
            )),
        }
    }

    pub(crate) fn save(&self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Clark permission profile path has no parent".to_string())?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create Clark permission profile directory {}: {error}",
                parent.display()
            )
        })?;
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, self.encode()).map_err(|error| {
            format!(
                "could not write Clark permission profile {}: {error}",
                temporary.display()
            )
        })?;
        fs::rename(&temporary, path).map_err(|error| {
            format!(
                "could not replace Clark permission profile {}: {error}",
                path.display()
            )
        })
    }

    pub(crate) fn profile(&self) -> PermissionProfile {
        self.profile
    }

    pub(crate) fn sandbox_mode(&self) -> &'static str {
        if self.sandbox_required {
            "required"
        } else {
            "disabled"
        }
    }

    pub(crate) fn read_roots(&self) -> &[PathBuf] {
        &self.read_roots
    }

    pub(crate) fn handles_line(line: &str) -> bool {
        line.trim()
            .strip_prefix('/')
            .and_then(|line| line.split_whitespace().next())
            == Some("permissions")
    }

    pub(crate) fn execute(&mut self, line: &str, cwd: &Path) -> Option<PermissionProfileEffect> {
        let line = line.trim().strip_prefix('/')?;
        let (command, argument) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(command, argument)| (command, argument.trim()));
        if command != "permissions" {
            return None;
        }
        Some(if argument == "reset-sandbox" {
            self.setup_default_sandbox()
        } else if let Some(path) = argument.strip_prefix("add-read-dir ") {
            self.add_read_root(path.trim(), cwd)
        } else {
            self.choose_profile(argument)
        })
    }

    pub(crate) fn inspect(&self) -> String {
        let read_roots = if self.read_roots.is_empty() {
            "none".into()
        } else {
            self.read_roots
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "Clark permission profile\nProfile: {}\nShell: {}\nFile writes: {}\nSandbox: {}\nAdditional readable roots: {read_roots}\nThe current provider session is unchanged; changes to provider permissions and roots apply to the next session.",
            self.profile.name(),
            self.profile.mode_for("bash"),
            self.profile.mode_for("write_file"),
            self.sandbox_mode(),
        )
    }

    fn choose_profile(&mut self, argument: &str) -> PermissionProfileEffect {
        if argument.is_empty() {
            return PermissionProfileEffect {
                status: "permission profile inspected".into(),
                transcript: format!(
                    "{}\nUsage: /permissions prompt|read-only|workspace-write · /permissions add-read-dir PATH · /permissions reset-sandbox",
                    self.inspect()
                ),
                changed: false,
            };
        }
        let Some(profile) = PermissionProfile::parse(argument) else {
            return unchanged(
                "permission profile unchanged",
                "Usage: /permissions prompt|read-only|workspace-write · /permissions add-read-dir PATH · /permissions reset-sandbox",
            );
        };
        let changed = self.profile != profile;
        self.profile = profile;
        PermissionProfileEffect {
            status: format!("permission profile · {}", profile.name()),
            transcript: format!(
                "Permission profile set to {} for the next session. The current provider session is unchanged.\n{}",
                profile.name(),
                self.inspect()
            ),
            changed,
        }
    }

    fn setup_default_sandbox(&mut self) -> PermissionProfileEffect {
        let changed = !self.sandbox_required;
        self.sandbox_required = true;
        PermissionProfileEffect {
            status: "default sandbox required".into(),
            transcript: format!(
                "Clark's default workspace sandbox is required for future sessions. The current session's sandbox cannot be replaced while tools are running.\n{}",
                self.inspect()
            ),
            changed,
        }
    }

    fn add_read_root(&mut self, argument: &str, cwd: &Path) -> PermissionProfileEffect {
        if argument.is_empty() {
            return unchanged(
                "sandbox read roots inspected",
                &format!(
                    "{}\nUsage: /permissions add-read-dir /absolute/path",
                    self.inspect()
                ),
            );
        }
        let candidate = PathBuf::from(argument);
        let absolute = if candidate.is_absolute() {
            candidate
        } else {
            cwd.join(candidate)
        };
        let canonical = match absolute.canonicalize() {
            Ok(path) if path.is_dir() => path,
            Ok(path) => {
                return unchanged(
                    "sandbox read root rejected",
                    &format!("{} is not a directory", path.display()),
                );
            }
            Err(error) => {
                return unchanged(
                    "sandbox read root rejected",
                    &format!("could not resolve {}: {error}", absolute.display()),
                );
            }
        };
        if self.read_roots.contains(&canonical) {
            return unchanged(
                "sandbox read root already present",
                &format!("{} is already configured", canonical.display()),
            );
        }
        self.read_roots.push(canonical.clone());
        self.read_roots.sort();
        PermissionProfileEffect {
            status: "sandbox read root added".into(),
            transcript: format!(
                "Added {} as a readable root for the next local provider session. It does not grant write access and the current session is unchanged.",
                canonical.display()
            ),
            changed: true,
        }
    }

    fn encode(&self) -> String {
        let mut lines = vec![
            "version=1".to_string(),
            format!("profile={}", self.profile.name()),
            format!("sandbox_required={}", self.sandbox_required),
        ];
        lines.extend(
            self.read_roots
                .iter()
                .map(|path| format!("read_root={}", path.display())),
        );
        format!("{}\n", lines.join("\n"))
    }

    fn decode(contents: &str) -> Result<Self, String> {
        let mut state = Self::default();
        let mut version = None;
        let mut profile = None;
        let mut sandbox_required = None;
        state.read_roots.clear();
        for line in contents.lines() {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("invalid Clark permission profile line {line:?}"))?;
            match key {
                "version" => version = Some(value),
                "profile" => {
                    profile = PermissionProfile::parse(value)
                        .ok_or_else(|| format!("unknown permission profile {value:?}"))?
                        .into();
                }
                "sandbox_required" => {
                    sandbox_required = match value {
                        "true" => Some(true),
                        "false" => Some(false),
                        _ => return Err(format!("invalid sandbox_required value {value:?}")),
                    };
                }
                "read_root" => state.read_roots.push(PathBuf::from(value)),
                _ => return Err(format!("unknown Clark permission setting {key:?}")),
            }
        }
        if version != Some("1") {
            return Err("unsupported or missing Clark permission profile version".into());
        }
        state.profile = profile.ok_or("Clark permission profile is missing profile")?;
        state.sandbox_required =
            sandbox_required.ok_or("Clark permission profile is missing sandbox_required")?;
        state.read_roots.sort();
        state.read_roots.dedup();
        Ok(state)
    }
}

fn unchanged(status: &str, transcript: &str) -> PermissionProfileEffect {
    PermissionProfileEffect {
        status: status.into(),
        transcript: transcript.into(),
        changed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "clark-permission-profile-{label}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn profiles_have_distinct_fail_closed_tool_modes() {
        assert_eq!(PermissionProfile::Prompt.mode_for("bash"), "ask");
        assert_eq!(PermissionProfile::ReadOnly.mode_for("write_file"), "deny");
        assert_eq!(
            PermissionProfile::WorkspaceWrite.mode_for("edit_file"),
            "allow"
        );
        assert_eq!(PermissionProfile::WorkspaceWrite.mode_for("bash"), "ask");
    }

    #[test]
    fn profile_and_read_roots_round_trip_durably() {
        let directory = temporary_directory("roundtrip");
        let root = directory.join("extra");
        fs::create_dir_all(&root).unwrap();
        let path = directory.join("permissions.conf");
        let mut state = PermissionProfileState::default();
        assert!(
            state
                .execute("/permissions workspace-write", &directory)
                .unwrap()
                .changed
        );
        assert!(
            state
                .execute(
                    &format!("/permissions add-read-dir {}", root.display()),
                    &directory
                )
                .unwrap()
                .changed
        );
        state.save(&path).unwrap();
        let restored = PermissionProfileState::load(&path).unwrap();
        assert_eq!(restored, state);
        assert!(restored.inspect().contains("current provider session"));
    }

    #[test]
    fn read_root_requires_an_existing_directory_and_deduplicates() {
        let directory = temporary_directory("validation");
        let mut state = PermissionProfileState::default();
        assert!(
            !state
                .execute("/permissions add-read-dir missing", &directory)
                .unwrap()
                .changed
        );
        assert!(
            state
                .execute("/permissions add-read-dir .", &directory)
                .unwrap()
                .changed
        );
        assert!(
            !state
                .execute("/permissions add-read-dir .", &directory)
                .unwrap()
                .changed
        );
        assert_eq!(state.read_roots().len(), 1);
    }

    #[test]
    fn command_detection_never_captures_an_ordinary_prompt() {
        assert!(PermissionProfileState::handles_line(
            "/permissions read-only"
        ));
        assert!(PermissionProfileState::handles_line(
            "/permissions add-read-dir /tmp"
        ));
        assert!(!PermissionProfileState::handles_line(
            "/sandbox-add-read-dir /tmp"
        ));
        assert!(!PermissionProfileState::handles_line(
            "please explain permissions"
        ));
        assert!(!PermissionProfileState::handles_line("/permissionless"));
    }
}
