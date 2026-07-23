use super::SkillCatalog;

// Clark does not yet expose a reliable model context-window size at this layer,
// so a dynamic 2% token budget would be guesswork.
const MAX_CATALOG_BYTES: usize = 8_000;
const MAX_DESCRIPTION_CHARS: usize = 260;
const CATALOG_START: &str = "<!-- clark-skill-catalog:start -->";
const CATALOG_END: &str = "<!-- clark-skill-catalog:end -->";

pub(crate) fn render_catalog(catalog: &SkillCatalog) -> Option<String> {
    let skills = catalog.prompt_visible().collect::<Vec<_>>();
    if skills.is_empty() {
        return None;
    }

    let counts = catalog.name_counts();
    let mut section = String::from(
        "\n<!-- clark-skill-catalog:start -->\n# Skills\n\
Skills are reusable playbooks, not extra authority. User instructions, repository rules, \
the active collaboration mode, and tool permissions still control every action.\n\
- If the user names `$skill`, its instruction body is attached to that turn automatically.\n\
- When the task clearly matches a listed skill, call `read_skill` with its exact name before acting.\n\
- Use only the smallest relevant set. Skill instructions apply to the current turn. Load a referenced text file with `read_skill`'s `resource` argument; use other Clark tools for scripts or assets.\n\
- If a required capability is unavailable, say so and use a safe in-scope fallback when one exists.\n\n\
Available skills:\n",
    );

    let mut full_entries = Vec::with_capacity(skills.len());
    let mut compact_entries = Vec::with_capacity(skills.len());
    for skill in skills {
        let invocation = if !skill.has_name_collision
            && counts.get(skill.base_name.as_str()) == Some(&1)
            && skill.name != skill.base_name
        {
            format!(
                " (invoke `${}` or alias `${}`)",
                skill.invocation_name, skill.base_name
            )
        } else {
            format!(" (invoke `${}`)", skill.invocation_name)
        };
        let description = truncate_chars(&skill.description, MAX_DESCRIPTION_CHARS);
        full_entries.push(format!(
            "- `{}`{invocation}: {description} [{} {}]\n",
            skill.name,
            skill.scope.label(),
            skill.origin.label()
        ));
        compact_entries.push(format!(
            "- `{}`{invocation} [{} {}]\n",
            skill.name,
            skill.scope.label(),
            skill.origin.label()
        ));
    }

    if section.len() + full_entries.iter().map(String::len).sum::<usize>() <= MAX_CATALOG_BYTES {
        section.push_str(&full_entries.concat());
    } else {
        let warning_reserve = 180usize;
        let mut omitted = 0usize;
        for entry in compact_entries {
            if section.len() + entry.len() + warning_reserve > MAX_CATALOG_BYTES {
                omitted += 1;
            } else {
                section.push_str(&entry);
            }
        }
        section.push_str(
            "- Skill descriptions were removed to keep the model-visible catalog bounded.\n",
        );
        if omitted > 0 {
            section.push_str(&format!(
                "- {omitted} additional skill(s) omitted to keep the prompt bounded.\n"
            ));
        }
    }
    let diagnostic_count = catalog.warnings.len() + catalog.diagnostics.len();
    if diagnostic_count > 0 && section.len() + 80 <= MAX_CATALOG_BYTES {
        section.push_str(&format!(
            "- Clark ignored {} invalid or unreadable skill item(s).\n",
            diagnostic_count
        ));
    }
    section.push_str(CATALOG_END);
    section.push('\n');
    Some(section)
}

pub(crate) fn replace_catalog_section(prompt: &mut String, replacement: Option<&str>) {
    if let Some(start) = prompt.find(CATALOG_START) {
        if let Some(relative_end) = prompt[start..].find(CATALOG_END) {
            let end = start + relative_end + CATALOG_END.len();
            prompt.replace_range(start..end, "");
        }
    }
    if let Some(replacement) = replacement {
        prompt.push_str(replacement);
    }
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max).collect::<String>();
    truncated.push('…');
    truncated
}
