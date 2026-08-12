//! Prompt and per-turn environment rendering helpers.

use agent_core::domain::ContentBlock;
use agent_core::provider::PromptInput;

use crate::sandbox::Sandbox;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct PromptParts {
    pub user_request: String,
    pub text_attachment_context: String,
}

pub(super) fn environment_context(sandbox: &Sandbox, remote: bool) -> String {
    let mut roots = vec![sandbox.root().display().to_string()];
    if let Some(docs) = sandbox.docs_root() {
        roots.push(docs.display().to_string());
    }
    let read_only_roots = sandbox
        .read_roots()
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>();
    let task_scope = sandbox
        .task_scope()
        .map(|scope| sandbox.display(scope))
        .unwrap_or_default();
    format!(
        "[runtime context — derived from the active session, not user instruction]\n\
<environment_context>\n  <cwd>{}</cwd>\n  <workspace_roots>{}</workspace_roots>\n  <read_only_roots>{}</read_only_roots>\n  <task_scope>{task_scope}</task_scope>\n  <remote>{remote}</remote>\n</environment_context>",
        sandbox.root().display(),
        roots.join(" | "),
        read_only_roots.join(" | "),
    )
}

/// Find an unambiguous existing directory named by the user as the place work
/// should happen. Only directory tokens immediately following scope language
/// qualify; unrelated path mentions or multiple sibling directories leave the
/// ordinary project root unchanged.
pub(super) fn explicit_task_scope(sandbox: &Sandbox, user_request: &str) -> Option<String> {
    fn clean_token(token: &str) -> &str {
        token
            .trim_matches(|character: char| {
                character.is_ascii_punctuation()
                    && character != '/'
                    && character != '\\'
                    && character != '-'
                    && character != '_'
                    && character != '.'
            })
            .trim_end_matches(['.', ',', ':', ';', '!', '?'])
    }

    let words = user_request.split_whitespace().collect::<Vec<_>>();
    let mut candidates = Vec::<String>::new();
    for (index, raw) in words.iter().enumerate() {
        if index == 0 {
            continue;
        }
        let previous = clean_token(words[index - 1]).to_ascii_lowercase();
        if !matches!(
            previous.as_str(),
            "in" | "inside" | "within" | "under" | "from"
        ) {
            continue;
        }
        let candidate = clean_token(raw);
        if candidate.is_empty()
            || candidate == "."
            || candidate.starts_with('/')
            || candidate.split(['/', '\\']).any(|part| part == "..")
        {
            continue;
        }
        let Ok(path) = sandbox.resolve_existing(candidate) else {
            continue;
        };
        if path.is_dir()
            && path != sandbox.root()
            && !candidates.iter().any(|item| item == candidate)
        {
            candidates.push(candidate.to_string());
        }
    }
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|candidate| candidate.split(['/', '\\']).count());
    let narrowest = candidates.last()?.clone();
    let narrowest_path = sandbox.resolve_existing(&narrowest).ok()?;
    candidates
        .into_iter()
        .all(|candidate| {
            sandbox
                .resolve_existing(&candidate)
                .is_ok_and(|path| narrowest_path.starts_with(path))
        })
        .then_some(narrowest)
}

/// Separate the user's request from attached text data so runtime context and
/// attachments can precede the request on the wire. This preserves the user's
/// request as the most recent, highest-authority content in the turn.
pub(super) fn prompt_parts(input: &PromptInput) -> PromptParts {
    let user_request: String = input
        .blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    let mut text_attachment_context = String::new();
    for att in &input.attachments {
        if att.is_text() {
            if let Ok(decoded) = decode_base64_text(&att.data_base64) {
                text_attachment_context.push_str(&format!(
                    "\n\n--- attached text file: {} (user-provided data) ---\n{decoded}\n",
                    att.filename
                ));
            }
        }
    }
    PromptParts {
        user_request,
        text_attachment_context: text_attachment_context.trim().to_string(),
    }
}

/// Flatten a prompt for surfaces such as in-flight steering that intentionally
/// do not receive the full per-turn context envelope.
pub(super) fn prompt_text(input: &PromptInput) -> String {
    let parts = prompt_parts(input);
    match parts.text_attachment_context.is_empty() {
        true => parts.user_request,
        false => format!(
            "{}\n\n{}",
            parts.user_request, parts.text_attachment_context
        ),
    }
}

/// Parse the built-in `/goal <objective>` command without treating lookalikes
/// such as `/goals` as commands. `Some("")` is intentional: callers can give
/// the user a focused missing-objective error instead of sending ambiguous
/// prose to the model.
pub(super) fn goal_command_objective(user_request: &str) -> Option<String> {
    let command = user_request.trim_start();
    let rest = command.strip_prefix("/goal")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim().to_string())
}

/// Make the goal schemas visible on the first model turn when the user names
/// that lifecycle explicitly. This does not create a goal or infer one from
/// ordinary work; it only removes an unnecessary deferred-discovery turn for
/// an already-authorized capability.
pub(super) fn explicitly_requests_goal_lifecycle(user_request: &str) -> bool {
    let normalized = user_request.to_ascii_lowercase();
    normalized.contains("create_goal")
        || normalized.contains("create a goal")
        || normalized.contains("start a goal")
        || normalized.contains("use a goal")
}

pub(super) fn goal_command_context() -> String {
    "[runtime command — derived from the user's explicit `/goal` prefix]\n\
The runtime has already selected the standing goal before this turn began, creating it only \
when needed. Do not call `create_goal` again. Begin or resume work toward the objective now, \
and use `update_goal` only when completion or a qualifying repeated blocker is proven."
        .into()
}

/// Render derived context before the actual request. Keeping the request last
/// matters because the model consumes the message autoregressively.
pub(super) fn assemble_turn_prompt(sections: &[String], user_request: &str) -> String {
    let context = sections
        .iter()
        .map(|section| section.trim())
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if context.is_empty() {
        return format!("# User request\n{user_request}");
    }
    format!("{context}\n\n# User request\n{user_request}")
}

/// Build the model-visible user content. Native images precede the assembled
/// text so the actual user request remains the final, most recent instruction.
pub(super) fn model_user_content(
    text: String,
    attachments: &[agent_core::domain::PendingUpload],
    native_image_support: bool,
) -> agent_loop::UserContent {
    if !native_image_support {
        return agent_loop::UserContent::Text(text);
    }
    let mut blocks = attachments
        .iter()
        .filter(|attachment| attachment.is_image())
        .map(|attachment| {
            agent_loop::UserBlock::Image(agent_loop::ImageContent {
                source: format!(
                    "data:{};base64,{}",
                    attachment.content_type, attachment.data_base64
                ),
                media_type: Some(attachment.content_type.clone()),
                alt: Some(attachment.filename.clone()),
            })
        })
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        return agent_loop::UserContent::Text(text);
    }
    blocks.push(agent_loop::UserBlock::Text(
        agent_loop::types::TextContent { text },
    ));
    agent_loop::UserContent::Blocks(blocks)
}

/// Minimal standard-base64 decoder (no external dep) for inlining text files.
pub(super) fn decode_base64_text(data: &str) -> std::result::Result<String, ()> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in data.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = val(c).ok_or(())? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_one_existing_user_named_directory_and_projects_it_into_context() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("developer-sweep-v9r2/src")).unwrap();
        std::fs::create_dir(root.path().join("sibling")).unwrap();
        let sandbox = Sandbox::new(root.path()).unwrap();
        let request = "Work as a developer in developer-sweep-v9r2. Extend src/parser.js and run tests from developer-sweep-v9r2.";

        let scope = explicit_task_scope(&sandbox, request).unwrap();
        assert_eq!(scope, "developer-sweep-v9r2");
        let scoped = sandbox.with_task_scope(&scope).unwrap();
        let context = environment_context(&scoped, false);
        assert!(context.contains("<task_scope>developer-sweep-v9r2</task_scope>"));
    }

    #[test]
    fn unrelated_named_directories_do_not_silently_narrow_the_project() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("app")).unwrap();
        std::fs::create_dir(root.path().join("crates")).unwrap();
        let sandbox = Sandbox::new(root.path()).unwrap();

        assert_eq!(
            explicit_task_scope(&sandbox, "Compare behavior in app with crates from crates."),
            None,
        );
    }
}
