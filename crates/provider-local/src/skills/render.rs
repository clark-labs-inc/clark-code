use super::SkillCatalog;

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

    let mut entries = Vec::with_capacity(skills.len());
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
        entries.push(format!(
            "- `{}`{invocation}: {} [{} {}]\n",
            skill.name,
            skill.description,
            skill.scope.label(),
            skill.origin.label()
        ));
    }

    section.push_str(&entries.concat());
    let diagnostic_count = catalog.warnings.len() + catalog.diagnostics.len();
    if diagnostic_count > 0 {
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
