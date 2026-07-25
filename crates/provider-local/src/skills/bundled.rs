use super::loader::{parse_bundled_skill, BundledSkillSpec};
use super::Skill;

const GITHUB: &str = include_str!("../../skills/github/github/SKILL.md");
const ADDRESS_COMMENTS: &str = include_str!("../../skills/github/gh-address-comments/SKILL.md");
const FIX_CI: &str = include_str!("../../skills/github/gh-fix-ci/SKILL.md");
const YEET: &str = include_str!("../../skills/github/yeet/SKILL.md");
const SENTRY: &str = include_str!("../../skills/sentry/SKILL.md");
const SCOUT: &str = include_str!("../../skills/scout/SKILL.md");

const BASH: &[&str] = &["bash"];
const SCOUT_TOOLS: &[&str] = &[
    "scout_capabilities",
    "scout_ledger",
    "scout_probe",
    "scout_measure",
    "delegate_read_only",
    "resolve_delegation",
];

pub(super) fn skills() -> Vec<Skill> {
    vec![
        bundled("github", "clark://skills/github", GITHUB, BASH),
        bundled(
            "github",
            "clark://skills/github/gh-address-comments",
            ADDRESS_COMMENTS,
            BASH,
        ),
        bundled("github", "clark://skills/github/gh-fix-ci", FIX_CI, BASH),
        bundled("github", "clark://skills/github/yeet", YEET, BASH),
        bundled("sentry", "clark://skills/sentry", SENTRY, BASH),
        bundled("scout", "clark://skills/scout", SCOUT, SCOUT_TOOLS),
    ]
}

fn bundled(
    namespace: &'static str,
    locator: &'static str,
    contents: &'static str,
    required_tools: &'static [&'static str],
) -> Skill {
    parse_bundled_skill(BundledSkillSpec {
        namespace,
        locator,
        contents,
        required_tools,
        allow_implicit_invocation: true,
    })
}
