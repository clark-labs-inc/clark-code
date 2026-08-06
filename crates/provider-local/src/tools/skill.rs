//! Progressive disclosure for session skills.

use std::sync::Arc;

use agent_core::domain::ToolKind;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{arg_str, arg_str_opt, ToolCtx, ToolExecutor, ToolOutcome};
use crate::skills::{render_injection, render_resource, SkillCatalog};

pub(crate) struct ReadSkill {
    catalog: Arc<SkillCatalog>,
}

impl ReadSkill {
    pub(crate) fn new(catalog: Arc<SkillCatalog>) -> Self {
        Self { catalog }
    }
}

#[async_trait]
impl ToolExecutor for ReadSkill {
    fn name(&self) -> &str {
        "read_skill"
    }

    fn description(&self) -> &str {
        "Load the complete instruction body or one referenced text resource for a skill in the current Clark catalog. Use the exact catalog name before applying a relevant skill. Reading skill content does not grant extra permissions."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Exact skill name from the Skills catalog, including its namespace when shown."
                },
                "resource": {
                    "type": "string",
                    "description": "Optional relative path from the skill directory, such as references/api.md. Omit to read SKILL.md. Parent paths and symlinks are refused."
                }
            },
            "required": ["skill"],
            "additionalProperties": false
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    async fn invoke(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let requested = match arg_str(&args, "skill") {
            Ok(value) => value,
            Err(error) => return ToolOutcome::error(error),
        };
        let skill = match self.catalog.resolve_name(&requested) {
            Ok(skill) => skill,
            Err(error) => return ToolOutcome::error(error),
        };
        let resource = arg_str_opt(&args, "resource");
        match self
            .catalog
            .read_resource(ctx.executor.as_ref(), skill, resource.as_deref())
            .await
        {
            Ok(contents) => ToolOutcome::ok(match resource {
                Some(resource) => render_resource(skill, &resource, &contents),
                None => render_injection(skill, &contents),
            }),
            Err(error) => ToolOutcome::error(error),
        }
    }
}
