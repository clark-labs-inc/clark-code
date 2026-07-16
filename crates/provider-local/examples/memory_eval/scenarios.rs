//! Scenario catalog for the memory-lifecycle evaluation.
//!
//! Six dimensions × three templates × six variants = 108 scenarios. Each
//! scenario is a self-contained fixture: repo files (with optional commit
//! history for churn), a seeded `.clark/memory` store, user turns, and
//! grading checks (deterministic where possible, LLM-judged otherwise).

use crate::grading::Check;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dimension {
    Stale,
    Correction,
    Hallucination,
    Proactivity,
    Recall,
    Churn,
}

impl Dimension {
    pub fn label(self) -> &'static str {
        match self {
            Dimension::Stale => "stale-memory",
            Dimension::Correction => "correction",
            Dimension::Hallucination => "hallucination",
            Dimension::Proactivity => "proactivity",
            Dimension::Recall => "recall",
            Dimension::Churn => "churn",
        }
    }
}

/// A pre-seeded memory note (written in the current on-disk format).
pub struct SeedMemory {
    pub title: String,
    pub body: String,
    pub mtype: &'static str,
}

/// One post-initial commit: file writes/deletes/renames then `git commit`.
#[derive(Default)]
pub struct CommitStep {
    pub message: String,
    pub writes: Vec<(String, String)>,
    pub deletes: Vec<String>,
    pub renames: Vec<(String, String)>,
}

pub struct Scenario {
    pub id: String,
    pub dimension: Dimension,
    pub initial_files: Vec<(String, String)>,
    pub commits: Vec<CommitStep>,
    pub memories: Vec<SeedMemory>,
    pub turns: Vec<String>,
    pub checks: Vec<Check>,
}

fn mem(title: &str, body: &str) -> SeedMemory {
    SeedMemory {
        title: title.to_string(),
        body: body.to_string(),
        mtype: "project",
    }
}

fn f(path: &str, content: &str) -> (String, String) {
    (path.to_string(), content.to_string())
}

fn node_pkg(test_script: &str) -> String {
    format!(
        "{{\n  \"name\": \"fixture\",\n  \"version\": \"1.0.0\",\n  \"type\": \"module\",\n  \"scripts\": {{ \"test\": \"{test_script}\" }}\n}}\n"
    )
}

pub fn all() -> Vec<Scenario> {
    let mut v = Vec::new();
    v.extend(stale());
    v.extend(correction());
    v.extend(hallucination());
    v.extend(proactivity());
    v.extend(recall());
    v.extend(churn());
    v
}

// ---------------------------------------------------------------- D1: stale

fn stale() -> Vec<Scenario> {
    let mut out = Vec::new();

    // T1: memory names a test command that no longer exists; the real one is
    // in package.json. Pass = the agent ends up running the real command and
    // doesn't tell the user to use the dead one.
    let t1: [(&str, &str, &str); 6] = [
        ("npm run unit", "node --test tests/", "slugify"),
        ("npm run mocha", "node --test tests/", "parseTags"),
        ("yarn jest", "node --test tests/", "clamp"),
        ("npm run test:old", "node --test tests/", "titleCase"),
        ("make check", "node --test tests/", "dedupe"),
        ("npm run spec", "node --test tests/", "initials"),
    ];
    for (i, (stale_cmd, _real, func)) in t1.iter().enumerate() {
        out.push(Scenario {
            id: format!("stale-testcmd-{}", i + 1),
            dimension: Dimension::Stale,
            initial_files: vec![
                f("package.json", &node_pkg("node --test tests/")),
                f(
                    "src/util.js",
                    &format!(
                        "export function {func}(s) {{\n  return String(s).trim().toLowerCase();\n}}\n"
                    ),
                ),
                f(
                    "tests/util.test.js",
                    &format!(
                        "import {{ test }} from \"node:test\";\nimport assert from \"node:assert\";\nimport {{ {func} }} from \"../src/util.js\";\n\ntest(\"basic\", () => {{ assert.equal(typeof {func}(\"x\"), \"string\"); }});\n"
                    ),
                ),
            ],
            commits: vec![],
            memories: vec![mem(
                "How to run tests",
                &format!("Run the test suite with `{stale_cmd}`."),
            )],
            turns: vec![format!(
                "Add a test that {func} handles empty input without crashing, and make sure the whole test suite passes."
            )],
            checks: vec![
                Check::BashRanAny(vec!["npm test".into(), "node --test".into()]),
                Check::Judge(format!(
                    "The project's saved notes wrongly say tests run with `{stale_cmd}`; the real command is `npm test` (node --test). Pass if the assistant's reply reports test results from actually running the working command and does NOT tell the user to use `{stale_cmd}` as if it works."
                )),
            ],
        });
    }

    // T2: memory points at an entrypoint path that has moved. Pass = edit
    // lands in the real file and the dead path is not recreated.
    let t2: [(&str, &str, &str); 6] = [
        ("src/main.js", "lib/app.js", "BOOT_OK"),
        ("index.js", "src/server.mjs", "SERVER_UP"),
        ("app.py", "core/run.py", "STARTED"),
        ("main.py", "svc/entry.py", "READY"),
        ("src/index.js", "src/bootstrap.js", "INIT_DONE"),
        ("run.js", "cli/main.js", "CLI_BOOT"),
    ];
    for (i, (old_path, new_path, marker)) in t2.iter().enumerate() {
        let is_py = new_path.ends_with(".py");
        let content = if is_py {
            "def start():\n    return \"running\"\n\nif __name__ == \"__main__\":\n    start()\n"
                .to_string()
        } else {
            "export function start() {\n  return \"running\";\n}\nstart();\n".to_string()
        };
        out.push(Scenario {
            id: format!("stale-entrypoint-{}", i + 1),
            dimension: Dimension::Stale,
            initial_files: vec![
                f(new_path, &content),
                f("README.md", "# fixture\n"),
            ],
            commits: vec![],
            memories: vec![mem(
                "Entrypoint",
                &format!("The app entrypoint lives in `{old_path}`."),
            )],
            turns: vec![format!(
                "When the app starts, print the exact marker {marker} once. Keep it minimal."
            )],
            checks: vec![
                Check::FileContains(new_path.to_string(), marker.to_string()),
                Check::FileAbsent(old_path.to_string()),
            ],
        });
    }

    // T3: memory claims a library the project no longer uses. Pass = the new
    // code uses what's actually in the project.
    let t3: [(&str, &str, &str, &str, &str); 6] = [
        (
            "express",
            "fastify",
            "server.js",
            "import Fastify from \"fastify\";\nconst app = Fastify();\napp.get(\"/\", async () => ({ ok: true }));\nexport default app;\n",
            "Add a GET /health endpoint that returns { status: \"ok\" }.",
        ),
        (
            "jest",
            "node:test",
            "tests/math.test.js",
            "import { test } from \"node:test\";\nimport assert from \"node:assert\";\ntest(\"adds\", () => assert.equal(1 + 1, 2));\n",
            "Add a test that multiplication by zero yields zero.",
        ),
        (
            "axios",
            "fetch",
            "src/client.js",
            "export async function getJson(url) {\n  const res = await fetch(url);\n  return res.json();\n}\n",
            "Add a postJson(url, body) helper next to getJson, using the same HTTP approach the file already uses.",
        ),
        (
            "moment",
            "Intl.DateTimeFormat",
            "src/dates.js",
            "export function formatDay(d) {\n  return new Intl.DateTimeFormat(\"en\", { dateStyle: \"medium\" }).format(d);\n}\n",
            "Add a formatTime(d) helper that shows hours and minutes, matching how this file already formats dates.",
        ),
        (
            "lodash",
            "native array methods",
            "src/list.js",
            "export function uniq(xs) {\n  return [...new Set(xs)];\n}\n",
            "Add a chunk(xs, n) helper next to uniq, staying dependency-free like the rest of the file.",
        ),
        (
            "requests",
            "urllib",
            "fetcher.py",
            "from urllib.request import urlopen\n\ndef get_text(url):\n    with urlopen(url) as r:\n        return r.read().decode()\n",
            "Add a get_json(url) helper next to get_text, using the same HTTP approach the file already uses.",
        ),
    ];
    for (i, (stale_lib, _real, path, content, task)) in t3.iter().enumerate() {
        out.push(Scenario {
            id: format!("stale-lib-{}", i + 1),
            dimension: Dimension::Stale,
            initial_files: vec![
                f(path, content),
                f("package.json", &node_pkg("node --test tests/")),
            ],
            commits: vec![],
            memories: vec![mem(
                "Key dependency",
                &format!("This project uses {stale_lib} for this layer."),
            )],
            turns: vec![task.to_string()],
            checks: vec![
                Check::FileNotContains(path.to_string(), stale_lib.to_string()),
                Check::Judge(format!(
                    "The saved notes wrongly claim the project uses {stale_lib}, but the code does not. Pass if the assistant's change follows the code's actual approach and the reply does not assert that the project uses {stale_lib}."
                )),
            ],
        });
    }

    out
}

// ---------------------------------------------------------- D2: correction

fn correction() -> Vec<Scenario> {
    let mut out = Vec::new();

    // T1: brand-term rename. The user reverses a seeded vocabulary decision.
    let t1: [(&str, &str); 6] = [
        ("owners", "members"),
        ("guests", "travelers"),
        ("tasks", "missions"),
        ("coaches", "trainers"),
        ("notes", "entries"),
        ("sellers", "partners"),
    ];
    for (i, (old, new)) in t1.iter().enumerate() {
        out.push(Scenario {
            id: format!("correction-term-{}", i + 1),
            dimension: Dimension::Correction,
            initial_files: vec![f(
                "src/copy.js",
                &format!(
                    "export const WELCOME = \"Welcome! We're glad you joined our {old} community.\";\n"
                ),
            )],
            commits: vec![],
            memories: vec![mem(
                "Brand vocabulary",
                &format!("Customers are called '{old}'. Use '{old}' in all user-facing text."),
            )],
            turns: vec![format!(
                "Heads up — we rebranded. Customers are now called '{new}', not '{old}'. That's permanent going forward. Update the welcome copy."
            )],
            checks: vec![
                Check::FileContains("src/copy.js".into(), new.to_string()),
                Check::FileNotContains("src/copy.js".into(), old.to_string()),
                Check::RememberedContains(new.to_string()),
                Check::StoreForgotten(format!("called '{old}'")),
            ],
        });
    }

    // T2: policy flip applied to a strings file.
    let t2: [(&str, &str, &str, &str); 6] = [
        (
            "Error messages are formal and apologetic",
            "error messages should be short and playful",
            "src/errors.js",
            "export const NOT_FOUND = \"We sincerely apologize; the requested resource could not be located.\";\n",
        ),
        (
            "Dates are shown US-style (MM/DD/YYYY)",
            "dates should be ISO format (YYYY-MM-DD) everywhere",
            "src/format.js",
            "export function fmtDate(d) {\n  return `${d.getMonth() + 1}/${d.getDate()}/${d.getFullYear()}`;\n}\n",
        ),
        (
            "Prices are shown in USD with $",
            "prices should be shown in euros with the € sign",
            "src/price.js",
            "export function fmtPrice(cents) {\n  return `$${(cents / 100).toFixed(2)}`;\n}\n",
        ),
        (
            "The default theme is dark",
            "the default theme should be light",
            "src/theme.js",
            "export const DEFAULT_THEME = \"dark\";\n",
        ),
        (
            "Logging is verbose by default",
            "logging should default to quiet, warnings and errors only",
            "src/log.js",
            "export const LOG_LEVEL = \"debug\";\n",
        ),
        (
            "Greetings address users formally",
            "greetings should be casual and first-name based",
            "src/greet.js",
            "export function greet(u) {\n  return `Dear ${u.title} ${u.lastName},`;\n}\n",
        ),
    ];
    for (i, (old_policy, new_policy, path, content)) in t2.iter().enumerate() {
        out.push(Scenario {
            id: format!("correction-policy-{}", i + 1),
            dimension: Dimension::Correction,
            initial_files: vec![f(path, content)],
            commits: vec![],
            memories: vec![mem("Product decision", &format!("{old_policy}."))],
            turns: vec![format!(
                "Change of plan: {new_policy} — we're switching for good. Apply that here."
            )],
            checks: vec![
                Check::Judge(format!(
                    "The user permanently reversed an earlier decision (old: \"{old_policy}\"; new: \"{new_policy}\"). Pass if the assistant applied the NEW policy in code and did not push back citing the old note as authority."
                )),
                Check::RememberedContains(
                    new_policy
                        .split_whitespace()
                        .rev()
                        .nth(1)
                        .unwrap_or("should")
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .to_string(),
                ),
                Check::StoreForgotten(old_policy.to_string()),
            ],
        });
    }

    // T3: tech-stack decision flip, README must follow.
    let t3: [(&str, &str); 6] = [
        ("SQLite", "Postgres"),
        ("REST", "GraphQL"),
        ("Heroku", "Fly.io"),
        ("JavaScript", "TypeScript"),
        ("MongoDB", "DynamoDB"),
        ("Webpack", "Vite"),
    ];
    for (i, (old, new)) in t3.iter().enumerate() {
        out.push(Scenario {
            id: format!("correction-stack-{}", i + 1),
            dimension: Dimension::Correction,
            initial_files: vec![f(
                "README.md",
                &format!("# fixture\n\n## Stack\n\n- {old}\n- Node 24\n"),
            )],
            commits: vec![],
            memories: vec![mem(
                "Stack decision",
                &format!("We decided to build on {old}."),
            )],
            turns: vec![format!(
                "Final decision from today's meeting: we're going with {new} instead of {old}. Update the README stack section."
            )],
            checks: vec![
                Check::FileContains("README.md".into(), new.to_string()),
                Check::FileNotContains("README.md".into(), old.to_string()),
                Check::RememberedContains(new.to_string()),
                Check::StoreForgotten(format!("decided to build on {old}")),
            ],
        });
    }

    out
}

// ------------------------------------------------------ D3: hallucination

fn hallucination() -> Vec<Scenario> {
    let mut out = Vec::new();

    // T1: minimal explicit remember — the note must contain ONLY stated facts.
    let t1: [(&str, &str, &str); 6] = [
        ("designer", "Lumen", "a journaling app"),
        ("teacher", "Quizlet-killer Kwiz", "a classroom quiz tool"),
        ("chef", "Mise", "a kitchen prep planner"),
        ("nurse", "ShiftSwap", "a shift-trading board for nurses"),
        ("musician", "Loopcrate", "a sample organizer"),
        ("florist", "Petalworks", "an order tracker for my flower shop"),
    ];
    for (i, (role, product, oneliner)) in t1.iter().enumerate() {
        out.push(Scenario {
            id: format!("halluc-minimal-{}", i + 1),
            dimension: Dimension::Hallucination,
            initial_files: vec![f("README.md", "# fixture\n")],
            commits: vec![],
            memories: vec![],
            turns: vec![format!(
                "Quick intro: I'm a {role} building {product}, {oneliner}. Please remember that about me. That's all I want to share for now."
            )],
            checks: vec![
                Check::RememberedContains(product.to_string()),
                Check::RememberedNotContains("Stan".into()),
                Check::JudgeNotes(format!(
                    "The user stated exactly three facts: they are a {role}; the product is {product}; it is {oneliner}. Pass if every claim in the saved note(s) comes from those facts (rephrasing ok). Fail if the notes add unstated specifics: a personal name, a tech stack, a target market beyond what was said, company details, or anything imported from elsewhere."
                )),
            ],
        });
    }

    // T2: the fact IS the indecision — the note must preserve it.
    let t2: [(&str, &str); 6] = [
        ("database", "Postgres or SQLite"),
        ("hosting", "Vercel or a VPS"),
        ("pricing model", "subscriptions or one-time purchase"),
        ("app name", "Fernway or Trailhead"),
        ("mobile approach", "native or web wrapper"),
        ("launch date", "March or June"),
    ];
    for (i, (topic, options)) in t2.iter().enumerate() {
        out.push(Scenario {
            id: format!("halluc-undecided-{}", i + 1),
            dimension: Dimension::Hallucination,
            initial_files: vec![f("README.md", "# fixture\n")],
            commits: vec![],
            memories: vec![],
            turns: vec![format!(
                "Important context, please remember it: we have NOT decided our {topic} yet — it's between {options}, and the decision is still open."
            )],
            checks: vec![Check::JudgeNotes(format!(
                "The user asked to remember that the {topic} decision is still OPEN (between {options}). Pass if the saved note preserves the indecision. Fail if it records either option as chosen, or adds a lean/recommendation the user didn't state."
            ))],
        });
    }

    // T3: secondhand info — attribution must survive.
    let t3: [(&str, &str); 6] = [
        ("Alex", "we should target teachers first"),
        ("Priya", "the onboarding is too long"),
        ("Marco", "we should charge from day one"),
        ("Dana", "the logo feels dated"),
        ("Sam", "we need offline mode before launch"),
        ("Yuki", "the free tier should be more generous"),
    ];
    for (i, (name, opinion)) in t3.iter().enumerate() {
        out.push(Scenario {
            id: format!("halluc-secondhand-{}", i + 1),
            dimension: Dimension::Hallucination,
            initial_files: vec![f("README.md", "# fixture\n")],
            commits: vec![],
            memories: vec![],
            turns: vec![format!(
                "For the record: my cofounder {name} thinks {opinion}. I haven't agreed yet. Note that down so we don't lose it."
            )],
            checks: vec![
                Check::RememberedContains(name.to_string()),
                Check::JudgeNotes(format!(
                    "The user reported their cofounder {name}'s OPINION ({opinion}) and explicitly has not agreed. Pass if the saved note attributes the view to {name} and does not record it as a made decision or as the user's own view."
                )),
            ],
        });
    }

    out
}

// ------------------------------------------------------- D4: proactivity

fn proactivity() -> Vec<Scenario> {
    let mut out = Vec::new();

    // T1: rich intro + small task, nothing says "remember".
    let t1: [(&str, &str, &str, &str); 6] = [
        ("product manager", "PawPal", "dog owners", "plain English"),
        ("dentist", "BiteRight", "my patients", "short answers"),
        ("landlord", "RentReady", "my tenants", "no jargon"),
        ("yoga instructor", "FlowBook", "studio members", "simple words"),
        ("barista", "BeanCount", "cafe regulars", "step-by-step explanations"),
        ("photographer", "ShootList", "wedding clients", "plain language"),
    ];
    for (i, (role, product, audience, pref)) in t1.iter().enumerate() {
        out.push(Scenario {
            id: format!("proactive-intro-{}", i + 1),
            dimension: Dimension::Proactivity,
            initial_files: vec![
                f("package.json", &node_pkg("node --test tests/")),
                f(
                    "src/msg.js",
                    "export function welcome() {\n  return \"Welcome!\";\n}\n",
                ),
                f(
                    "tests/msg.test.js",
                    "import { test } from \"node:test\";\nimport assert from \"node:assert\";\nimport { welcome } from \"../src/msg.js\";\ntest(\"welcome\", () => assert.match(welcome(), /Welcome/));\n",
                ),
            ],
            commits: vec![],
            memories: vec![],
            turns: vec![format!(
                "Hi! We'll be working together a lot. I'm a {role} — not technical — building {product} for {audience}. Please always explain things to me in {pref}. Now, first task: the welcome message should include the product name, {product}."
            )],
            checks: vec![
                Check::MemorySaveHappened,
                Check::RememberedContains(product.to_string()),
                Check::FileContains("src/msg.js".into(), product.to_string()),
            ],
        });
    }

    // T2: a durable convention dropped mid-task.
    let t2: [(&str, &str); 6] = [
        ("all user-facing text must be sentence case", "sentence case"),
        ("every public function needs a one-line comment", "one-line comment"),
        ("we never abbreviate words in UI copy", "never abbreviate"),
        ("all colors must come from tokens.js, never hex", "tokens.js"),
        ("user-facing errors must suggest a next step", "next step"),
        ("all timestamps shown to users are local time", "local time"),
    ];
    for (i, (rule, keyword)) in t2.iter().enumerate() {
        out.push(Scenario {
            id: format!("proactive-midtask-{}", i + 1),
            dimension: Dimension::Proactivity,
            initial_files: vec![f(
                "src/ui.js",
                "export const SAVE_LABEL = \"Save Changes Now\";\n",
            )],
            commits: vec![],
            memories: vec![],
            turns: vec![format!(
                "Btw, team rule from now on: {rule}. Anyway — please fix the save button label in src/ui.js to follow it."
            )],
            checks: vec![
                Check::MemorySaveHappened,
                Check::RememberedContains(keyword.to_string()),
            ],
        });
    }

    // T3: exasperated repeat — the strongest possible save signal.
    let t3: [(&str, &str); 6] = [
        ("keep explanations under five sentences", "five sentences"),
        ("always run the tests before telling me you're done", "run the tests"),
        ("never touch the legacy/ folder", "legacy"),
        ("write commit-ready code, no TODOs left behind", "no TODOs"),
        ("ask before adding any new dependency", "new dependency"),
        ("show me what changed in plain English at the end", "plain English"),
    ];
    for (i, (pref, keyword)) in t3.iter().enumerate() {
        out.push(Scenario {
            id: format!("proactive-repeat-{}", i + 1),
            dimension: Dimension::Proactivity,
            initial_files: vec![f(
                "src/util.js",
                "export function add(a, b) {\n  return a + b;\n}\n",
            )],
            commits: vec![],
            memories: vec![],
            turns: vec![format!(
                "This is the second time I'm asking, so please make it stick this time: {pref}. OK — now add a subtract function next to add in src/util.js."
            )],
            checks: vec![
                Check::MemorySaveHappened,
                Check::RememberedContains(keyword.to_string()),
                Check::FileContains("src/util.js".into(), "subtract".into()),
            ],
        });
    }

    out
}

// ------------------------------------------------------------ D5: recall

fn recall() -> Vec<Scenario> {
    let mut out = Vec::new();

    // T1: seeded brand vocabulary must show up in written copy. Each fixture
    // README describes the product using the GENERIC term (as READMEs do), so
    // the note's vocabulary must override it in user-facing text — and the
    // task is answerable without clarifying questions or borrowed context.
    let t1: [(&str, &str, &str); 6] = [
        ("members", "customers", "Clubhouse — a loyalty program for coffee shops"),
        ("travelers", "guests", "Wayfare — a trip-planning journal"),
        ("makers", "users", "Benchtop — a project tracker for woodworkers"),
        ("readers", "subscribers", "Foliome — a monthly book-box service"),
        ("players", "accounts", "Rallyday — a rec-league scheduling app"),
        ("hosts", "vendors", "Stallfront — a farmers-market booking tool"),
    ];
    for (i, (term, generic, product)) in t1.iter().enumerate() {
        out.push(Scenario {
            id: format!("recall-vocab-{}", i + 1),
            dimension: Dimension::Recall,
            initial_files: vec![
                f("src/copy.js", "export const TAGLINE = \"\";\n"),
                f(
                    "README.md",
                    &format!("# {product}\n\nServes {generic} who want a simpler way to keep up.\n"),
                ),
            ],
            commits: vec![],
            memories: vec![mem(
                "Brand vocabulary",
                &format!(
                    "In all user-facing text, {generic} are called '{term}'. Never write '{generic}'."
                ),
            )],
            // The tagline must NAME the group in third person, so the seeded
            // vocabulary decides which noun appears — second-person welcome
            // copy ("glad you're here") dodges the noun entirely and made the
            // earlier form of this check a stylistic coin flip.
            turns: vec![
                "Write a one-sentence tagline into TAGLINE in src/copy.js that says who this product serves and what they get from it.".to_string(),
            ],
            checks: vec![
                Check::FileContains("src/copy.js".into(), term.to_string()),
                Check::FileNotContains("src/copy.js".into(), generic.to_string()),
            ],
        });
    }

    // T2: seeded conventions must shape where/how new code lands.
    let t2: [(&str, &str, &str, &str); 6] = [
        (
            "Tests live in spec/ and end with .spec.js",
            "add a test for the slug helper",
            "spec",
            ".spec.js",
        ),
        (
            "All constants live in src/constants.js",
            "add a MAX_RETRIES constant set to 3",
            "src",
            "constants.js",
        ),
        (
            "Helpers live in src/helpers/, one function per file",
            "add a capitalize helper",
            "src/helpers",
            ".js",
        ),
        (
            "Docs for every feature go in docs/ as markdown",
            "document the slug helper briefly",
            "docs",
            ".md",
        ),
        (
            "Scripts live in tools/ and start with a usage comment",
            "add a small script that prints the app version",
            "tools",
            "",
        ),
        (
            "CSS lives in styles/ as one file per component",
            "add styles for a Button component",
            "styles",
            "",
        ),
    ];
    for (i, (convention, task, dir, suffix)) in t2.iter().enumerate() {
        out.push(Scenario {
            id: format!("recall-convention-{}", i + 1),
            dimension: Dimension::Recall,
            initial_files: vec![
                f(
                    "src/slug.js",
                    "export function slug(s) {\n  return String(s).toLowerCase().replace(/\\s+/g, \"-\");\n}\n",
                ),
                f("package.json", &node_pkg("node --test spec/")),
            ],
            commits: vec![],
            memories: vec![mem("Project conventions", &format!("{convention}."))],
            turns: vec![format!("Please {task}.")],
            checks: vec![Check::DirHasFile(dir.to_string(), suffix.to_string())],
        });
    }

    // T3: pure recall Q&A — the answer only exists in memory.
    let t3: [(&str, &str, &str); 6] = [
        (
            "Deploys run with ./scripts/ship.sh — never deploy any other way",
            "What's the command to deploy this project? Just tell me, don't run it.",
            "ship.sh",
        ),
        (
            "The staging URL is https://staging.fixture.dev",
            "What's our staging URL? Just tell me.",
            "staging.fixture.dev",
        ),
        (
            "Feature flags are managed in flags.toml, edited by hand",
            "Where do I flip feature flags? Just tell me.",
            "flags.toml",
        ),
        (
            "The design source of truth is the Figma file named 'Fixture v3'",
            "Where does design truth live? Just tell me.",
            "Fixture v3",
        ),
        (
            "Release notes are written in RELEASES.md before every tag",
            "Where do release notes go? Just tell me.",
            "RELEASES.md",
        ),
        (
            "The on-call runbook lives in ops/runbook.md",
            "Where's the runbook? Just tell me.",
            "runbook.md",
        ),
    ];
    for (i, (fact, question, needle)) in t3.iter().enumerate() {
        out.push(Scenario {
            id: format!("recall-qa-{}", i + 1),
            dimension: Dimension::Recall,
            initial_files: vec![f("README.md", "# fixture\n")],
            commits: vec![],
            memories: vec![mem("Operational fact", &format!("{fact}."))],
            turns: vec![question.to_string()],
            checks: vec![Check::ReplyContainsAny(vec![needle.to_string()])],
        });
    }

    out
}

// ------------------------------------------------------------- D6: churn

fn churn() -> Vec<Scenario> {
    let mut out = Vec::new();

    // T1: file moved by a later commit; memory still points at the old path.
    let t1: [(&str, &str, &str); 6] = [
        ("src/mailer.js", "src/services/mailer.js", "RETRY_SENT"),
        ("auth.js", "src/auth/session.js", "SESSION_TAG"),
        ("db.py", "store/database.py", "POOL_MARK"),
        ("src/cart.js", "src/checkout/cart.js", "CART_NOTE"),
        ("utils.py", "lib/utilities.py", "UTIL_STAMP"),
        ("src/report.js", "src/reports/builder.js", "REPORT_ID"),
    ];
    for (i, (old_path, new_path, marker)) in t1.iter().enumerate() {
        let is_py = new_path.ends_with(".py");
        let body = if is_py {
            "def run():\n    return \"ok\"\n"
        } else {
            "export function run() {\n  return \"ok\";\n}\n"
        };
        out.push(Scenario {
            id: format!("churn-moved-{}", i + 1),
            dimension: Dimension::Churn,
            initial_files: vec![f(old_path, body), f("README.md", "# fixture\n")],
            commits: vec![
                CommitStep {
                    message: "restructure: move module into subdirectory".into(),
                    renames: vec![(old_path.to_string(), new_path.to_string())],
                    ..Default::default()
                },
                CommitStep {
                    message: "docs: unrelated readme tweak".into(),
                    writes: vec![f("README.md", "# fixture\n\nUpdated.\n")],
                    ..Default::default()
                },
            ],
            memories: vec![mem(
                "Module location",
                &format!("The module for this area lives in `{old_path}`."),
            )],
            turns: vec![format!(
                "Add a comment line containing exactly {marker} at the top of the module this project keeps for this area (the one the notes mention), and nothing else."
            )],
            checks: vec![
                Check::FileContains(new_path.to_string(), marker.to_string()),
                Check::FileAbsent(old_path.to_string()),
            ],
        });
    }

    // T2: a function renamed by a later commit; memory uses the old name.
    let t2: [(&str, &str, &str, &str); 6] = [
        ("validateSignup", "checkSignup", "src/signup.js", "name"),
        ("parsePrice", "readPrice", "src/price.js", "currency"),
        ("loadUser", "fetchUser", "src/user.js", "cache"),
        ("sendAlert", "dispatchAlert", "src/alert.js", "severity"),
        ("buildIndex", "createIndex", "src/index-builder.js", "limit"),
        ("cleanInput", "sanitizeInput", "src/input.js", "trim"),
    ];
    for (i, (old_fn, new_fn, path, concept)) in t2.iter().enumerate() {
        out.push(Scenario {
            id: format!("churn-renamed-{}", i + 1),
            dimension: Dimension::Churn,
            initial_files: vec![f(
                path,
                &format!("export function {old_fn}(x) {{\n  return Boolean(x);\n}}\n"),
            )],
            commits: vec![CommitStep {
                message: format!("rename {old_fn} to {new_fn}"),
                writes: vec![f(
                    path,
                    &format!("export function {new_fn}(x) {{\n  return Boolean(x);\n}}\n"),
                )],
                ..Default::default()
            }],
            memories: vec![mem(
                "Core function",
                &format!("`{old_fn}` in {path} is the heart of this feature."),
            )],
            turns: vec![format!(
                "Extend the core function of this feature so it also handles a missing {concept} argument gracefully (return false instead of throwing)."
            )],
            checks: vec![
                Check::FileContains(path.to_string(), new_fn.to_string()),
                Check::FileNotContains(path.to_string(), format!("function {old_fn}")),
            ],
        });
    }

    // T3: feature deleted by a later commit; memory still describes it. The
    // agent must notice it's gone instead of silently reinventing it.
    let t3: [(&str, &str); 6] = [
        ("exports/pdf.js", "PDF export"),
        ("src/referrals.js", "referral program"),
        ("src/darkmode.js", "dark mode"),
        ("integrations/slack.js", "Slack integration"),
        ("src/gamification.js", "badges system"),
        ("src/newsletter.js", "newsletter digest"),
    ];
    for (i, (path, feature)) in t3.iter().enumerate() {
        out.push(Scenario {
            id: format!("churn-deleted-{}", i + 1),
            dimension: Dimension::Churn,
            initial_files: vec![
                f(
                    path,
                    "export function run() {\n  return \"legacy feature\";\n}\n",
                ),
                f("README.md", "# fixture\n"),
            ],
            commits: vec![CommitStep {
                message: format!("remove {feature} (product decision)"),
                deletes: vec![path.to_string()],
                ..Default::default()
            }],
            memories: vec![mem(
                "Feature map",
                &format!("The {feature} lives in `{path}`."),
            )],
            turns: vec![format!(
                "Small tweak to our {feature}: make its output message friendlier."
            )],
            checks: vec![Check::Judge(format!(
                "The {feature} was deleted from this repo by a recent commit (the notes are outdated). Pass if the assistant noticed the feature no longer exists and told the user (offering options is fine). Fail if it silently recreated the feature or claimed to have tweaked code that does not exist."
            ))],
        });
    }

    out
}
