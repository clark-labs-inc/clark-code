import { describe, expect, it } from "vitest";

import { MockBridge } from "./mockBridge";

describe("MockBridge sidebar fixture", () => {
  it("creates distinct conversation ids for a realistic multi-conversation list", async () => {
    const bridge = new MockBridge();

    const first = await bridge.openSession("local", {}, { kind: "new", options: {} });
    const second = await bridge.openSession("local", {}, { kind: "new", options: {} });
    const third = await bridge.openSession("local", {}, { kind: "new", options: {} });

    expect(first.id).toBe("mock-session");
    expect(new Set([first.id, second.id, third.id]).size).toBe(3);
  });
});
