use super::loader::{parse_bundled_skill, BundledSkillSpec};
use super::Skill;

const GITHUB: &str = include_str!("../../skills/github/github/SKILL.md");
const ADDRESS_COMMENTS: &str = include_str!("../../skills/github/gh-address-comments/SKILL.md");
const FIX_CI: &str = include_str!("../../skills/github/gh-fix-ci/SKILL.md");
const YEET: &str = include_str!("../../skills/github/yeet/SKILL.md");
const SENTRY: &str = include_str!("../../skills/sentry/SKILL.md");
const SCOUT: &str = include_str!("../../skills/scout/SKILL.md");
const SECURITY_SCAN: &str = include_str!("../../skills/security/security-scan/SKILL.md");
const SECURITY_DIFF: &str = include_str!("../../skills/security/security-diff/SKILL.md");
const SECURITY_DEEP: &str = include_str!("../../skills/security/security-deep/SKILL.md");

const BASH: &[&str] = &["bash"];
const SCOUT_TOOLS: &[&str] = &[
    "scout_capabilities",
    "scout_adapter",
    "scout_ledger",
    "scout_enterprise",
    "scout_enterprise_query",
    "scout_probe",
    "scout_measure",
];
const SECURITY_TOOLS: &[&str] = &[
    "security_scan_contract",
    "security_poc_execute",
    "read_file",
    "grep",
    "glob",
    "bash",
];
const SECURITY_DEEP_TOOLS: &[&str] = &[
    "security_scan_contract",
    "security_poc_execute",
    "delegate_read_only",
    "resolve_delegation",
    "read_file",
    "grep",
    "glob",
    "bash",
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
        {
            let mut scout = bundled("scout", "clark://skills/scout", SCOUT, SCOUT_TOOLS);
            scout.allow_implicit_invocation = false;
            scout
        },
        {
            let mut security = bundled(
                "security",
                "clark://skills/security/security-scan",
                SECURITY_SCAN,
                SECURITY_TOOLS,
            );
            security.allow_implicit_invocation = false;
            security
        },
        {
            let mut security = bundled(
                "security",
                "clark://skills/security/security-diff",
                SECURITY_DIFF,
                SECURITY_TOOLS,
            );
            security.allow_implicit_invocation = false;
            security
        },
        {
            let mut security = bundled(
                "security",
                "clark://skills/security/security-deep",
                SECURITY_DEEP,
                SECURITY_DEEP_TOOLS,
            );
            security.allow_implicit_invocation = false;
            security
        },
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
