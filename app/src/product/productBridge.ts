import { invoke } from "@tauri-apps/api/core";

const PRODUCT_OPERATION = /^[a-z0-9][a-z0-9._-]{0,127}$/;
const AUTH_CONTROL_OPERATIONS = new Set([
  "account.load",
  "account.refresh",
  "account.sign_in",
  "account.sign_out",
]);

type ProductAuthRecovery = () => Promise<void>;

let authRecovery: ProductAuthRecovery | null = null;
let authRecoveryFlight: Promise<void> | null = null;

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

export function isProductAuthExpiredError(error: unknown): boolean {
  const message = errorText(error);
  return (
    /\b401\b/.test(message) ||
    /Unauthorized/i.test(message) ||
    /ExpiredSignature/i.test(message) ||
    /JWT validation failed/i.test(message)
  );
}

/** Install the app-owned refresh boundary. Product calls retain no credential
 * material; they only wait for the native host to rotate its active session. */
export function configureProductAuthRecovery(
  recover: ProductAuthRecovery | null,
): void {
  authRecovery = recover;
}

/** Coalesce WebView calls and native cloud events onto one native refresh. */
export function recoverProductAuthentication(): Promise<void> {
  if (authRecoveryFlight) return authRecoveryFlight;
  const recover = authRecovery;
  if (!recover) {
    return Promise.reject(new Error("Product authentication recovery is unavailable."));
  }

  let flight: Promise<void>;
  flight = Promise.resolve().then(recover).finally(() => {
    if (authRecoveryFlight === flight) authRecoveryFlight = null;
  });
  authRecoveryFlight = flight;
  return flight;
}

function canRecoverAuthentication(operation: string): boolean {
  return !AUTH_CONTROL_OPERATIONS.has(operation);
}

function invokeProduct<T>(operation: string, payload: unknown): Promise<T> {
  return invoke<T>("product_request", { operation, payload });
}

export async function productRequest<T>(
  operation: string,
  payload: unknown = {},
): Promise<T> {
  if (!PRODUCT_OPERATION.test(operation)) {
    throw new Error("Product operation is invalid");
  }
  const recoverable = canRecoverAuthentication(operation);
  if (recoverable && authRecoveryFlight) await authRecoveryFlight;

  try {
    return await invokeProduct<T>(operation, payload);
  } catch (error) {
    if (!recoverable || !authRecovery || !isProductAuthExpiredError(error)) throw error;
    await recoverProductAuthentication();
    // Authentication failed before the product operation was dispatched. One
    // exact replay is safe; never recurse if the rotated credential is refused.
    return invokeProduct<T>(operation, payload);
  }
}
