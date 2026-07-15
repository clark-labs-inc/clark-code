import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import {
  X, SlidersHorizontal, FolderGit2, Blocks, SquareTerminal, CircleUser, Info,
  Sun, Moon, Eye, EyeOff, AlertTriangle, ExternalLink, CreditCard, LogOut,
  RefreshCw, Loader2, Trash2, Plus, Server, Brain, Check, FolderOpen,
} from "lucide-react";
import { useSessionStore, type SettingsSection } from "../store/sessionStore";
import { cn } from "../lib/cn";
import { PERMISSION_MODES } from "../lib/permissions";
import { OUTPUT_STYLES } from "../lib/outputStyle";
import { projectName, loadRecentProjects } from "../lib/localAgent";
import { loadMcpServers } from "../lib/mcpServers";
import { loadSshHosts } from "../lib/sshHosts";
import {
  loadAllowlist, loadDenylist, allowCommand, denyCommand, removeAllowed, removeDenied,
} from "../lib/commandPolicy";
import { clarkBillingUrl, openExternal } from "../lib/account";
import { useAppVersion } from "../lib/appInfo";
import { OrganizationKnowledgeSettings } from "./OrganizationKnowledgeSettings";

const input =
  "w-full rounded-lg border border-border bg-bg px-2.5 py-1.5 text-sm text-ink outline-none transition focus:border-accent placeholder:text-ink-muted";

const SECTIONS: { id: SettingsSection; label: string; icon: typeof SlidersHorizontal }[] = [
  { id: "general", label: "General", icon: SlidersHorizontal },
  { id: "project", label: "Project", icon: FolderGit2 },
  { id: "integrations", label: "Integrations", icon: Blocks },
  { id: "commands", label: "Command policy", icon: SquareTerminal },
  { id: "account", label: "Account", icon: CircleUser },
  { id: "about", label: "About & updates", icon: Info },
];

// --- shared presentational bits --------------------------------------------

function GroupLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-2 text-xs font-semibold uppercase tracking-wider text-ink-faint">
      {children}
    </div>
  );
}

function Card({ children }: { children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-xl border border-border-subtle bg-bg-elevated/40 [&>*+*]:border-t [&>*+*]:border-border-subtle">
      {children}
    </div>
  );
}

function Row({ name, sub, children }: { name: React.ReactNode; sub?: React.ReactNode; children?: React.ReactNode }) {
  return (
    <div className="flex items-center gap-3 px-3.5 py-3">
      <div className="min-w-0 flex-1">
        <div className="text-sm text-ink">{name}</div>
        {sub && <div className="mt-0.5 text-xs text-ink-faint">{sub}</div>}
      </div>
      {children}
    </div>
  );
}

function Toggle({ on, onClick, label }: { on: boolean; onClick: () => void; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={onClick}
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
    </button>
  );
}

// --- General ---------------------------------------------------------------

function GeneralSection({
  dark,
  onToggleTheme,
  colorblind,
  onToggleColorblind,
}: {
  dark: boolean;
  onToggleTheme: () => void;
  colorblind: boolean;
  onToggleColorblind: () => void;
}) {
  const permissionMode = useSessionStore((s) => s.permissionMode);
  const setPermissionMode = useSessionStore((s) => s.setPermissionMode);
  const outputStyle = useSessionStore((s) => s.outputStyle);
  const setOutputStyle = useSessionStore((s) => s.setOutputStyle);
  const memoriesEnabled = useSessionStore((s) => s.memoriesEnabled);
  const browserEnabled = useSessionStore((s) => s.browserEnabled);
  const setBrowserEnabled = useSessionStore((s) => s.setBrowserEnabled);
  const setMemoriesEnabled = useSessionStore((s) => s.setMemoriesEnabled);

  const themeBtn = (isDark: boolean, Icon: typeof Sun, text: string) => (
    <button
      onClick={() => dark !== isDark && onToggleTheme()}
      className={cn(
        "flex items-center gap-1.5 rounded-md px-2.5 py-1 transition",
        dark === isDark ? "bg-bg-elevated text-ink shadow-sm" : "text-ink-muted hover:text-ink-secondary",
      )}
    >
      <Icon className="size-3.5" /> {text}
    </button>
  );

  return (
    <div className="space-y-6">
      <div>
        <GroupLabel>Appearance</GroupLabel>
        <Card>
          <Row name="Theme" sub="Violet Paper light · warm graphite dark">
            <div className="inline-flex shrink-0 rounded-lg border border-border-subtle bg-bg-sunken p-0.5 text-xs">
              {themeBtn(false, Sun, "Light")}
              {themeBtn(true, Moon, "Dark")}
            </div>
          </Row>
          <Row name="Colorblind-friendly colors" sub="Blue/orange status instead of red/green">
            <Toggle on={colorblind} onClick={onToggleColorblind} label="Toggle colorblind-friendly colors" />
          </Row>
        </Card>
      </div>

      <div>
        <GroupLabel>Approvals</GroupLabel>
        <Card>
          {PERMISSION_MODES.map((m) => {
            const active = permissionMode === m.id;
            return (
              <button
                key={m.id}
                onClick={() => setPermissionMode(m.id)}
                className={cn(
                  "flex w-full items-start gap-3 px-3.5 py-3 text-left transition",
                  active ? "bg-accent-subtle" : "hover:bg-bg-hover/30",
                )}
              >
                <span
                  className={cn(
                    "mt-0.5 grid size-4 shrink-0 place-items-center rounded-full border",
                    active ? "border-accent" : "border-ink-faint",
                  )}
                >
                  {active && <span className="size-2 rounded-full bg-accent" />}
                </span>
                <span className="min-w-0">
                  <span className="flex items-center gap-1.5 text-sm text-ink">
                    {m.label}
                    {m.id === "full" && <AlertTriangle className="size-3.5 text-warning" />}
                  </span>
                  <span className="mt-0.5 block text-xs text-ink-faint">{m.description}</span>
                </span>
              </button>
            );
          })}
        </Card>
      </div>

      <div>
        <GroupLabel>Output style</GroupLabel>
        <Card>
          {OUTPUT_STYLES.map((style) => {
            const active = outputStyle === style.id;
            return (
              <button
                key={style.id}
                onClick={() => setOutputStyle(style.id)}
                className={cn(
                  "flex w-full items-start gap-3 px-3.5 py-3 text-left transition",
                  active ? "bg-accent-subtle" : "hover:bg-bg-hover/30",
                )}
              >
                <span
                  className={cn(
                    "mt-0.5 grid size-4 shrink-0 place-items-center rounded-full border",
                    active ? "border-accent" : "border-ink-faint",
                  )}
                >
                  {active && <span className="size-2 rounded-full bg-accent" />}
                </span>
                <span className="min-w-0">
                  <span className="block text-sm text-ink">{style.label}</span>
                  <span className="mt-0.5 block text-xs text-ink-faint">{style.description}</span>
                </span>
              </button>
            );
          })}
        </Card>
      </div>

      <div>
        <GroupLabel>Memory</GroupLabel>
        <Card>
          <Row
            name={
              <span className="flex items-center gap-2">
                <Brain className="size-4 text-ink-muted" /> Enable memories
              </span>
            }
            sub="Remember facts across chats — per project and globally"
          >
            <Toggle on={memoriesEnabled} onClick={() => setMemoriesEnabled(!memoriesEnabled)} label="Enable memories" />
          </Row>
        </Card>
      </div>

      <div>
        <GroupLabel>Experimental</GroupLabel>
        <Card>
          <Row
            name={
              <span className="flex items-center gap-2">
                <AlertTriangle className="size-4 text-warning" /> Enable browser tool
              </span>
            }
            sub="Downloads a ~150-300MB browser (clark-browser) on first use. Every action needs your approval."
          >
            <Toggle on={browserEnabled} onClick={() => setBrowserEnabled(!browserEnabled)} label="Enable browser tool" />
          </Row>
        </Card>
      </div>
    </div>
  );
}

// --- Project ---------------------------------------------------------------

function ProjectSection() {
  const cwd = useSessionStore((s) => s.localSettings.cwd);
  const model = useSessionStore((s) => s.localSettings.model);
  const apiKey = useSessionStore((s) => s.localSettings.apiKey);
  const setLocalSettings = useSessionStore((s) => s.setLocalSettings);
  const setProjectFolder = useSessionStore((s) => s.setProjectFolder);
  const pickProjectFolder = useSessionStore((s) => s.pickProjectFolder);
  const [showKey, setShowKey] = useState(false);
  const recents = useMemo(() => loadRecentProjects().filter((p) => p !== cwd), [cwd]);

  return (
    <div className="space-y-6">
      <div>
        <GroupLabel>Project folder</GroupLabel>
        <Card>
          <Row
            name={cwd ? projectName(cwd) : "No folder selected"}
            sub={<span className="font-mono">{cwd || "Pick the folder the agent works in"}</span>}
          >
            <button
              onClick={() => void pickProjectFolder()}
              className="flex shrink-0 items-center gap-1.5 rounded-lg bg-bg-tertiary px-2.5 py-1.5 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover"
            >
              <FolderOpen className="size-3.5" /> Change
            </button>
          </Row>
        </Card>
        {recents.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1.5">
            {recents.map((p) => (
              <button
                key={p}
                onClick={() => setProjectFolder(p)}
                title={p}
                className="max-w-[16rem] truncate rounded-md bg-chip px-2 py-1 text-xs text-ink-secondary transition hover:bg-bg-hover"
              >
                {projectName(p)}
              </button>
            ))}
          </div>
        )}
      </div>

      <div>
        <GroupLabel>Model</GroupLabel>
        <input
          value={model}
          onChange={(e) => setLocalSettings({ model: e.target.value })}
          placeholder="clark-code"
          spellCheck={false}
          className={cn(input, "font-mono")}
        />
        <p className="mt-1.5 text-xs text-ink-faint">Clark tier id (see GET /v1/models).</p>
      </div>

      <OrganizationKnowledgeSettings />

      <div>
        <GroupLabel>API key</GroupLabel>
        <div className="flex items-center gap-2">
          <input
            value={apiKey}
            onChange={(e) => setLocalSettings({ apiKey: e.target.value })}
            type={showKey ? "text" : "password"}
            placeholder="ck_live_…"
            spellCheck={false}
            autoComplete="off"
            className={cn(input, "font-mono")}
          />
          <button
            onClick={() => setShowKey((v) => !v)}
            aria-label={showKey ? "Hide key" : "Show key"}
            className="grid size-9 shrink-0 place-items-center rounded-lg border border-border text-ink-muted transition hover:bg-bg-hover hover:text-ink"
          >
            {showKey ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
          </button>
        </div>
        <p className="mt-1.5 text-xs text-ink-faint">
          Provisioned automatically on sign-in. Stored on this device only.
        </p>
      </div>

      <p className="flex items-center gap-1.5 text-xs text-ink-faint">
        <Info className="size-3.5" /> Changes apply to new sessions.
      </p>
    </div>
  );
}

// --- Integrations ----------------------------------------------------------

function IntegrationsSection() {
  const setSettingsOpen = useSessionStore((s) => s.setSettingsOpen);
  const setMcpOpen = useSessionStore((s) => s.setMcpOpen);
  const setSshOpen = useSessionStore((s) => s.setSshOpen);
  const servers = useMemo(() => loadMcpServers(), []);
  const hosts = useMemo(() => loadSshHosts(), []);
  const mcpEnabled = servers.filter((s) => s.enabled && s.command.trim()).length;

  const manage = (open: () => void) => {
    setSettingsOpen(false);
    open();
  };

  return (
    <div className="space-y-6">
      <div>
        <GroupLabel>Extend Clark Code</GroupLabel>
        <Card>
          <Row
            name={
              <span className="flex items-center gap-2">
                <Blocks className="size-4 text-ink-muted" /> MCP servers
              </span>
            }
            sub={
              servers.length
                ? `${mcpEnabled} enabled · ${servers.length} configured`
                : "Add external tools via Model Context Protocol"
            }
          >
            <button
              onClick={() => manage(() => setMcpOpen(true))}
              className="shrink-0 rounded-lg bg-bg-tertiary px-2.5 py-1.5 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover"
            >
              Manage
            </button>
          </Row>
          <Row
            name={
              <span className="flex items-center gap-2">
                <Server className="size-4 text-ink-muted" /> Remote hosts
              </span>
            }
            sub={
              hosts.length
                ? `${hosts.length} host${hosts.length === 1 ? "" : "s"} saved`
                : "Run the agent on a machine over SSH"
            }
          >
            <button
              onClick={() => manage(() => setSshOpen(true))}
              className="shrink-0 rounded-lg bg-bg-tertiary px-2.5 py-1.5 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover"
            >
              Manage
            </button>
          </Row>
        </Card>
      </div>
    </div>
  );
}

// --- Command policy --------------------------------------------------------

function PolicyList({
  title, items, onAdd, onRemove, value, setValue, placeholder,
}: {
  title: string; items: string[]; onAdd: () => void; onRemove: (c: string) => void;
  value: string; setValue: (v: string) => void; placeholder: string;
}) {
  return (
    <div>
      <GroupLabel>{title}</GroupLabel>
      {items.length > 0 && (
        <Card>
          {items.map((c) => (
            <div key={c} className="flex items-center gap-2 px-3 py-2">
              <code className="min-w-0 flex-1 truncate font-mono text-xs text-ink-secondary">{c}</code>
              <button
                onClick={() => onRemove(c)}
                aria-label={`Remove ${c}`}
                className="grid size-6 shrink-0 place-items-center rounded-md text-ink-muted transition hover:bg-danger/15 hover:text-danger"
              >
                <Trash2 className="size-3.5" />
              </button>
            </div>
          ))}
        </Card>
      )}
      <div className="mt-2 flex items-center gap-2">
        <input
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && onAdd()}
          placeholder={placeholder}
          spellCheck={false}
          className={cn(input, "font-mono text-xs")}
        />
        <button
          onClick={onAdd}
          disabled={!value.trim()}
          className="grid size-9 shrink-0 place-items-center rounded-lg border border-border text-ink-muted transition enabled:hover:bg-bg-hover enabled:hover:text-ink disabled:opacity-40"
        >
          <Plus className="size-4" />
        </button>
      </div>
    </div>
  );
}

function CommandsSection() {
  const cwd = useSessionStore((s) => s.localSettings.cwd);
  const project = cwd.trim();
  const [allow, setAllow] = useState<string[]>([]);
  const [deny, setDeny] = useState<string[]>([]);
  const [allowInput, setAllowInput] = useState("");
  const [denyInput, setDenyInput] = useState("");

  const reload = () => {
    setAllow(loadAllowlist(project));
    setDeny(loadDenylist(project));
  };
  useEffect(reload, [project]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!project) {
    return (
      <p className="rounded-xl border border-border-subtle bg-bg-elevated/40 px-3.5 py-6 text-center text-sm text-ink-muted">
        Open a project to manage its command policy.
      </p>
    );
  }

  return (
    <div className="space-y-6">
      <p className="text-xs text-ink-faint">
        Commands you always allow skip the approval gate for{" "}
        <span className="font-mono text-ink-muted">{projectName(project)}</span>; blocked commands are always
        refused. Saved per project — nothing leaves this device.
      </p>
      <PolicyList
        title="Allowed"
        items={allow}
        value={allowInput}
        setValue={setAllowInput}
        placeholder="e.g. npm run test"
        onAdd={() => {
          allowCommand(project, allowInput);
          setAllowInput("");
          reload();
        }}
        onRemove={(c) => {
          removeAllowed(project, c);
          reload();
        }}
      />
      <PolicyList
        title="Blocked"
        items={deny}
        value={denyInput}
        setValue={setDenyInput}
        placeholder="e.g. rm -rf"
        onAdd={() => {
          denyCommand(project, denyInput);
          setDenyInput("");
          reload();
        }}
        onRemove={(c) => {
          removeDenied(project, c);
          reload();
        }}
      />
    </div>
  );
}

// --- Account ---------------------------------------------------------------

function statusTone(status?: string | null): { label: string; tone: string } {
  switch (status) {
    case "active": return { label: "Active", tone: "text-success" };
    case "trialing": return { label: "Trial", tone: "text-info" };
    case "past_due": return { label: "Past due", tone: "text-warning" };
    case "canceled": return { label: "Canceled", tone: "text-ink-muted" };
    default: return { label: "No plan", tone: "text-ink-muted" };
  }
}

function formatDate(iso?: string | null): string | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  return new Date(t).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

function AccountSection() {
  const auth = useSessionStore((s) => s.auth);
  const billing = useSessionStore((s) => s.billing);
  const loading = useSessionStore((s) => s.loadingBilling);
  const loadBilling = useSessionStore((s) => s.loadBilling);
  const signOut = useSessionStore((s) => s.signOutAuth);
  const setSettingsOpen = useSessionStore((s) => s.setSettingsOpen);

  useEffect(() => {
    void loadBilling();
  }, [loadBilling]);

  if (!auth) return null;
  const user = auth.user;
  const sub = billing?.subscription ?? null;
  const st = statusTone(sub?.status);
  const credits = billing?.credits;
  const renews = formatDate(sub?.current_period_end);
  const firstLoad = loading && !billing;

  return (
    <div className="space-y-6">
      <div>
        <GroupLabel>Signed in</GroupLabel>
        <Card>
          <div className="flex items-center gap-3 px-3.5 py-3">
            {user.avatar ? (
              <img src={user.avatar} alt="" className="size-9 rounded-full" />
            ) : (
              <span className="grid size-9 shrink-0 place-items-center rounded-full bg-bg-tertiary text-sm font-semibold text-ink-secondary">
                {user.name.charAt(0).toUpperCase()}
              </span>
            )}
            <div className="min-w-0">
              <div className="truncate text-sm font-medium text-ink">{user.name}</div>
              {user.email && <div className="truncate text-xs text-ink-muted">{user.email}</div>}
            </div>
          </div>
        </Card>
      </div>

      <div>
        <GroupLabel>Plan & credits</GroupLabel>
        <Card>
          <Row name="Plan">
            {firstLoad ? (
              <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite] text-ink-muted" />
            ) : (
              <span className="flex items-center gap-1.5 text-sm">
                <span className="font-medium text-ink">{sub?.plan_key ? sub.plan_key : "Free"}</span>
                <span className={cn("text-xs", st.tone)}>· {st.label}</span>
              </span>
            )}
          </Row>
          <Row name="Credits">
            <span className="text-sm font-medium tabular-nums text-ink">
              {credits?.is_unlimited
                ? "Unlimited"
                : credits
                  ? credits.available_credits.toLocaleString()
                  : "—"}
            </span>
          </Row>
          {renews && (
            <Row name={sub?.cancel_at_period_end ? "Ends" : "Renews"}>
              <span className="text-sm text-ink-secondary">{renews}</span>
            </Row>
          )}
        </Card>
      </div>

      <div className="space-y-2">
        <button
          onClick={() => void openExternal(clarkBillingUrl())}
          className="flex w-full items-center gap-2.5 rounded-lg border border-border-subtle px-3.5 py-2.5 text-sm text-ink-secondary transition hover:bg-bg-hover"
        >
          <CreditCard className="size-4" /> Manage subscription & credits
          <ExternalLink className="ml-auto size-3.5 text-ink-faint" />
        </button>
        <button
          onClick={() => {
            setSettingsOpen(false);
            signOut();
          }}
          className="flex w-full items-center gap-2.5 rounded-lg border border-border-subtle px-3.5 py-2.5 text-sm text-ink-secondary transition hover:bg-danger/10 hover:text-danger"
        >
          <LogOut className="size-4" /> Sign out
        </button>
      </div>
    </div>
  );
}

// --- About -----------------------------------------------------------------

function AboutSection() {
  const version = useAppVersion();
  const update = useSessionStore((s) => s.update);
  const checkForUpdate = useSessionStore((s) => s.checkForUpdate);
  const applyUpdate = useSessionStore((s) => s.applyUpdate);
  const [checking, setChecking] = useState(false);
  const [checked, setChecked] = useState(false);

  const check = async () => {
    setChecking(true);
    setChecked(false);
    try {
      await checkForUpdate();
    } finally {
      setChecking(false);
      setChecked(true);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <GroupLabel>About</GroupLabel>
        <Card>
          <Row name="Clark Code" sub="Local AI coding agent">
            <span className="font-mono text-sm text-ink-secondary">{version ? `v${version}` : "—"}</span>
          </Row>
        </Card>
      </div>

      <div>
        <GroupLabel>Updates</GroupLabel>
        {update ? (
          <button
            onClick={() => void applyUpdate()}
            className="flex w-full items-center gap-2.5 rounded-lg bg-accent/15 px-3.5 py-2.5 text-sm font-medium text-accent transition hover:bg-accent/25"
          >
            <RefreshCw className="size-4" /> Clark Code {update.version} is ready — restart to update
          </button>
        ) : (
          <button
            onClick={() => void check()}
            disabled={checking}
            className="flex w-full items-center gap-2.5 rounded-lg border border-border-subtle px-3.5 py-2.5 text-sm text-ink-secondary transition hover:bg-bg-hover disabled:opacity-60"
          >
            {checking ? (
              <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
            ) : checked ? (
              <Check className="size-4 text-success" />
            ) : (
              <RefreshCw className="size-4" />
            )}
            {checking ? "Checking…" : checked ? "You're up to date" : "Check for updates"}
          </button>
        )}
      </div>
    </div>
  );
}

// --- Shell -----------------------------------------------------------------

export function Settings({
  dark,
  onToggleTheme,
  colorblind,
  onToggleColorblind,
}: {
  dark: boolean;
  onToggleTheme: () => void;
  colorblind: boolean;
  onToggleColorblind: () => void;
}) {
  const open = useSessionStore((s) => s.settingsOpen);
  const section = useSessionStore((s) => s.settingsSection);
  const setOpen = useSessionStore((s) => s.setSettingsOpen);
  // Reduced Motion (or WKWebView's opacity-fade flicker in general) → appear
  // instantly, no fade. The opacity animation is exactly the "half-opacity
  // flicker" the ProfileMenu popover was fixed for; the modal dialogs never
  // got the same treatment and re-animated regardless of the OS preference.
  const reduce = useReducedMotion();

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, setOpen]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={reduce ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: reduce ? 0 : 0.15 }}
          className="fixed inset-0 z-50 grid place-items-center bg-black/40 p-6"
          onClick={() => setOpen(false)}
        >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-labelledby="settings-title"
            initial={reduce ? false : { opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: reduce ? 0 : 0.12 }}
            onClick={(e) => e.stopPropagation()}
            className="popover-surface flex h-[80vh] max-h-[640px] w-full max-w-3xl overflow-hidden rounded-[22px] border border-border-subtle bg-bg-elevated shadow-lifted"
          >
            {/* Left rail */}
            <nav className="flex w-52 shrink-0 flex-col border-r border-border-subtle bg-bg-secondary/50 p-3">
              <h2 id="settings-title" className="px-2 py-2 text-sm font-semibold text-ink">
                Settings
              </h2>
              {SECTIONS.map((s) => (
                <button
                  key={s.id}
                  onClick={() => setOpen(true, s.id)}
                  className={cn(
                    "flex min-h-9 w-full items-center gap-2.5 rounded-xl px-2.5 py-1.5 text-left text-sm transition duration-200 ease-clark",
                    section === s.id ? "bg-accent-soft text-ink" : "text-ink-secondary hover:bg-accent-subtle",
                  )}
                >
                  <s.icon className={cn("size-4 shrink-0", section === s.id ? "text-accent" : "text-ink-muted")} />
                  {s.label}
                </button>
              ))}
            </nav>

            {/* Right pane */}
            <div className="relative min-w-0 flex-1 overflow-y-auto p-5">
              <button
                onClick={() => setOpen(false)}
                aria-label="Close settings"
                className="absolute right-3 top-3 grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
              >
                <X className="size-4" />
              </button>
              <h3 className="mb-4 text-sm font-semibold text-ink">
                {SECTIONS.find((s) => s.id === section)?.label}
              </h3>
              {section === "general" && (
                <GeneralSection
                  dark={dark}
                  onToggleTheme={onToggleTheme}
                  colorblind={colorblind}
                  onToggleColorblind={onToggleColorblind}
                />
              )}
              {section === "project" && <ProjectSection />}
              {section === "integrations" && <IntegrationsSection />}
              {section === "commands" && <CommandsSection />}
              {section === "account" && <AccountSection />}
              {section === "about" && <AboutSection />}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
