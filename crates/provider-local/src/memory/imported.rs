use std::path::Path;

use sha2::{Digest, Sha256};

use super::{
    iso_date_today, parse_frontmatter, read_text, single_line_ellipsis, slugify, strip_frontmatter,
    MemoryType, INDEX_FILE,
};
use crate::exec::Executor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportedMemoryChange {
    Created,
    Updated,
    Unchanged,
}

/// Create or refresh one externally owned memory note in a collision-resistant
/// namespace. The stable origin key determines the file name, so repeated
/// startup imports update only their own note and can never overwrite a note
/// created by Clark's agent or the user.
pub(crate) async fn upsert_imported_memory(
    exec: &dyn Executor,
    mem_dir: &Path,
    source: &str,
    origin_key: &str,
    title: &str,
    content: &str,
    kind: MemoryType,
) -> Result<ImportedMemoryChange, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("imported memory is empty".into());
    }
    let source_slug = slugify(source);
    if source_slug.is_empty() {
        return Err("imported memory source must contain letters or digits".into());
    }
    let digest = format!("{:x}", Sha256::digest(origin_key.as_bytes()));
    let file = format!("imported-{source_slug}-{}.md", &digest[..16]);
    let path = mem_dir.join(&file);
    let source_label = format!("imported-{source_slug}");
    let clean_title = title.trim().replace(['\n', '\r'], " ");
    let description = content
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(&clean_title)
        .trim()
        .replace(['\n', '\r'], " ");
    let description = single_line_ellipsis(&description, 160);

    let existing = read_text(exec, &path).await;
    if let Some(existing) = existing.as_deref() {
        let metadata = parse_frontmatter(existing);
        if metadata.source.as_deref() != Some(source_label.as_str()) {
            return Err(format!(
                "refusing to overwrite non-imported memory file {file}"
            ));
        }
        if metadata.name.as_deref() == Some(clean_title.as_str())
            && metadata.description.as_deref() == Some(description.as_str())
            && metadata.kind == Some(kind)
            && strip_frontmatter(existing).trim() == content
        {
            return Ok(ImportedMemoryChange::Unchanged);
        }
    }

    let body = format!(
        "---\nname: {}\ndescription: {}\nsaved: {}\nsource: {}\ntype: {}\n---\n\n{}\n",
        clean_title,
        description,
        iso_date_today(),
        source_label,
        kind.label(),
        content,
    );
    exec.create_dir_all(mem_dir)
        .await
        .map_err(|error| format!("creating imported memory dir: {error}"))?;
    exec.write(&path, body.as_bytes())
        .await
        .map_err(|error| format!("writing imported memory: {error}"))?;

    let index_path = mem_dir.join(INDEX_FILE);
    let index = read_text(exec, &index_path)
        .await
        .unwrap_or_else(|| "# Memory index\n".to_string());
    let pointer = format!("- [{}]({file}) — {}", clean_title, description);
    let mut replaced = false;
    let mut lines = index
        .lines()
        .map(|line| {
            if line.contains(&format!("]({file})")) {
                replaced = true;
                pointer.clone()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>();
    if !replaced {
        lines.push(pointer);
    }
    let mut index = lines.join("\n");
    index.push('\n');
    exec.write(&index_path, index.as_bytes())
        .await
        .map_err(|error| format!("writing imported memory index: {error}"))?;

    Ok(if existing.is_some() {
        ImportedMemoryChange::Updated
    } else {
        ImportedMemoryChange::Created
    })
}
