use std::fs;
use std::path::{Path, PathBuf};

const MAX_DISCOVERED_FILES: usize = 20_000;
const MAX_MATCHES: usize = 10;

pub(super) fn fuzzy_files(root: &Path, query: &str) -> Result<Vec<PathBuf>, String> {
    let mut stack = vec![root.to_path_buf()];
    let mut scored = Vec::new();
    let mut discovered = 0usize;
    while let Some(directory) = stack.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("Could not search {}: {error}", directory.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                if !ignored_directory(&path) {
                    stack.push(path);
                }
            } else if file_type.is_file() {
                discovered += 1;
                if discovered > MAX_DISCOVERED_FILES {
                    return Err(format!(
                        "File search stopped after {MAX_DISCOVERED_FILES} files; enter a narrower path."
                    ));
                }
                let label = relative_label(root, &path);
                if let Some(score) = fuzzy_score(query, &label) {
                    scored.push((score, label, path));
                }
            }
        }
    }
    scored.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.len().cmp(&right.1.len()))
            .then_with(|| left.1.cmp(&right.1))
    });
    Ok(scored
        .into_iter()
        .take(MAX_MATCHES)
        .map(|(_, _, path)| path)
        .collect())
}

fn fuzzy_score(query: &str, candidate: &str) -> Option<u32> {
    let query = query.to_ascii_lowercase();
    let candidate = candidate.to_ascii_lowercase();
    if let Some(index) = candidate.find(&query) {
        return Some(u32::try_from(index).unwrap_or(u32::MAX));
    }
    let mut score = 100u32;
    let mut cursor = 0usize;
    for character in query.chars() {
        let suffix = candidate.get(cursor..)?;
        let offset = suffix.find(character)?;
        score = score.saturating_add(u32::try_from(offset).unwrap_or(u32::MAX));
        cursor = cursor.saturating_add(offset + character.len_utf8());
    }
    Some(score)
}

fn ignored_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | "node_modules" | "target" | "dist"))
}

fn relative_label(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contiguous_matches_rank_before_sparse_subsequences() {
        assert!(
            fuzzy_score("parser", "src/parser.rs").unwrap()
                < fuzzy_score("parser", "src/p_a_r_s_e_r.rs").unwrap()
        );
        assert_eq!(fuzzy_score("missing", "src/parser.rs"), None);
    }
}
