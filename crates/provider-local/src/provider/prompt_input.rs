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
    format!(
        "[runtime context — derived from the active session, not user instruction]\n\
<environment_context>\n  <cwd>{}</cwd>\n  <workspace_roots>{}</workspace_roots>\n  <remote>{remote}</remote>\n</environment_context>",
        sandbox.root().display(),
        roots.join(" | ")
    )
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
) -> clark_agent::UserContent {
    if !native_image_support {
        return clark_agent::UserContent::Text(text);
    }
    let mut blocks = attachments
        .iter()
        .filter(|attachment| attachment.is_image())
        .map(|attachment| {
            clark_agent::UserBlock::Image(clark_agent::ImageContent {
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
        return clark_agent::UserContent::Text(text);
    }
    blocks.push(clark_agent::UserBlock::Text(
        clark_agent::types::TextContent { text },
    ));
    clark_agent::UserContent::Blocks(blocks)
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
