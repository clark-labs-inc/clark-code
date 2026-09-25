import { invoke } from "@tauri-apps/api/core";
import type { SecurityScanRecord } from "../core-bridge/types";

/** Export a local evidence bundle without registering it with a cloud service. */
export async function saveSecurityScanPdf(record: SecurityScanRecord): Promise<boolean> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    throw new Error("Security reports can be saved from the desktop app.");
  }
  const { save } = await import("@tauri-apps/plugin-dialog");
  const mode = record.bundle.mode;
  const path = await save({
    title: "Save security report",
    defaultPath: `security-report-${mode}.pdf`,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (!path) return false;
  await invoke("export_security_scan_pdf", {
    path,
    scan: {
      id: record.bundle.scanId,
      repositoryId: record.bundle.scope,
      mode,
      model: record.bundle.model,
      status: record.seal ? "sealed" : "unsealed",
      createdAt: "Not recorded in local scan metadata",
      generatedAt: new Date().toISOString(),
    },
    localRecord: record,
  });
  return true;
}
