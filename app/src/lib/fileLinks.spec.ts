import { describe, expect, it } from "vitest";
import {
  localFileName,
  localPathFromHref,
  markdownUrlTransform,
} from "./fileLinks";

describe("Markdown file links", () => {
  it("recognizes absolute and project-relative file destinations", () => {
    expect(localPathFromHref("/workspace/report.docx", "/project")).toBe(
      "/workspace/report.docx",
    );
    expect(localPathFromHref("docs/report.md", "/workspace/project")).toBe(
      "/workspace/project/docs/report.md",
    );
    expect(localPathFromHref("../report.md#summary", "/workspace/project/app")).toBe(
      "/workspace/project/app/../report.md",
    );
    expect(localPathFromHref("C:\\work\\report.docx", "C:\\project")).toBe(
      "C:\\work\\report.docx",
    );
  });

  it("decodes file URLs and encoded path characters", () => {
    expect(localPathFromHref("file:///workspace/My%20Report.pdf", "/project")).toBe(
      "/workspace/My Report.pdf",
    );
    expect(localPathFromHref("docs/My%20Report.pdf", "/project")).toBe(
      "/project/docs/My Report.pdf",
    );
  });

  it("leaves external links and document anchors to web navigation", () => {
    expect(localPathFromHref("https://example.com/report", "/project")).toBeNull();
    expect(localPathFromHref("//example.com/report", "/project")).toBeNull();
    expect(localPathFromHref("mailto:hello@example.com", "/project")).toBeNull();
    expect(localPathFromHref("#summary", "/project")).toBeNull();
  });

  it("preserves local paths without allowing unsafe URL schemes", () => {
    expect(markdownUrlTransform("file:///workspace/report.pdf")).toBe(
      "file:///workspace/report.pdf",
    );
    expect(markdownUrlTransform("/workspace/report.pdf")).toBe(
      "/workspace/report.pdf",
    );
    expect(markdownUrlTransform("https://example.com/report")).toBe(
      "https://example.com/report",
    );
    expect(markdownUrlTransform("mailto:hello@example.com")).toBe(
      "mailto:hello@example.com",
    );
    expect(markdownUrlTransform("javascript:alert(1)")).toBe("");
    expect(markdownUrlTransform("data:text/html,unsafe")).toBe("");
  });

  it("derives a save-dialog filename on either path style", () => {
    expect(localFileName("/workspace/My Report.pdf")).toBe("My Report.pdf");
    expect(localFileName("C:\\work\\report.docx")).toBe("report.docx");
  });
});
