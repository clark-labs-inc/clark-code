use agent_core::ContentBlock;
use clark_agent as ca;

pub(super) fn user_content(blocks: &[ContentBlock]) -> Option<ca::UserContent> {
    let mut rich = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text }
            | ContentBlock::Resource {
                text: Some(text), ..
            } => rich.push(ca::UserBlock::Text(ca::TextContent { text: text.clone() })),
            ContentBlock::Image {
                mime_type,
                data,
                uri,
            } => rich.push(ca::UserBlock::Image(ca::ImageContent {
                source: uri
                    .clone()
                    .unwrap_or_else(|| format!("data:{mime_type};base64,{data}")),
                media_type: Some(mime_type.clone()),
                alt: None,
            })),
            ContentBlock::ResourceLink { uri, name } => {
                rich.push(ca::UserBlock::Text(ca::TextContent {
                    text: name.as_deref().unwrap_or(uri).to_string(),
                }))
            }
            ContentBlock::SkillReference { id, revision, name } => {
                rich.push(ca::UserBlock::Text(ca::TextContent {
                    text: format!("[Selected Clark skill: {name} ({id}@{revision})]"),
                }))
            }
            ContentBlock::Audio { .. }
            | ContentBlock::Thinking { .. }
            | ContentBlock::Resource { text: None, .. } => {}
        }
    }
    match rich.as_slice() {
        [] => None,
        [ca::UserBlock::Text(text)] => Some(ca::UserContent::Text(text.text.clone())),
        _ => Some(ca::UserContent::Blocks(rich)),
    }
}

pub(super) fn assistant_content(blocks: &[ContentBlock]) -> ca::AssistantContent {
    let mut content = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text }
            | ContentBlock::Resource {
                text: Some(text), ..
            } => content.push(ca::AssistantBlock::Text(ca::TextContent {
                text: text.clone(),
            })),
            ContentBlock::Thinking { text } => {
                content.push(ca::AssistantBlock::Reasoning(ca::TextContent {
                    text: text.clone(),
                }))
            }
            ContentBlock::ResourceLink { uri, name } => {
                content.push(ca::AssistantBlock::Text(ca::TextContent {
                    text: name.as_deref().unwrap_or(uri).to_string(),
                }))
            }
            ContentBlock::Image { .. }
            | ContentBlock::Audio { .. }
            | ContentBlock::Resource { text: None, .. }
            | ContentBlock::SkillReference { .. } => {}
        }
    }
    ca::AssistantContent { blocks: content }
}

pub(super) fn assistant_blocks(content: &ca::AssistantContent) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();
    for block in &content.blocks {
        match block {
            ca::AssistantBlock::Text(text) => blocks.push(ContentBlock::text(text.text.clone())),
            ca::AssistantBlock::Thinking(text) | ca::AssistantBlock::Reasoning(text) => {
                blocks.push(ContentBlock::thinking(text.text.clone()))
            }
            ca::AssistantBlock::ReasoningDetails(details) => {
                for item in details.as_items() {
                    let readable = match item {
                        ca::ReasoningItem::Text { text, .. } => Some(text),
                        ca::ReasoningItem::Summary { summary, .. } => Some(summary),
                        ca::ReasoningItem::Encrypted { .. } => None,
                    };
                    if let Some(readable) = readable {
                        blocks.push(ContentBlock::thinking(readable));
                    }
                }
            }
            ca::AssistantBlock::ToolCall(_) => {}
        }
    }
    blocks
}

pub(super) fn user_block(block: &ca::UserBlock) -> ContentBlock {
    match block {
        ca::UserBlock::Text(text) => ContentBlock::text(text.text.clone()),
        ca::UserBlock::Image(image) => image_block(image),
    }
}

pub(super) fn tool_result_block(block: &ca::ToolResultBlock) -> ContentBlock {
    match block {
        ca::ToolResultBlock::Text(text) => ContentBlock::text(text.text.clone()),
        ca::ToolResultBlock::Image(image) => image_block(image),
    }
}

pub(super) fn tool_result_content(blocks: &[ContentBlock]) -> Option<ca::ToolResultContent> {
    let mut content = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text }
            | ContentBlock::Resource {
                text: Some(text), ..
            } => content.push(ca::ToolResultBlock::Text(ca::TextContent {
                text: text.clone(),
            })),
            ContentBlock::Image {
                mime_type,
                data,
                uri,
            } => content.push(ca::ToolResultBlock::Image(ca::ImageContent {
                source: uri
                    .clone()
                    .unwrap_or_else(|| format!("data:{mime_type};base64,{data}")),
                media_type: Some(mime_type.clone()),
                alt: None,
            })),
            ContentBlock::ResourceLink { uri, name } => {
                content.push(ca::ToolResultBlock::Text(ca::TextContent {
                    text: name.as_deref().unwrap_or(uri).to_string(),
                }))
            }
            ContentBlock::Audio { .. }
            | ContentBlock::Thinking { .. }
            | ContentBlock::Resource { text: None, .. }
            | ContentBlock::SkillReference { .. } => {}
        }
    }
    (!content.is_empty()).then_some(ca::ToolResultContent { blocks: content })
}

fn image_block(image: &ca::ImageContent) -> ContentBlock {
    let mime_type = image
        .media_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());
    let data_prefix = format!("data:{mime_type};base64,");
    if let Some(data) = image.source.strip_prefix(&data_prefix) {
        ContentBlock::Image {
            mime_type,
            data: data.to_string(),
            uri: None,
        }
    } else {
        ContentBlock::Image {
            mime_type,
            data: String::new(),
            uri: Some(image.source.clone()),
        }
    }
}
