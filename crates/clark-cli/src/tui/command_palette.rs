#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
}

impl CommandSpec {
    const fn new(name: &'static str, description: &'static str) -> Self {
        Self { name, description }
    }
}

pub(crate) const COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("attach", "Attach an exact or fuzzy-matched project file"),
    CommandSpec::new("clear", "Clear the visible transcript"),
    CommandSpec::new("goal", "Inspect or resume a durable Clark goal"),
    CommandSpec::new("init", "Create project AGENTS.md guidance"),
    CommandSpec::new("model", "Choose a provider-advertised Clark model"),
    CommandSpec::new(
        "permissions",
        "Inspect or choose a persistent permission profile",
    ),
    CommandSpec::new("quit", "Close this Clark session"),
    CommandSpec::new(
        "status",
        "Show authentication, workspace, provider, and sync status",
    ),
];

pub(crate) fn is_tui_command(name: &str) -> bool {
    COMMANDS.iter().any(|command| command.name == name)
}

#[derive(Debug, Default)]
pub(crate) struct CommandPalette {
    query: Option<String>,
    selected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PaletteRow {
    pub(crate) spec: &'static CommandSpec,
    pub(crate) selected: bool,
}

impl CommandPalette {
    pub(crate) fn sync(&mut self, query: Option<&str>) {
        if self.query.as_deref() != query {
            self.query = query.map(str::to_owned);
            self.selected = 0;
        }
        self.clamp_selection();
    }

    pub(crate) fn is_open(&self) -> bool {
        self.query.is_some()
    }

    pub(crate) fn select_previous(&mut self) -> bool {
        if !self.is_open() || self.selected == 0 || self.matches().is_empty() {
            return false;
        }
        self.selected -= 1;
        true
    }

    pub(crate) fn select_next(&mut self) -> bool {
        let count = self.matches().len();
        if !self.is_open() || self.selected + 1 >= count {
            return false;
        }
        self.selected += 1;
        true
    }

    pub(crate) fn selected(&self) -> Option<&'static CommandSpec> {
        self.matches().get(self.selected).copied()
    }

    pub(crate) fn rows(&self, max_rows: usize) -> Vec<PaletteRow> {
        let first = self.selected.saturating_add(1).saturating_sub(max_rows);
        self.matches()
            .into_iter()
            .skip(first)
            .take(max_rows)
            .enumerate()
            .map(|(index, spec)| PaletteRow {
                spec,
                selected: first + index == self.selected,
            })
            .collect()
    }

    fn matches(&self) -> Vec<&'static CommandSpec> {
        let Some(query) = self.query.as_deref() else {
            return Vec::new();
        };
        let query = query.to_ascii_lowercase();
        let mut matches = COMMANDS
            .iter()
            .filter_map(|spec| {
                let name = spec.name.to_ascii_lowercase();
                let description = spec.description.to_ascii_lowercase();
                let rank = if name == query {
                    0
                } else if name.starts_with(&query) {
                    1
                } else if name.contains(&query) {
                    2
                } else if description.contains(&query) {
                    3
                } else {
                    return None;
                };
                Some((rank, spec.name, spec))
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(rank, name, _)| (*rank, *name));
        matches.into_iter().map(|(_, _, spec)| spec).collect()
    }

    fn clamp_selection(&mut self) {
        let count = self.matches().len();
        self.selected = self.selected.min(count.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn catalog_is_complete_and_unique() {
        assert_eq!(COMMANDS.len(), 8);
        let names = COMMANDS
            .iter()
            .map(|spec| spec.name)
            .collect::<HashSet<_>>();
        assert_eq!(names.len(), COMMANDS.len());
        assert!(COMMANDS.iter().all(|spec| !spec.description.is_empty()));
        assert_eq!(
            names,
            HashSet::from([
                "attach",
                "clear",
                "goal",
                "init",
                "model",
                "permissions",
                "quit",
                "status",
            ])
        );
        for excluded in ["vim", "theme", "pets", "plugins", "feedback", "btw"] {
            assert!(!is_tui_command(excluded));
        }
    }

    #[test]
    fn exact_and_prefix_matches_rank_before_description_matches() {
        let mut palette = CommandPalette::default();
        palette.sync(Some("status"));
        assert_eq!(palette.selected().map(|spec| spec.name), Some("status"));
        palette.sync(Some("sta"));
        assert_eq!(palette.selected().map(|spec| spec.name), Some("status"));
        assert!(palette.rows(5).iter().any(|row| row.spec.name == "status"));
    }

    #[test]
    fn selection_is_deterministic_and_resets_with_the_query() {
        let mut palette = CommandPalette::default();
        palette.sync(Some("s"));
        let first = palette.selected().expect("a match").name;
        assert!(palette.select_next());
        assert_ne!(palette.selected().expect("a match").name, first);
        assert!(palette.select_previous());
        assert_eq!(palette.selected().expect("a match").name, first);
        palette.sync(Some("status"));
        assert_eq!(palette.selected().expect("a match").name, "status");
    }

    #[test]
    fn closing_palette_removes_selection() {
        let mut palette = CommandPalette::default();
        palette.sync(Some("model"));
        assert!(palette.is_open());
        palette.sync(None);
        assert!(!palette.is_open());
        assert_eq!(palette.selected(), None);
    }

    #[test]
    fn visible_rows_follow_a_selection_beyond_the_first_page() {
        let mut palette = CommandPalette::default();
        palette.sync(Some(""));
        for _ in 0..7 {
            assert!(palette.select_next());
        }
        let rows = palette.rows(5);
        assert_eq!(rows.len(), 5);
        assert!(rows.last().expect("selected row").selected);
    }
}
