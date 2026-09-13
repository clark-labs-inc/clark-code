type MobileRemoteFailureCode =
  | "model_unavailable"
  | "invalid_command"
  | "invalid_claim"
  | "desktop_unavailable"
  | "project_unavailable"
  | "conversation_busy"
  | "stale_run"
  | "stale_edit"
  | "stale_permission"
  | "submission_failed"
  | "command_failed";

export class MobileRemoteFailure extends Error {
  constructor(
    readonly code: MobileRemoteFailureCode,
    message: string,
    readonly retryable = false,
  ) {
    super(message);
  }
}

export function remoteFailure(
  code: MobileRemoteFailureCode,
  message: string,
  retryable = false,
): never {
  throw new MobileRemoteFailure(code, message, retryable);
}

export function remoteFailureReceipt(error: unknown): {
  error: string;
  error_code: MobileRemoteFailureCode;
  retryable: boolean;
} {
  if (error instanceof MobileRemoteFailure) {
    return {
      error: error.message.slice(0, 800),
      error_code: error.code,
      retryable: error.retryable,
    };
  }
  return {
    error: String(error).slice(0, 800),
    error_code: "command_failed",
    retryable: false,
  };
}

