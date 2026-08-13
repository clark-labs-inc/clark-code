import { describe, expect, it } from "vitest";
import {
  directPdfPreviewUri,
  isLocalDocUri,
  isPreviewableDocument,
  mdFileName,
  pdfFileName,
  readDocText,
  readDocumentPreview,
  toPath,
} from "./docs";

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

  it("turns encoded file URIs into native paths", () => {
    expect(toPath("file:///tmp/the agent%20report.docx")).toBe("/tmp/the agent report.docx");
    expect(toPath("file:///C:/work/report.docx")).toBe("C:/work/report.docx");
    expect(toPath("file://localhost/tmp/report.docx")).toBe("/tmp/report.docx");
  });

  it("recognizes office, presentation, sheet, and PDF preview formats", () => {
    const names = [
      "report.docx",
      "report.odt",
      "sheet.xlsx",
      "sheet.ods",
      "data.csv",
      "deck.pptx",
      "deck.odp",
      "paper.pdf",
    ];
    for (const name of names) {
      expect(isPreviewableDocument(`/workspace/${name}`)).toBe(true);
    }
    expect(isPreviewableDocument(undefined, "report", "application/pdf")).toBe(true);
    expect(isPreviewableDocument("/workspace/image.png")).toBe(false);
  });

  it("previews embedded PDF bytes directly in a browser", async () => {
    const uri = "data:application/pdf;base64,JVBERi0xLjQK";

    expect(directPdfPreviewUri(uri, "report.pdf", "application/pdf")).toBe(uri);
    await expect(readDocumentPreview(uri, "report.pdf", "application/pdf")).resolves.toEqual({
      kind: "direct",
      uri,
    });
    expect(directPdfPreviewUri("data:image/png;base64,AQ==", "image.png", "image/png"))
      .toBeNull();
  });

  it("does not duplicate an existing markdown filename extension", () => {
    expect(mdFileName("Artifact UX recommendations.md")).toBe("Artifact UX recommendations.md");
    expect(mdFileName("notes.markdown")).toBe("notes.markdown");
    expect(mdFileName("report")).toBe("report.md");
  });

  it("derives a PDF filename from Markdown artifact titles", () => {
    expect(pdfFileName("Artifact UX recommendations.md")).toBe("Artifact UX recommendations.pdf");
    expect(pdfFileName("notes.markdown")).toBe("notes.pdf");
    expect(pdfFileName("report")).toBe("report.pdf");
  });
});
