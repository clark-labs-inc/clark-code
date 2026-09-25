import { afterEach, describe, expect, it, vi } from "vitest";
import type { SecurityScanRecord } from "../core-bridge/types";

const { invoke, save } = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save }));
import { saveSecurityScanPdf } from "./securityReport";

const record = {
  path: ".agent/security-scans/local/scan.json",
  bundle: { scanId: "local", mode: "standard", model: "test", scope: "src" },
  seal: null,
} as SecurityScanRecord;

afterEach(() => { vi.unstubAllGlobals(); vi.clearAllMocks(); });

describe("local Security artifact export", () => {
  it("exports the supplied unsealed artifact without cloud identity or registration", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    save.mockResolvedValue("/tmp/report.pdf");
    expect(await saveSecurityScanPdf(record)).toBe(true);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("export_security_scan_pdf", {
      path: "/tmp/report.pdf",
      scan: { id: "local", repositoryId: "src", mode: "standard", model: "test", status: "unsealed",
        createdAt: "Not recorded in local scan metadata", generatedAt: expect.any(String) },
      localRecord: record,
    });
  });

  it("does not export when the save dialog is canceled", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    save.mockResolvedValue(null);
    expect(await saveSecurityScanPdf(record)).toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  });
});
