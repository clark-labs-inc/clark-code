import { describe, expect, it } from "vitest";
import { isLocalDocUri, mdFileName, readDocText } from "./docs";

describe("document artifacts", () => {
  it("distinguishes filesystem paths from fetchable URI schemes", () => {
    expect(isLocalDocUri("/tmp/report.md")).toBe(true);
    expect(isLocalDocUri("C:\\work\\report.md")).toBe(true);
    expect(isLocalDocUri("file:///tmp/report.md")).toBe(true);
    expect(isLocalDocUri("https://example.com/report.md")).toBe(false);
    expect(isLocalDocUri("data:text/markdown,hello")).toBe(false);
  });

  it("reads a small remote markdown data URI", async () => {
    await expect(readDocText("data:text/markdown,%23%20Artifact%20report")).resolves.toBe("# Artifact report");
  });

  it("does not duplicate an existing markdown filename extension", () => {
    expect(mdFileName("Artifact UX recommendations.md")).toBe("Artifact UX recommendations.md");
    expect(mdFileName("notes.markdown")).toBe("notes.markdown");
    expect(mdFileName("report")).toBe("report.md");
  });
});
