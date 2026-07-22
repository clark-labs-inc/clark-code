use std::collections::HashSet;

use super::{render_injection, SkillCatalog};
use crate::exec::Executor;

pub(crate) async fn explicit_skill_injections(
    exec: &dyn Executor,
    catalog: &SkillCatalog,
    user_request: &str,
) -> Vec<String> {
    let mut sections = Vec::new();
    let mut seen = HashSet::new();
    for requested in explicit_names(user_request) {
        let Ok(skill) = catalog.resolve_name(&requested) else {
            continue;
        };
        if !seen.insert(skill.name.clone()) {
            continue;
        }
        match catalog.read(exec, skill).await {
            Ok(contents) => sections.push(render_injection(skill, &contents)),
            Err(error) => sections.push(format!(
                "[runtime skill error]\nClark could not load explicitly requested skill `{}`: {error}",
                skill.name
            )),
        }
    }
    sections
}

fn explicit_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        let (start, character) = chars[index];
        if character != '$' {
            index += 1;
            continue;
        }
        let mut end = start + character.len_utf8();
        let mut cursor = index + 1;
        while cursor < chars.len() {
            let (offset, candidate) = chars[cursor];
            if candidate.is_ascii_alphanumeric() || matches!(candidate, '-' | '_' | ':') {
                end = offset + candidate.len_utf8();
                cursor += 1;
            } else {
                break;
            }
        }
        if end > start + 1 {
            names.push(text[start + 1..end].to_string());
        }
        index = cursor.max(index + 1);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::explicit_names;

    #[test]
    fn finds_qualified_mentions_and_ignores_bare_dollars() {
        assert_eq!(
            explicit_names("Use $github:gh-fix-ci, then $review_agent. Cost is $5."),
            vec!["github:gh-fix-ci", "review_agent", "5"]
        );
        assert!(explicit_names("$ is not a skill").is_empty());
    }
}
