import { invoke } from "@tauri-apps/api/core";
import type { SecurityScanRecord } from "../core-bridge/types";
import type { SecurityScan } from "./specialistCloud";

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function reportFileName(scan: SecurityScan): string {
  const mode = scan.mode.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-") || "scan";
  const date = scan.createdAt.slice(0, 10).replace(/[^0-9-]/g, "") || "report";
  return `security-report-${mode}-${date}.pdf`;
}

/** Open the native save dialog and render the selected scan with the bundled
 * Rust PDF engine. Returns false only when the user cancels the dialog. */
export async function saveSecurityScanPdf(
  scan: SecurityScan,
  localRecord?: SecurityScanRecord,
): Promise<boolean> {
  if (!inTauri()) {
    throw new Error("Security reports can be saved from the desktop app.");
  }
  const { save } = await import("@tauri-apps/plugin-dialog");
  const path = await save({
    title: "Save security report",
    defaultPath: reportFileName(scan),
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (!path) return false;
  await invoke("export_security_scan_pdf", {
    path,
    scan: {
      id: scan.id,
      repositoryId: scan.repositoryId,
      mode: scan.mode,
      model: scan.model,
      status: scan.status,
      createdAt: scan.createdAt,
      generatedAt: new Date().toISOString(),
    },
    localRecord: localRecord ?? null,
  });
  return true;
}
