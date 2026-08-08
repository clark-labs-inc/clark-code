const MOBILE_REMOTE_RETRY_MIN_MS = 1_000;
export const MOBILE_REMOTE_RETRY_MAX_MS = 30_000;

export function mobileRemoteRetryDelayMs(consecutiveFailures: number): number {
  const exponent = Math.max(0, Math.floor(consecutiveFailures) - 1);
  return Math.min(MOBILE_REMOTE_RETRY_MAX_MS, MOBILE_REMOTE_RETRY_MIN_MS * 2 ** exponent);
}
