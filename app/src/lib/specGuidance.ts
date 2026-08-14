export interface SpecGuidanceQuestion {
  id: string;
  label: string;
  question: string;
  why: string;
  options: readonly string[];
  placeholder: string;
  heading: RegExp;
}

export interface SpecGuidanceReport {
  clear: number;
  total: number;
  missing: readonly SpecGuidanceQuestion[];
  current: SpecGuidanceQuestion;
  complete: boolean;
}

const QUESTIONS: readonly SpecGuidanceQuestion[] = [
  {
    id: "purpose",
    label: "Problem and outcome",
    question: "What should be meaningfully better when this is done?",
    why: "This keeps the team focused on the result, not just the requested feature.",
    options: ["The task takes less effort", "The experience feels clearer", "A new outcome becomes possible"],
    placeholder: "Or describe the change you want people to feel…",
    heading: /\b(summary|overview|problem|goal|outcome|purpose)\b/i,
  },
  {
    id: "audience",
    label: "People and roles",
    question: "Who should benefit first?",
    why: "A clear first audience helps the team make sensible tradeoffs without guessing.",
    options: ["Someone using the product", "Someone running or supporting it", "Both, but users come first"],
    placeholder: "Say it your way — a role, customer, or kind of person…",
    heading: /\b(user|users|audience|people|personas?|roles?|permissions?)\b/i,
  },
  {
    id: "journey",
    label: "End-to-end experience",
    question: "Which moment in the experience matters most?",
    why: "This anchors the spec in what a person actually sees, does, and understands.",
    options: ["Getting started", "Completing the main task", "Recovering when something goes wrong"],
    placeholder: "Describe the moment that has to feel right…",
    heading: /\b(experience|journey|scenario|workflow|flow|story|stories)\b/i,
  },
  {
    id: "behavior",
    label: "Expected behavior",
    question: "What must the product always do?",
    why: "Predictable rules turn the idea into behavior an agent or engineer can implement.",
    options: ["Guide one clear next step", "Show progress and status", "Keep the person in control"],
    placeholder: "Name a behavior that should never be ambiguous…",
    heading: /\b(requirements?|behavior|interaction|states?|display|rules?|data|lifecycle)\b/i,
  },
  {
    id: "edges",
    label: "Edge cases and recovery",
    question: "What should happen when something goes wrong?",
    why: "Recovery decisions prevent the team from inventing error behavior during implementation.",
    options: ["Explain the problem and let them retry", "Save progress and resume later", "Offer a safe fallback"],
    placeholder: "Describe an interruption, error, or unusual case…",
    heading: /\b(edge|error|failure|offline|interruption|empty|loading|recovery|duplicate|concurren)\b/i,
  },
  {
    id: "boundaries",
    label: "Boundaries and constraints",
    question: "What boundary must the team respect?",
    why: "Constraints and non-goals stop an implementation agent from expanding the scope silently.",
    options: ["Protect privacy and permissions", "Work with the current tools", "Keep the first release deliberately small"],
    placeholder: "Name a constraint, dependency, or thing this must not do…",
    heading: /\b(constraints?|dependencies|assumptions?|non-goals?|privacy|security|accessibility|performance|integration|rollout|migration)\b/i,
  },
  {
    id: "acceptance",
    label: "Acceptance criteria",
    question: "What would prove the feature works?",
    why: "Observable acceptance criteria give agents and engineering teams an unambiguous finish line.",
    options: ["A person completes the main task", "Every important state behaves correctly", "A repeatable test proves the outcome"],
    placeholder: "Describe what someone could observe or test…",
    heading: /\b(acceptance|criteria|definition of done|test scenarios?)\b/i,
  },
  {
    id: "success",
    label: "Success measures",
    question: "What result matters after launch?",
    why: "A measurable result lets the team evaluate whether shipping the feature solved the problem.",
    options: ["More people finish successfully", "The task takes less time or effort", "Errors or support requests decrease"],
    placeholder: "Describe the signal that would make this a success…",
    heading: /\b(success|metrics?|measures?|evaluation|signals?|observability)\b/i,
  },
] as const;

const PLACEHOLDER_LINE = /^(?:[-*]\s*)?(?:describe|capture|record|cover|turn|what\b|who\b|which\b|tbd\b|todo\b|not yet\b)/i;

function substantiveSection(markdown: string, question: SpecGuidanceQuestion): boolean {
  const lines = markdown.split("\n");
  for (let index = 0; index < lines.length; index += 1) {
    const match = /^(#{1,4})\s+(.+?)\s*$/.exec(lines[index] ?? "");
    if (!match || !question.heading.test(match[2] ?? "")) continue;
    const level = match[1]?.length ?? 4;
    const body: string[] = [];
    for (let cursor = index + 1; cursor < lines.length; cursor += 1) {
      const next = /^(#{1,4})\s+/.exec(lines[cursor] ?? "");
      if (next && next[1].length <= level) break;
      body.push(lines[cursor] ?? "");
    }
    const substance = body
      .map((line) => line.replace(/^\s*(?:[-*+] |\d+\.\s+|>\s*)/, "").trim())
      .filter((line) => line && !PLACEHOLDER_LINE.test(line) && !line.endsWith("?"))
      .join(" ")
      .replace(/[`*_#[\]()>|]/g, "")
      .trim();
    if (substance.length >= 60) return true;
  }
  return false;
}

export function specGuidance(markdown: string): SpecGuidanceReport {
  const missing = QUESTIONS.filter((question) => !substantiveSection(markdown, question));
  return {
    clear: QUESTIONS.length - missing.length,
    total: QUESTIONS.length,
    missing,
    current: missing[0] ?? QUESTIONS[QUESTIONS.length - 1],
    complete: missing.length === 0,
  };
}

export function guidedSpecPrompt(question: SpecGuidanceQuestion, answer: string): string {
  return `Continue the feature-specification workflow for the current SPEC.md.

The guided interview asked:
<guided_question>
${question.question}
</guided_question>

The user's answer, in their own words, is:
<guided_answer>
${answer.trim()}
</guided_answer>

Treat this as a product decision about “${question.label}”. Reconcile it into the correct section of the existing SPEC.md, preserving unrelated content and the semantic *_SPEC.md filename. Keep the language readable for a nontechnical product owner while making the behavior precise enough for coding agents and an engineering team to implement and test. Record any resulting tradeoff in the decision log. Do not implement the feature. After updating the document, briefly name what changed and ask only the single next question that would reduce the most uncertainty.`;
}
