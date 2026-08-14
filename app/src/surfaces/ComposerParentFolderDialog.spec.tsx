import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import {
  ComposerParentFolderDialog,
  isAbsoluteFolderPath,
} from "./ComposerParentFolderDialog";

describe("ComposerParentFolderDialog", () => {
  it("accepts Unix, Windows drive, and UNC absolute paths", () => {
    expect(isAbsoluteFolderPath("/Users/stan/Documents/git/clark")).toBe(true);
    expect(isAbsoluteFolderPath("C:\\work\\clark")).toBe(true);
    expect(isAbsoluteFolderPath("\\\\server\\share\\clark")).toBe(true);
    expect(isAbsoluteFolderPath("../clark")).toBe(false);
    expect(isAbsoluteFolderPath("clark")).toBe(false);
  });

  it("renders an actionable browser-preview fallback", () => {
    const markup = renderToStaticMarkup(
      <ComposerParentFolderDialog
        open
        suggestedBase="/Users/stan/Documents/git"
        onCancel={vi.fn()}
        onChoose={vi.fn()}
      />,
    );

    expect(markup).toContain('role="dialog"');
    expect(markup).toContain("Attach a read-only folder");
    expect(markup).toContain("browser preview cannot open the system folder picker");
    expect(markup).toContain("/Users/stan/Documents/git/folder");
    expect(markup).toContain("Attach folder");
  });

  it("labels paths as remote when the active project is over SSH", () => {
    const markup = renderToStaticMarkup(
      <ComposerParentFolderDialog
        open
        suggestedBase="/srv/projects"
        remoteHost="ubuntu@builder"
        onCancel={vi.fn()}
        onChoose={vi.fn()}
      />,
    );

    expect(markup).toContain("Remote folder path");
    expect(markup).toContain("ubuntu@builder");
    expect(markup).toContain("remote checkout");
    expect(markup).not.toContain("browser preview cannot open");
  });
});
