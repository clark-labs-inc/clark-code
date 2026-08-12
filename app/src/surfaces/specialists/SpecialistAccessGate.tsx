import {
  ArrowUpRight,
  RefreshCw,
  Sparkles,
} from "lucide-react";

import {
  projectedSpecialistAccess,
  specialistAccessCopy,
  type SpecialistKind,
} from "../../lib/specialists";
import { useSessionStore } from "../../store/sessionStore";
import { productModule } from "../../product/productModule";

type SpecialistGateAction = "sign_in" | "product_action" | "retry" | "setup_workspace" | null;

export function runSpecialistGateAction(
  action: SpecialistGateAction,
  handlers: {
    signIn: () => void;
    retry: () => void;
    productAction: () => void;
    setupWorkspace: () => void;
  },
): void {
  if (action === "sign_in") handlers.signIn();
  else if (action === "retry") handlers.retry();
  else if (action === "product_action") handlers.productAction();
  else if (action === "setup_workspace") handlers.setupWorkspace();
}

export function SpecialistAccessGate({
  kind,
  state,
  onRetry,
  onProductAction,
  onWorkspaceSetup,
}: {
  kind: SpecialistKind;
  state: ReturnType<typeof projectedSpecialistAccess> | "offline";
  onRetry: () => void;
  onProductAction: () => void;
  onWorkspaceSetup: () => void;
}) {
  const copy = specialistAccessCopy(state, kind);
  const signIn = useSessionStore((session) => session.signIn);
  const Icon = productModule().specialistIcons?.[kind] ?? Sparkles;
  const action = () => runSpecialistGateAction(copy.action, {
    signIn: () => void signIn("google"),
    retry: onRetry,
    productAction: onProductAction,
    setupWorkspace: onWorkspaceSetup,
  });

  return (
    <div
      data-qa={`specialist-gate-${kind}`}
      key={state}
      className="flex min-h-0 flex-1 items-center justify-center bg-bg px-6 py-10"
    >
      <div className="w-full max-w-lg border-y border-border px-8 py-10 text-center">
        <span className="mx-auto grid size-12 place-items-center text-accent">
          <Icon className="size-6" />
        </span>
        <h2 className="mt-5 font-serif text-2xl font-semibold tracking-[-0.025em] text-ink">{copy.title}</h2>
        <p className="mx-auto mt-2 max-w-md text-sm leading-6 text-ink-muted">{copy.detail}</p>
        {copy.action && (
          <button
            data-qa={`specialist-gate-action-${kind}`}
            type="button"
            onClick={action}
            className="mt-6 inline-flex items-center gap-2 rounded-xl bg-accent px-4 py-2 text-sm font-semibold text-on-accent shadow-sm transition hover:bg-accent/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
          >
            {copy.action === "retry" ? <RefreshCw className="size-4" /> : <Sparkles className="size-4" />}
            {copy.action === "sign_in"
              ? "Sign in"
              : copy.action === "retry"
                ? "Try again"
                : copy.actionLabel ?? "Review access"}
            {copy.action !== "retry" && <ArrowUpRight className="size-3.5" />}
          </button>
        )}
      </div>
    </div>
  );
}
