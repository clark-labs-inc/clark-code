export interface ProductAuthFailure {
  code: "session_expired" | "reconnect_required" | "service_unavailable" | "cancelled" | "account_changed";
  message: string;
}

const CODES = new Set(["session_expired", "reconnect_required", "service_unavailable", "cancelled", "account_changed"]);

export function productAuthFailure(error: unknown): ProductAuthFailure | null {
  try {
    const raw = error instanceof Error ? error.message : error;
    const value = typeof raw === "string" ? JSON.parse(raw) : raw;
    return value && CODES.has(value.code) && typeof value.message === "string"
      ? { code: value.code, message: value.message } : null;
  } catch { return null; }
}
