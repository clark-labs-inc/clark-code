import { useState } from "react";
import { AlertTriangle, Loader2, RefreshCw, Trash2 } from "lucide-react";
import type { UnavailableConversation as UnavailableConversationState } from "../store/sessionStore.runtime";
import { useSessionStore } from "../store/sessionStore";
import { ProductExceptionalStateIllustration } from "../components/ProductExceptionalStateIllustration";
import { productModule } from "../product/productModule";

export function UnavailableConversationPanel({
  conversation,
  removing,
  cleanupError,
  allowCleanup,
  onRetry,
  onCleanup,
}: {
  conversation: UnavailableConversationState;
  removing: boolean;
  cleanupError: string | null;
  allowCleanup: boolean;
  onRetry: () => void;
  onCleanup: () => void;
}) {
  const [confirmingCleanup, setConfirmingCleanup] = useState(false);

  return (
    <main className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto p-6">
      <section
        aria-labelledby="unavailable-conversation-title"
        className="w-full max-w-lg text-center"
      >
        <ProductExceptionalStateIllustration
          state="recovery"
          size={176}
          className="mx-auto mb-2"
          label={`${productModule().branding.shortName} is reconnecting this conversation`}
          fallback={(
            <span className="mx-auto mb-4 grid size-10 place-items-center rounded-xl bg-warning/10 text-warning">
              <AlertTriangle className="size-5" />
            </span>
          )}
        />
        <p className="truncate text-sm font-medium text-ink-muted">{conversation.title}</p>
        <h1
          id="unavailable-conversation-title"
          className="font-display mt-1 text-2xl leading-tight text-ink"
        >
          {conversation.kind === "refresh_required"
            ? "This chat has a newer version"
            : "This chat isn’t available"}
        </h1>
        <p className="mt-2 text-sm leading-6 text-ink-muted">
          {conversation.kind === "refresh_required"
            ? "Clark Code stopped the stale copy so it can’t overwrite newer history. Reload the latest version to continue; your recent Spec prompts remain in Prompt history."
            : "Clark Code couldn’t reopen it. The chat stays selected so you can retry or remove the unavailable entry from your history."}
        </p>

        {cleanupError && (
          <p role="alert" className="mt-4 rounded-xl bg-danger/10 px-3 py-2 text-sm text-danger">
            {cleanupError}
          </p>
        )}

        <details className="mx-auto mt-4 max-w-md text-left text-xs text-ink-faint">
          <summary className="cursor-pointer select-none font-medium text-ink-muted">
            Technical details
          </summary>
          <p className="mt-2 break-words rounded-lg bg-bg-sunken px-3 py-2 font-mono leading-5">
            {conversation.detail}
          </p>
        </details>

        {allowCleanup && confirmingCleanup ? (
          <div className="mx-auto mt-5 max-w-md rounded-xl border border-danger/20 bg-danger/5 p-3 text-left">
            <p className="text-sm font-medium text-ink">Remove this chat from history?</p>
            <p className="mt-1 text-xs leading-5 text-ink-muted">
              This permanently deletes the unavailable entry and can’t be undone.
            </p>
            <div className="mt-3 flex items-center gap-2">
              <button
                type="button"
                disabled={removing}
                onClick={onCleanup}
                className="flex min-h-9 items-center gap-2 rounded-lg bg-danger/10 px-3 text-sm font-semibold text-danger transition hover:bg-danger/20 disabled:cursor-wait disabled:opacity-60"
              >
                {removing ? (
                  <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
                ) : (
                  <Trash2 className="size-4" />
                )}
                {removing ? "Removing…" : "Remove chat"}
              </button>
              <button
                type="button"
                disabled={removing}
                onClick={() => setConfirmingCleanup(false)}
                className="min-h-9 rounded-lg px-3 text-sm font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-60"
              >
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <div className="mt-5 flex flex-wrap items-center justify-center gap-2">
            <button
              type="button"
              onClick={onRetry}
              className="flex min-h-9 items-center gap-2 rounded-lg bg-accent px-3 text-sm font-semibold text-on-accent transition hover:bg-accent-hover"
            >
              <RefreshCw className="size-4" />
              {conversation.kind === "refresh_required" ? "Reload latest" : "Try again"}
            </button>
            {allowCleanup && (
              <button
                type="button"
                onClick={() => setConfirmingCleanup(true)}
                className="flex min-h-9 items-center gap-2 rounded-lg px-3 text-sm font-medium text-ink-muted transition hover:bg-danger/10 hover:text-danger"
              >
                <Trash2 className="size-4" />
                Clean up
              </button>
            )}
          </div>
        )}
      </section>
    </main>
  );
}

export function UnavailableConversation() {
  const conversation = useSessionStore((state) => state.unavailableConversation);
  const cleanup = useSessionStore((state) => state.cleanupUnavailableConversation);
  const retry = useSessionStore((state) => state.openConversation);
  const cleanupId = useSessionStore((state) => state.unavailableCleanupId);
  const cleanupError = useSessionStore((state) => state.error);
  if (!conversation) return null;

  return (
    <UnavailableConversationPanel
      conversation={conversation}
      removing={cleanupId === conversation.id}
      cleanupError={cleanupError}
      allowCleanup={conversation.kind === "unavailable"}
      onRetry={() => void retry(conversation.id)}
      onCleanup={() => void cleanup()}
    />
  );
}
