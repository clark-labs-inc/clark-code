import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import type { FanOut } from "../core-bridge/types";
import { resetFanOut } from "../store/fanOutStore";
import { formatElapsed, SubagentsInspectorView } from "./SubagentsInspector";

const fanOut: FanOut = {
  title: "Map the provider path",
  total: 3,
  done: 1,
  running: 1,
  agents: [
    {
      id: "platform",
      label: "Platform endpoint survey",
      status: "done",
      objective: "Trace the platform boundary.",
      activity: "Complete",
      result: "The route is verified.",
      started_at_ms: 10_000,
      updated_at_ms: 31_000,
    },
    {
      id: "desktop",
      label: "Desktop tool wiring",
      status: "running",
      objective: "Add typed local image tools.",
      activity: "Reviewing the provider-local tool registry",
      attempt: 1,
      started_at_ms: Date.now() - 54_000,
    },
    { id: "verify", label: "Image workflow verification", status: "queued" },
  ],
};

afterEach(() => resetFanOut());

describe("SubagentsInspector", () => {
  it("shows typed objective, activity, status, and progress for the selected child", () => {
    const markup = renderToStaticMarkup(
      <SubagentsInspectorView
        fanOut={fanOut}
        inspectorOpen
        selectedAgentId="desktop"
        selectAgent={() => {}}
        closeInspector={() => {}}
        reduce
      />,
    );

    expect(markup).toContain('aria-label="Subagents"');
    expect(markup).toContain("1 running · 1 complete · 1 queued");
    expect(markup).toContain("Desktop tool wiring");
    expect(markup).toContain("Add typed local image tools.");
    expect(markup).toContain("Reviewing the provider-local tool registry");
    expect(markup).toContain("Jump to transcript");
    expect(markup).not.toContain("Thinking");
  });

  it("formats elapsed time without dropping leading zeroes", () => {
    expect(formatElapsed(54_900)).toBe("00:54");
    expect(formatElapsed(125_000)).toBe("02:05");
  });
});
