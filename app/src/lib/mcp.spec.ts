import { describe, expect, it } from "vitest";
import { mergeDiscoveredMcp } from "./mcp";
import type { McpServer } from "./mcpServers";

describe("mergeDiscoveredMcp", () => {
  it("adds selected missing servers without replacing existing configuration", () => {
    const existing: McpServer[] = [
      {
        id: "existing",
        name: "shared",
        command: "keep-this",
        args: [],
        env: {},
        enabled: false,
      },
    ];
    let nextId = 0;
    const result = mergeDiscoveredMcp(
      existing,
      [
        { name: "shared", command: "replace-this", args: [], env: {} },
        { name: "new", command: "npx", args: ["fixture"], env: { TOKEN: "fake" } },
        { name: "new", command: "duplicate", args: [], env: {} },
      ],
      () => `id-${++nextId}`,
    );

    expect(result).toEqual({
      added: 1,
      servers: [
        existing[0],
        {
          id: "id-1",
          name: "new",
          command: "npx",
          args: ["fixture"],
          env: { TOKEN: "fake" },
          enabled: true,
        },
      ],
    });
    expect(existing).toHaveLength(1);
  });
});
