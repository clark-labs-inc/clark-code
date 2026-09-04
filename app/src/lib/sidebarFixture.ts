import type { ConversationMeta } from "./history";

const SIDEBAR_FIXTURE_QUERY = "sidebar-fixture";

const fixtureProjects = [
  {
    path: "/workspace/northstar",
    titles: [
      "Review sidebar interaction feedback",
      "Map keyboard navigation gaps",
      "Polish the empty conversation state",
      "Triage the latest desktop regression",
      "Prepare the accessibility review",
      "Validate project switching",
      "Follow up on restore behavior",
      "Capture design QA notes",
    ],
  },
  {
    path: "/workspace/atlas",
    titles: [
      "Plan the onboarding walkthrough",
      "Audit message actions",
      "Refine search result grouping",
      "Compare archive flows",
      "Review command palette labels",
      "Document focus behavior",
      "Test a long conversation list",
      "Close the release checklist",
    ],
  },
  {
    path: "/workspace/harbor",
    titles: [
      "Investigate conversation ordering",
      "Prepare usability interview notes",
      "Resolve selection edge cases",
      "Check reduced-motion behavior",
      "Review the project menu",
      "Plan the next support handoff",
      "Verify restored chat context",
      "Summarize desktop feedback",
    ],
  },
] as const;

/** A realistic, local-only long-list state for repeatable browser QA. It is
 * intentionally gated by a dev query parameter so it cannot replace a real
 * user's cloud history in a shipped build. */
export function sidebarFixtureEnabled(search = typeof window === "undefined" ? "" : window.location.search): boolean {
  return import.meta.env.DEV && new URLSearchParams(search).has(SIDEBAR_FIXTURE_QUERY);
}

export function sidebarFixtureConversations(now = Date.now()): ConversationMeta[] {
  const quickChats: ConversationMeta[] = [
    {
      id: "00000000-0000-4000-8000-000000000001",
      title: "can you look at latest audio files",
      provider: "local",
      project: "/workspace/.agent/workspace/00000000-0000-4000-8000-000000000001",
      mode: "default",
      createdAt: now - 10 * 60 * 1000,
      updatedAt: now - 2 * 60 * 1000,
    },
    {
      id: "00000000-0000-4000-8000-000000000002",
      title: "hi",
      provider: "local",
      project: "/workspace/.agent/workspace/00000000-0000-4000-8000-000000000002",
      mode: "default",
      createdAt: now - 20 * 60 * 1000,
      updatedAt: now - 15 * 60 * 1000,
    },
  ];
  const active = fixtureProjects.flatMap((project, projectIndex) =>
    project.titles.map((title, titleIndex) => {
      const index = projectIndex * project.titles.length + titleIndex;
      return {
        id: `sidebar-fixture-${String(index + 1).padStart(2, "0")}`,
        title,
        provider: "local",
        project: project.path,
        mode: "default",
        createdAt: now - (index + 1) * 60 * 60 * 1000,
        updatedAt: now - index * 5 * 60 * 1000,
      };
    }),
  );
  const archived: ConversationMeta[] = [
    {
      id: "sidebar-fixture-archived-01",
      title: "Restore this design review",
      provider: "local",
      project: "/workspace/northstar",
      mode: "default",
      createdAt: now - 30 * 60 * 60 * 1000,
      updatedAt: now - 4 * 60 * 1000,
      archived: true,
    },
    {
      id: "sidebar-fixture-archived-02",
      title: "Older project handoff",
      provider: "local",
      project: "/workspace/atlas",
      mode: "default",
      createdAt: now - 60 * 60 * 60 * 1000,
      updatedAt: now - 8 * 60 * 1000,
      archived: true,
    },
  ];

  const specialist: ConversationMeta = {
    id: "sidebar-fixture-rsi-01",
    title: "Create a deterministic evaluation harness",
    provider: "specialist",
    project: "/workspace/northstar",
    createdAt: now - 25 * 60 * 1000,
    updatedAt: now - 2 * 60 * 1000,
    specialist: { kind: "rsi" },
  };

  return [...quickChats, ...active, specialist, ...archived];
}
