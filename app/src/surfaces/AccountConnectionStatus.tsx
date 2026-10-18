import { useSessionStore } from "../store/sessionStore";
import { authConnection } from "../lib/auth";

export function AccountConnectionStatus() {
  const auth = useSessionStore((s) => s.auth);
  const busy = useSessionStore((s) => s.authReconnecting);
  const error = useSessionStore((s) => s.authReconnectError);
  const reconnect = useSessionStore((s) => s.reconnectAuth);
  const cancel = useSessionStore((s) => s.cancelAuthReconnect);
  const connection = authConnection(auth);
  if (!auth || (connection === "ready" && !busy && !error)) return null;
  const needsSignIn = connection === "reconnect_required";
  return (
    <div role={error || needsSignIn ? "alert" : "status"} className="border-b border-border-subtle bg-bg-secondary px-4 py-2 text-sm text-ink-secondary">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span>{busy ? "Reconnecting account… Finish sign-in in your browser if it opens."
          : needsSignIn ? "Reconnect your account to restore cloud sync and phone access."
          : "Account service unavailable. Clark will reconnect automatically."}</span>
        {busy ? (
          <button className="font-medium text-accent hover:underline" onClick={() => void cancel()}>Cancel sign-in</button>
        ) : (
          <button className="font-medium text-accent hover:underline" onClick={() => void reconnect()}>
            {needsSignIn ? "Reconnect account" : "Retry connection"}
          </button>
        )}
      </div>
      {error && <p className="mt-1 text-danger">{error}</p>}
    </div>
  );
}
