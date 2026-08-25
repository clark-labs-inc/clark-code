import { useEffect, useRef, useState } from "react";
import {
  LogOut, Brain, SlidersHorizontal, ChevronsUpDown,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { cn } from "../lib/cn";
import { isAccountReconnectError } from "../lib/errors";
import { authConnection } from "../lib/auth";
import { productModule } from "../product/productModule";
import { useProductAccess } from "../lib/useProductAccess";
const ACTION =
  "flex w-full items-center gap-2.5 rounded-xl px-3 py-2 text-base text-ink-secondary transition duration-base ease-agent hover:bg-accent-subtle hover:text-ink";

/** Account trigger → product-owned account popover. Product access state and
 * management destinations are supplied by the active product module.
 *  `topbar` = a compact avatar button (opens down-right); `rail` = the
 *  collapsed sidebar avatar (opens right); `sidebar` = a full-width account
 *  row in the sidebar footer (opens up). */
export function ProfileMenu({ variant = "topbar" }: { variant?: "topbar" | "rail" | "sidebar" }) {
  const auth = useSessionStore((s) => s.auth);
  const error = useSessionStore((s) => s.error);
  const signOut = useSessionStore((s) => s.signOutAuth);
  const reconnect = useSessionStore((s) => s.reconnectAuth);
  const memoriesEnabled = useSessionStore((s) => s.memoriesEnabled);
  const setMemoriesEnabled = useSessionStore((s) => s.setMemoriesEnabled);
  const setSettingsOpen = useSessionStore((s) => s.setSettingsOpen);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const AccountSlot = productModule().slots.account;
  const productAccess = useProductAccess(open && Boolean(AccountSlot));

  useEffect(() => {
    if (!open) return;
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
  }, [open]);

  if (!auth) return null;
  const user = auth.user;
  const connection = authConnection(auth);
  const accountNeedsReconnect = connection === "reconnect_required" || isAccountReconnectError(error);
  const accountOffline = connection === "offline";
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
            <span className="grid aspect-square min-h-5 min-w-5 shrink-0 place-items-center rounded-full bg-bg-tertiary text-sm font-medium text-ink-secondary">
              {user.name.charAt(0).toUpperCase()}
            </span>
          )}
          <span className="min-w-0 flex-1 truncate text-base font-medium leading-5 text-ink">{user.name}</span>
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
            <span className="grid aspect-square min-h-7 min-w-7 shrink-0 place-items-center rounded-full bg-bg-tertiary text-sm font-semibold text-ink-secondary transition hover:bg-bg-hover">
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
              <div className="truncate text-base font-medium text-ink">{user.name}</div>
              {user.email && <div className="truncate text-sm text-ink-muted">{user.email}</div>}
            </div>

            {(accountNeedsReconnect || accountOffline) && (
              <div
                role="alert"
                className="mx-2 rounded-xl border border-danger/20 bg-danger/10 px-3 py-2.5"
              >
                <div className="text-base font-medium text-danger">
                  {accountNeedsReconnect ? "Account needs reconnecting" : "Account service unavailable"}
                </div>
                <div className="mt-0.5 text-sm leading-4 text-ink-secondary">
                  {accountNeedsReconnect
                    ? "Local work is safe. Reconnect this account to restore cloud features."
                    : "Local work remains available while Clark reconnects."}
                </div>
                {accountNeedsReconnect && (
                  <button
                    type="button"
                    onClick={() => void reconnect()}
                    className="mt-2 text-sm font-semibold text-accent hover:underline"
                  >
                    Reconnect account
                  </button>
                )}
              </div>
            )}

            {AccountSlot && (
              <AccountSlot
                access={productAccess.access}
                accessLoading={productAccess.loading}
                accessError={productAccess.error}
                reloadAccess={productAccess.reload}
              />
            )}

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
                    <span className="block text-base text-ink-secondary">Enable memories</span>
                    <span className="block text-sm text-ink-faint">
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
          "absolute left-0.5 top-0.5 size-[14px] rounded-full bg-knob shadow-sm transition-transform",
          on ? "translate-x-[13px]" : "translate-x-0",
        )}
      />
    </span>
  );
}
