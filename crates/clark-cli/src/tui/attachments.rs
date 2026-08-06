use std::fs;
use std::path::{Path, PathBuf};

use agent_core::{AttachmentKind, ContentBlock, PendingUpload, PromptInput, ProviderCapabilities};
use base64::Engine as _;
use serde::Deserialize;

const MAX_ATTACHMENTS: usize = 8;
const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct EditorContext {
    filename: String,
    line: Option<u32>,
    column: Option<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct AttachmentInput {
    root: PathBuf,
    supported: Vec<AttachmentKind>,
    pending: Vec<(PathBuf, PendingUpload)>,
    candidates: Vec<PathBuf>,
    editor: Option<EditorContext>,
}

impl Default for AttachmentInput {
    fn default() -> Self {
        Self::new(Path::new("."), Vec::new())
    }
}

impl AttachmentInput {
    #[cfg(test)]
    pub(crate) fn handles_line(line: &str) -> bool {
        parse(line).is_some()
    }

    pub(crate) fn execute(
        &mut self,
        line: &str,
        root: &Path,
        capabilities: &ProviderCapabilities,
    ) -> Option<Result<String, String>> {
        let command = parse(line)?;
        self.root = root.to_path_buf();
        self.supported = capabilities.attachment_kinds.clone();
        Some(
            match command {
                Err(error) => Err(error),
                Ok(AttachmentCommand::Files(query)) => self.files(&query),
                Ok(AttachmentCommand::Ide(Some(specification))) => self.ide(Some(&specification)),
                Ok(AttachmentCommand::Ide(None)) => self.ide_from_host(),
            }
            .map_err(|error| format!("{error} Rejected locally; no provider turn started.")),
        )
    }

    pub(crate) fn count(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn new(root: &Path, supported: Vec<AttachmentKind>) -> Self {
        Self {
            root: root.to_path_buf(),
            supported,
            pending: Vec::new(),
            candidates: Vec::new(),
            editor: None,
        }
    }

    pub(crate) fn files(&mut self, query: &str) -> Result<String, String> {
        let query = query.trim();
        if query.is_empty() {
            return Err(
                "Usage: /attach FILE_OR_QUERY · /attach NUMBER · /attach clear · /attach --ide [PATH@LINE:COLUMN]"
                    .into(),
            );
        }
        if query == "clear" {
            self.pending.clear();
            self.candidates.clear();
            self.editor = None;
            return Ok("Cleared pending attachments and editor context.".into());
        }
        if let Ok(index) = query.parse::<usize>() {
            let path = self
                .candidates
                .get(index.saturating_sub(1))
                .cloned()
                .ok_or_else(|| format!("No fuzzy file result numbered {index}."))?;
            return self.attach(&path, None);
        }
        let path = self.root.join(query);
        if path.is_file() {
            return self.attach(&path, None);
        }
        self.candidates = super::attachment_search::fuzzy_files(&self.root, query)?;
        if self.candidates.is_empty() {
            return Err(format!("No project file matches `{query}`."));
        }
        let rows = self
            .candidates
            .iter()
            .enumerate()
            .map(|(index, path)| format!("{}. {}", index + 1, relative_label(&self.root, path)))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(format!(
            "Clark file matches for `{query}`\n{rows}\nUse /attach NUMBER to attach one."
        ))
    }

    pub(crate) fn ide(&mut self, specification: Option<&str>) -> Result<String, String> {
        let specification = specification
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| std::env::var("CLARK_IDE_FILE").ok())
            .ok_or_else(|| {
                "Usage: /attach --ide PATH[@LINE[:COLUMN]], or set CLARK_IDE_FILE for headless editor context."
                    .to_string()
            })?;
        let (path, line, column) = parse_editor_spec(&specification)?;
        let report = self.attach(&self.root.join(&path), Some((line, column)))?;
        Ok(format!(
            "{report}\nEditor cursor: {path}{}",
            cursor_label(line, column)
        ))
    }

    pub(crate) fn prompt(&self, text: String) -> PromptInput {
        let mut blocks = vec![ContentBlock::text(text)];
        if let Some(editor) = &self.editor {
            blocks.push(ContentBlock::text(format!(
                "[Clark editor context]\nFile: {}{}",
                editor.filename,
                cursor_label(editor.line, editor.column)
            )));
        }
        PromptInput {
            blocks,
            attachments: self
                .pending
                .iter()
                .map(|(_, upload)| upload.clone())
                .collect(),
        }
    }

    pub(crate) fn submission_label(&self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        format!(
            " · attachments: {}",
            self.pending
                .iter()
                .map(|(_, upload)| upload.filename.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    pub(crate) fn clear_after_start(&mut self) {
        self.pending.clear();
        self.candidates.clear();
        self.editor = None;
    }

    fn ide_from_host(&mut self) -> Result<String, String> {
        let manifest = self.root.join(".clark/ide-context.json");
        if manifest.is_file() {
            let bytes = fs::read(&manifest).map_err(|error| {
                format!(
                    "Could not read editor context {}: {error}",
                    manifest.display()
                )
            })?;
            let context: IdeContextManifest = serde_json::from_slice(&bytes).map_err(|error| {
                format!("Invalid editor context {}: {error}", manifest.display())
            })?;
            if context.files.is_empty() {
                return Err(format!(
                    "Editor context {} has no files.",
                    manifest.display()
                ));
            }
            let mut staged = self.clone();
            let mut reports = Vec::new();
            for file in context.files {
                let path = staged.root.join(&file.path);
                reports.push(staged.attach(&path, Some((file.line, file.column)))?);
            }
            *self = staged;
            return Ok(format!(
                "Attached Clark editor context atomically:\n{}",
                reports.join("\n")
            ));
        }
        self.ide(None)
    }

    fn attach(
        &mut self,
        path: &Path,
        editor: Option<(Option<u32>, Option<u32>)>,
    ) -> Result<String, String> {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Could not open attachment {}: {error}", path.display()))?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| format!("Could not resolve project root: {error}"))?;
        if !canonical.starts_with(&root) {
            return Err(format!(
                "Attachment {} escapes the active project root {}.",
                canonical.display(),
                root.display()
            ));
        }
        if !canonical.is_file() {
            return Err(format!("Attachment is not a file: {}", canonical.display()));
        }
        if self
            .pending
            .iter()
            .any(|(existing, _)| existing == &canonical)
        {
            return Err(format!(
                "Attachment is already pending: {}",
                relative_label(&root, &canonical)
            ));
        }
        if self.pending.len() >= MAX_ATTACHMENTS {
            return Err(format!(
                "At most {MAX_ATTACHMENTS} files may be attached to one turn."
            ));
        }
        let metadata = fs::metadata(&canonical)
            .map_err(|error| format!("Could not inspect {}: {error}", canonical.display()))?;
        if metadata.len() > MAX_ATTACHMENT_BYTES {
            return Err(format!(
                "Attachment is {} bytes; the Clark TUI limit is {MAX_ATTACHMENT_BYTES} bytes per file.",
                metadata.len()
            ));
        }
        let existing_bytes = self
            .pending
            .iter()
            .filter_map(|(path, _)| fs::metadata(path).ok().map(|metadata| metadata.len()))
            .sum::<u64>();
        if existing_bytes.saturating_add(metadata.len()) > MAX_TOTAL_BYTES {
            return Err(format!(
                "Pending attachments would exceed the {MAX_TOTAL_BYTES}-byte per-turn limit."
            ));
        }
        let filename = relative_label(&root, &canonical);
        let content_type = content_type(&canonical);
        let kind = attachment_kind(&filename, content_type);
        if !self.supported.contains(&kind) {
            return Err(format!(
                "This Clark provider does not advertise {} attachments; `{filename}` was not read or submitted.",
                kind_label(kind)
            ));
        }
        let bytes = fs::read(&canonical)
            .map_err(|error| format!("Could not read {}: {error}", canonical.display()))?;
        let upload = PendingUpload {
            filename: filename.clone(),
            content_type: content_type.into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        };
        self.pending.push((canonical, upload));
        if let Some((line, column)) = editor {
            self.editor = Some(EditorContext {
                filename: filename.clone(),
                line,
                column,
            });
        }
        Ok(format!(
            "Attached `{filename}` as {content_type}. It will be submitted once with the next user turn."
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AttachmentCommand {
    Files(String),
    Ide(Option<String>),
}

#[derive(Debug, Deserialize)]
struct IdeContextManifest {
    files: Vec<IdeContextFile>,
}

#[derive(Debug, Deserialize)]
struct IdeContextFile {
    path: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    column: Option<u32>,
}

pub(crate) fn parse(line: &str) -> Option<Result<AttachmentCommand, String>> {
    let command = line.trim().strip_prefix('/')?;
    let (name, rest) = command
        .split_once(char::is_whitespace)
        .map_or((command, ""), |(name, rest)| (name, rest.trim()));
    if name != "attach" {
        return None;
    }
    if rest == "--ide" {
        Some(Ok(AttachmentCommand::Ide(None)))
    } else if let Some(specification) = rest.strip_prefix("--ide ") {
        Some(Ok(AttachmentCommand::Ide(Some(
            specification.trim().into(),
        ))))
    } else {
        Some(Ok(AttachmentCommand::Files(rest.into())))
    }
}

fn parse_editor_spec(value: &str) -> Result<(String, Option<u32>, Option<u32>), String> {
    let Some((path, location)) = value.rsplit_once('@') else {
        return Ok((value.into(), None, None));
    };
    let mut parts = location.split(':');
    let line = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| "Editor line must be a positive integer.".to_string())?;
    let column = parts
        .next()
        .map(|value| {
            value
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| "Editor column must be a positive integer.".to_string())
        })
        .transpose()?;
    if parts.next().is_some() || path.is_empty() {
        return Err("Usage: /attach --ide PATH[@LINE[:COLUMN]]".into());
    }
    Ok((path.into(), Some(line), column))
}

fn cursor_label(line: Option<u32>, column: Option<u32>) -> String {
    match (line, column) {
        (Some(line), Some(column)) => format!(" · line {line}, column {column}"),
        (Some(line), None) => format!(" · line {line}"),
        _ => String::new(),
    }
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "json" => "application/json",
        "xml" => "application/xml",
        "js" | "mjs" | "cjs" => "application/javascript",
        "txt" | "md" | "rs" | "toml" | "yaml" | "yml" | "ts" | "tsx" | "jsx" | "css" | "html"
        | "py" | "sh" | "zsh" | "go" | "java" | "kt" | "swift" | "c" | "h" | "cpp" | "hpp"
        | "sql" | "csv" => "text/plain",
        _ => "application/octet-stream",
    }
}

fn attachment_kind(filename: &str, content_type: &str) -> AttachmentKind {
    PendingUpload {
        filename: filename.into(),
        content_type: content_type.into(),
        data_base64: String::new(),
    }
    .kind()
}

fn kind_label(kind: AttachmentKind) -> &'static str {
    match kind {
        AttachmentKind::Text => "text",
        AttachmentKind::Image => "image",
        AttachmentKind::Audio => "audio",
        AttachmentKind::Pdf => "PDF",
        AttachmentKind::Docx => "DOCX",
        AttachmentKind::Binary => "binary",
    }
}

fn relative_label(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
