import { useEffect, useRef, useState } from "react";
import {
  CreditCard, ExternalLink, LogOut, Loader2, Brain, SlidersHorizontal, ChevronsUpDown,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import {
  clarkBillingUrl,
  openExternal,
} from "../lib/account";
import {
  billingAccountStatusPresentation,
  projectClarkCodeBilling,
} from "../lib/billing";
import { cn } from "../lib/cn";
import { isClarkAccountReconnectError } from "../lib/errors";

function formatDate(iso?: string | null): string | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  return new Date(t).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

const ROW = "flex items-center justify-between";
const ACTION =
  "flex w-full items-center gap-2.5 rounded-xl px-3 py-2 text-sm text-ink-secondary transition duration-200 ease-clark hover:bg-accent-subtle hover:text-ink";

/** Account trigger → subscription popover. Subscription + credit state is read
 *  from Clark; "Manage" opens clarkchat.com (same account → same wallet).
 *  `topbar` = a compact avatar button (opens down-right); `rail` = the
 *  collapsed sidebar avatar (opens right); `sidebar` = a full-width account
 *  row in the sidebar footer (opens up). */
export function ProfileMenu({ variant = "topbar" }: { variant?: "topbar" | "rail" | "sidebar" }) {
  const auth = useSessionStore((s) => s.auth);
  const billing = useSessionStore((s) => s.billing);
  const loading = useSessionStore((s) => s.loadingBilling);
  const error = useSessionStore((s) => s.error);
  const loadBilling = useSessionStore((s) => s.loadBilling);
  const signOut = useSessionStore((s) => s.signOutAuth);
  const memoriesEnabled = useSessionStore((s) => s.memoriesEnabled);
  const setMemoriesEnabled = useSessionStore((s) => s.setMemoriesEnabled);
  const setSettingsOpen = useSessionStore((s) => s.setSettingsOpen);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    void loadBilling();
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, loadBilling]);

  if (!auth) return null;
  const user = auth.user;
  const billingState = projectClarkCodeBilling(billing);
  const activeBilling = billingState.effective;
  const isTeamBilling = activeBilling?.owner_kind === "organization";
  const sub = activeBilling?.subscription ?? null;
  const accountNeedsReconnect = isClarkAccountReconnectError(error);
  const st = accountNeedsReconnect
    ? { label: "Reconnect", tone: "text-danger" }
    : billingAccountStatusPresentation(billingState.accountStatus);
  const planLabel = billingState.planLabel;
  const limitLabel = billingState.usage.limitLabel;
  const renews = formatDate(sub?.current_period_end);
  const firstLoad = loading && !billing;
  return (
    <div ref={ref} className={cn("relative", variant === "sidebar" && "w-full")}>
      {variant === "sidebar" ? (
        <button
          onClick={() => setOpen((o) => !o)}
          aria-label="Account"
          title={user.email ?? user.name}
          className="flex min-h-9 w-full items-center gap-2 rounded-lg px-2 text-left transition hover:bg-bg-hover"
        >
          {user.avatar ? (
            <img src={user.avatar} alt="" className="size-5 shrink-0 rounded-full" />
          ) : (
            <span className="grid aspect-square min-h-5 min-w-5 shrink-0 place-items-center rounded-full bg-bg-tertiary text-xs font-medium text-ink-secondary">
              {user.name.charAt(0).toUpperCase()}
            </span>
          )}
          <span className="min-w-0 flex-1 truncate text-sm font-medium leading-5 text-ink">{user.name}</span>
          <ChevronsUpDown className="size-3.5 shrink-0 text-ink-faint" />
        </button>
      ) : (
        <button
          onClick={() => setOpen((o) => !o)}
          aria-label="Account"
          title={user.email ?? user.name}
          className="flex items-center"
        >
          {user.avatar ? (
            <img src={user.avatar} alt="" className="size-7 rounded-full" />
          ) : (
            <span className="grid aspect-square min-h-7 min-w-7 shrink-0 place-items-center rounded-full bg-bg-tertiary text-xs font-semibold text-ink-secondary transition hover:bg-bg-hover">
              {user.name.charAt(0).toUpperCase()}
            </span>
          )}
        </button>
      )}

      {/* Instant show/hide — no fade (avoids WKWebView half-opacity flicker). */}
      {open && (
        <div
          className={cn(
            "popover-surface absolute z-30 w-72 rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle",
            variant === "sidebar"
              ? "bottom-full left-0 mb-2"
              : variant === "rail"
                ? "bottom-0 left-full ml-2"
                : "right-0 top-full mt-2",
          )}
        >
            <div className="px-3 py-2.5">
              <div className="truncate text-sm font-medium text-ink">{user.name}</div>
              {user.email && <div className="truncate text-xs text-ink-muted">{user.email}</div>}
            </div>

            {accountNeedsReconnect && (
              <div
                role="alert"
                className="mx-2 rounded-xl border border-danger/20 bg-danger/10 px-3 py-2.5"
              >
                <div className="text-sm font-medium text-danger">Session needs reconnecting</div>
                <div className="mt-0.5 text-xs leading-4 text-ink-secondary">
                  Sign out, then sign in again to reconnect this Clark account.
                </div>
              </div>
            )}

            <div className="space-y-2 px-3 py-2.5">
              {isTeamBilling && (
                <div className={ROW}>
                  <span className="text-xs text-ink-muted">Billing account</span>
                  <span className="max-w-40 truncate text-sm font-medium text-ink">
                    {activeBilling?.display_name ?? "Workspace"}
                  </span>
                </div>
              )}
              <div className={ROW}>
                <span className="text-xs text-ink-muted">Plan</span>
                {firstLoad ? (
                  <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite] text-ink-muted" />
                ) : (
                  <span className="flex items-center gap-1.5 text-sm">
                    <span className="font-medium text-ink">
                      {planLabel}
                    </span>
                    <span className={cn("text-xs", st.tone)}>· {st.label}</span>
                  </span>
                )}
              </div>
              <div className={ROW}>
                <span className="text-xs text-ink-muted">Limit used</span>
                <span className="text-sm font-medium tabular-nums text-ink">
                  {limitLabel}
                </span>
              </div>
              {isTeamBilling && activeBilling?.seat && (
                <div className={ROW}>
                  <span className="text-xs text-ink-muted">Seats</span>
                  <span className="text-xs text-ink-secondary">
                    {activeBilling.seat.purchased} purchased · {activeBilling.seat.assigned} assigned
                  </span>
                </div>
              )}
              {renews && (
                <div className={ROW}>
                  <span className="text-xs text-ink-muted">
                    {sub?.cancel_at_period_end ? "Ends" : "Renews"}
                  </span>
                  <span className="text-xs text-ink-secondary">{renews}</span>
                </div>
              )}
            </div>

            <div className="px-2 py-1.5">
              <button
                type="button"
                role="switch"
                aria-checked={memoriesEnabled}
                onClick={() => setMemoriesEnabled(!memoriesEnabled)}
                className="flex w-full items-center justify-between gap-3 rounded-lg px-1.5 py-1.5 text-left transition hover:bg-bg-hover"
              >
                <span className="flex items-center gap-2.5">
                  <Brain className="size-4 shrink-0 text-ink-muted" />
                  <span className="leading-tight">
                    <span className="block text-sm text-ink-secondary">Enable memories</span>
                    <span className="block text-xs text-ink-faint">
                      Remember facts across chats — per project and globally
                    </span>
                  </span>
                </span>
                <Toggle on={memoriesEnabled} />
              </button>
            </div>

            <button
              onClick={() => {
                setOpen(false);
                setSettingsOpen(true);
              }}
              className={ACTION}
            >
              <SlidersHorizontal className="size-4" />
              Settings
            </button>
            <button onClick={() => void openExternal(clarkBillingUrl())} className={ACTION}>
              <CreditCard className="size-4" />
              Review billing accounts
              <ExternalLink className="ml-auto size-3.5 text-ink-faint" />
            </button>
            <button
              onClick={() => {
                setOpen(false);
                signOut();
              }}
              className={cn(ACTION, "hover:text-danger")}
            >
              <LogOut className="size-4" />
              {accountNeedsReconnect ? "Sign out to reconnect" : "Sign out"}
            </button>
        </div>
      )}
    </div>
  );
}

/** A small on/off switch (presentational; the button owns the click + a11y). */
function Toggle({ on }: { on: boolean }) {
  return (
    <span
      className={cn(
        "relative h-[18px] w-8 shrink-0 rounded-full transition-colors",
        on ? "bg-accent" : "bg-bg-tertiary",
      )}
    >
      <span
        className={cn(
          "absolute left-0.5 top-0.5 size-[14px] rounded-full bg-white shadow-sm transition-transform",
          on ? "translate-x-[13px]" : "translate-x-0",
        )}
      />
    </span>
  );
}
