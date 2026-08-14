import { Loader2, Server, MessageSquare, Laptop } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { ProductExceptionalStateIllustration } from "../components/ProductExceptionalStateIllustration";
import { productModule } from "../product/productModule";

/** Full-pane feedback only when no target conversation can be rendered yet.
 * Cached conversations stay visible while native reattachment runs in the
 * background, matching the persistent-host resume contract. */
export function OpeningScreen() {
  const opening = useSessionStore((s) => s.opening);
  const cancel = useSessionStore((s) => s.endSession);
  if (!opening) return null;

  const isStart = opening.kind === "start";
  const Icon = opening.remoteHost ? Server : isStart ? Laptop : MessageSquare;
  const status = opening.remoteHost
    ? `${isStart ? "Connecting" : "Reconnecting"} to ${opening.remoteHost}… this can take a moment`
    : isStart
      ? "Starting session…"
      : "Opening session…";
  const hasIllustration = Boolean(productModule().exceptionalStateIllustration);

  return (
    <div className="appear-delayed flex flex-1 flex-col items-center justify-center p-6 text-center">
      <ProductExceptionalStateIllustration
        state="loading"
        size={176}
        label={`${productModule().branding.shortName} is opening ${opening.title}`}
        fallback={(
          <span className="relative grid size-14 place-items-center">
            <Loader2 className="absolute size-14 animate-[spin_1s_linear_infinite] text-ink-faint/60" />
            <Icon className="size-5 text-ink-muted" />
          </span>
        )}
      />
      <div className="max-w-md text-center">
        {hasIllustration ? (
          <>
            <p className="mt-3 text-xs font-semibold uppercase tracking-[0.12em] text-accent">
              {isStart ? "Starting workspace" : "Restoring workspace"}
            </p>
            <h1 className="mt-2 font-display text-2xl leading-tight text-ink">
              {productModule().branding.shortName} is getting things ready.
            </h1>
            <p className="mt-2 truncate text-sm font-medium text-ink-secondary">{opening.title}</p>
          </>
        ) : (
          <p className="truncate text-base font-semibold text-ink">{opening.title}</p>
        )}
        <p className="mt-1.5 text-sm text-ink-muted">{status}</p>
      </div>
      <button
        onClick={() => cancel()}
        className="mt-4 rounded-lg px-3 py-1.5 text-sm font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink"
      >
        Cancel
      </button>
    </div>
  );
}
