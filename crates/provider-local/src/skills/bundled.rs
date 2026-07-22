use super::loader::parse_bundled_skill;
use super::Skill;

const GITHUB: &str = include_str!("../../skills/github/github/SKILL.md");
const ADDRESS_COMMENTS: &str = include_str!("../../skills/github/gh-address-comments/SKILL.md");
const FIX_CI: &str = include_str!("../../skills/github/gh-fix-ci/SKILL.md");
const YEET: &str = include_str!("../../skills/github/yeet/SKILL.md");
const SENTRY: &str = include_str!("../../skills/sentry/SKILL.md");

pub(super) fn skills() -> Vec<Skill> {
    vec![
        parse_bundled_skill("github", "clark://skills/github", GITHUB),
        parse_bundled_skill(
            "github",
            "clark://skills/github/gh-address-comments",
            ADDRESS_COMMENTS,
        ),
        parse_bundled_skill("github", "clark://skills/github/gh-fix-ci", FIX_CI),
        parse_bundled_skill("github", "clark://skills/github/yeet", YEET),
        parse_bundled_skill("sentry", "clark://skills/sentry", SENTRY),
    ]
}
