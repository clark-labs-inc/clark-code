import { exec } from "node:child_process";

export function exportTenant(tenantId: string, format: string) {
  // Vulnerable: both request values are interpolated into a shell command.
  exec(`/usr/local/bin/export --tenant ${tenantId} --format ${format}`);
}
