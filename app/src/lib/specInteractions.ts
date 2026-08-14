export interface SpecInteractionAction {
  id: "recommend" | "decide" | "question" | "revise";
  label: string;
  prompt: string;
}

function looksLikeOpenQuestion(text: string): boolean {
  return /(?:^|\s)OQ\d+\b/i.test(text) || text.includes("?");
}

/** Suggested scoped prompts for the selected part of a living specification.
 * Questions get decision-oriented actions; ordinary prose gets review/edit
 * actions. The user can always ignore these and write a custom prompt. */
export function specInteractionActions(selection: string): SpecInteractionAction[] {
  if (looksLikeOpenQuestion(selection)) {
    return [
      {
        id: "recommend",
        label: "Recommend",
        prompt: "Recommend the strongest answer to this open question and briefly explain the tradeoff.",
      },
      {
        id: "decide",
        label: "Record decision",
        prompt: "Record this decision in the spec: ",
      },
      {
        id: "question",
        label: "Clarify",
        prompt: "What do we still need to learn before deciding this?",
      },
    ];
  }

  return [
    {
      id: "question",
      label: "Ask about this",
      prompt: "Explain the reasoning behind this part of the spec.",
    },
    {
      id: "revise",
      label: "Suggest edit",
      prompt: "Make this clearer and more specific without changing the intended behavior.",
    },
  ];
}
