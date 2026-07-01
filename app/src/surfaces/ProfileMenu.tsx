import { useEffect, useRef, useState } from "react";
import { CreditCard, ExternalLink, LogOut, Loader2, Brain, SlidersHorizontal } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { clarkBillingUrl, openExternal } from "../lib/account";
import { cn } from "../lib/cn";

function statusTone(status?: string | null): { label: string; tone: string } {
  switch (status) {
    case "active":
      return { label: "Active", tone: "text-success" };
    case "trialing":
      return { label: "Trial", tone: "text-info" };
    case "past_due":
      return { label: "Past due", tone: "text-warning" };
    case "canceled":
      return { label: "Canceled", tone: "text-ink-muted" };
    default:
      return { label: "No plan", tone: "text-ink-muted" };
  }
}

function titleCase(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function formatDate(iso?: string | null): string | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  return new Date(t).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

const ROW = "flex items-center justify-between";
const ACTION =
  "flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-sm text-ink-secondary transition hover:bg-bg-hover";

/** Avatar button → account/subscription popover. Subscription + credit state is
 *  read from Clark; "Manage" opens clarkchat.com (same account → same wallet). */
export function ProfileMenu() {
  const auth = useSessionStore((s) => s.auth);
  const billing = useSessionStore((s) => s.billing);
  const loading = useSessionStore((s) => s.loadingBilling);
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
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open, loadBilling]);

  if (!auth) return null;
  const user = auth.user;
  const sub = billing?.subscription ?? null;
  const st = statusTone(sub?.status);
  const credits = billing?.credits;
  const renews = formatDate(sub?.current_period_end);
  const firstLoad = loading && !billing;

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        aria-label="Account"
        title={user.email ?? user.name}
        className="flex items-center"
      >
        {user.avatar ? (
          <img src={user.avatar} alt="" className="size-7 rounded-full" />
        ) : (
          <span className="grid size-7 place-items-center rounded-full bg-bg-tertiary text-xs font-semibold text-ink-secondary transition hover:bg-bg-hover">
            {user.name.charAt(0).toUpperCase()}
          </span>
        )}
      </button>

      {/* Instant show/hide — no fade (avoids WKWebView half-opacity flicker). */}
      {open && (
        <div
          className="popover-surface absolute right-0 top-full z-30 mt-2 w-72 rounded-xl bg-bg-elevated p-1 shadow-lg ring-1 ring-border-subtle"
        >
            <div className="px-3 py-2.5">
              <div className="truncate text-sm font-medium text-ink">{user.name}</div>
              {user.email && <div className="truncate text-xs text-ink-muted">{user.email}</div>}
            </div>

            <div className="mx-1 border-t border-border-subtle" />

            <div className="space-y-2 px-3 py-2.5">
              <div className={ROW}>
                <span className="text-xs text-ink-muted">Plan</span>
                {firstLoad ? (
                  <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite] text-ink-muted" />
                ) : (
                  <span className="flex items-center gap-1.5 text-sm">
                    <span className="font-medium text-ink">
                      {sub?.plan_key ? titleCase(sub.plan_key) : "Free"}
                    </span>
                    <span className={cn("text-xs", st.tone)}>· {st.label}</span>
                  </span>
                )}
              </div>
              <div className={ROW}>
                <span className="text-xs text-ink-muted">Credits</span>
                <span className="text-sm font-medium tabular-nums text-ink">
                  {credits?.is_unlimited
                    ? "Unlimited"
                    : credits
                      ? credits.available_credits.toLocaleString()
                      : "—"}
                </span>
              </div>
              {renews && (
                <div className={ROW}>
                  <span className="text-xs text-ink-muted">
                    {sub?.cancel_at_period_end ? "Ends" : "Renews"}
                  </span>
                  <span className="text-xs text-ink-secondary">{renews}</span>
                </div>
              )}
            </div>

            <div className="mx-1 border-t border-border-subtle" />

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
                    <span className="block text-[11px] text-ink-faint">
                      Remember facts across chats — per project and globally
                    </span>
                  </span>
                </span>
                <Toggle on={memoriesEnabled} />
              </button>
            </div>

            <div className="mx-1 border-t border-border-subtle" />

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
              Manage subscription &amp; credits
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
              Sign out
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
          "absolute top-0.5 size-[14px] rounded-full bg-white shadow-sm transition-all",
          on ? "left-[15px]" : "left-0.5",
        )}
      />
    </span>
  );
}
