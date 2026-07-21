import { describe, expect, it } from "vitest";
import {
  localFileName,
  localPathFromHref,
  markdownUrlTransform,
} from "./fileLinks";

describe("Markdown file links", () => {
  it("recognizes absolute and project-relative file destinations", () => {
    expect(localPathFromHref("/Users/stan/report.docx", "/project")).toBe(
      "/Users/stan/report.docx",
    );
    expect(localPathFromHref("docs/report.md", "/Users/stan/project")).toBe(
      "/Users/stan/project/docs/report.md",
    );
    expect(localPathFromHref("../report.md#summary", "/Users/stan/project/app")).toBe(
      "/Users/stan/project/app/../report.md",
    );
    expect(localPathFromHref("C:\\work\\report.docx", "C:\\project")).toBe(
      "C:\\work\\report.docx",
    );
  });

  it("decodes file URLs and encoded path characters", () => {
    expect(localPathFromHref("file:///Users/stan/My%20Report.pdf", "/project")).toBe(
      "/Users/stan/My Report.pdf",
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
    expect(markdownUrlTransform("file:///Users/stan/report.pdf")).toBe(
      "file:///Users/stan/report.pdf",
    );
    expect(markdownUrlTransform("/Users/stan/report.pdf")).toBe(
      "/Users/stan/report.pdf",
    );
    expect(markdownUrlTransform("javascript:alert(1)")).toBe("");
  });

  it("derives a save-dialog filename on either path style", () => {
    expect(localFileName("/Users/stan/My Report.pdf")).toBe("My Report.pdf");
    expect(localFileName("C:\\work\\report.docx")).toBe("report.docx");
  });
});
