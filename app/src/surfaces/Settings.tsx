import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import {
  Blocks, Info, Sun, Moon, Eye, EyeOff, AlertTriangle, ExternalLink, CreditCard, LogOut,
  RefreshCw, Loader2, Trash2, Plus, Server, Brain, Check, FolderOpen,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { cn } from "../lib/cn";
import { DUR } from "../lib/motion";
import { APPROVAL_POLICIES } from "../lib/permissions";
import { OUTPUT_STYLES } from "../lib/outputStyle";
import { projectName, loadRecentProjects } from "../lib/localAgent";
import { loadMcpServers } from "../lib/mcpServers";
import { loadSshHosts } from "../lib/sshHosts";
import {
  loadAllowlist, loadDenylist, allowCommand, denyCommand, removeAllowed, removeDenied,
} from "../lib/commandPolicy";
import { billingPlanLabel, clarkBillingUrl, effectiveBalance, effectiveBilling, openExternal } from "../lib/account";
import { useAppVersion } from "../lib/appInfo";
import { TEXT_SIZES, TEXT_SIZE_LABELS, type TextSize } from "../lib/useTextSize";
import { OrganizationKnowledgeSettings } from "./OrganizationKnowledgeSettings";
import { SandboxSetupCard } from "./SandboxSetupCard";
import { GroupLabel, Card, Row, Toggle } from "./settings/Primitives";
import {
  SETTINGS_SECTIONS,
  SettingsNavigation,
} from "./settings/SettingsNavigation";

const input =
  "w-full rounded-lg border border-border bg-bg px-2.5 py-1.5 text-sm text-ink outline-none transition focus:border-accent placeholder:text-ink-muted";

// --- General ---------------------------------------------------------------

function GeneralSection({
  dark,
  onToggleTheme,
  colorblind,
  onToggleColorblind,
  textSize,
  onTextSizeChange,
}: {
  dark: boolean;
  onToggleTheme: () => void;
  colorblind: boolean;
  onToggleColorblind: () => void;
  textSize: TextSize;
  onTextSizeChange: (size: TextSize) => void;
}) {
  const approvalPolicy = useSessionStore((s) => s.approvalPolicy);
  const setApprovalPolicy = useSessionStore((s) => s.setApprovalPolicy);
  const outputStyle = useSessionStore((s) => s.outputStyle);
  const setOutputStyle = useSessionStore((s) => s.setOutputStyle);
  const memoriesEnabled = useSessionStore((s) => s.memoriesEnabled);
  const browserEnabled = useSessionStore((s) => s.browserEnabled);
  const orchestrationEnabled = useSessionStore((s) => s.orchestrationEnabled);
  const setBrowserEnabled = useSessionStore((s) => s.setBrowserEnabled);
  const setOrchestrationEnabled = useSessionStore((s) => s.setOrchestrationEnabled);
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
          <Row name="Text size" sub="Messages, code, terminal · Ctrl/⌘ +/− · Ctrl/⌘ 0 resets">
            <div
              role="radiogroup"
              aria-label="Text size"
              className="inline-flex shrink-0 rounded-lg border border-border-subtle bg-bg-sunken p-0.5 text-xs"
            >
              {TEXT_SIZES.map((size) => (
                <button
                  key={size}
                  type="button"
                  role="radio"
                  aria-checked={textSize === size}
                  onClick={() => onTextSizeChange(size)}
                  className={cn(
                    "rounded-md px-2 py-1 transition",
                    textSize === size
                      ? "bg-bg-elevated text-ink shadow-sm"
                      : "text-ink-muted hover:text-ink-secondary",
                  )}
                >
                  {TEXT_SIZE_LABELS[size]}
                </button>
              ))}
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
          {APPROVAL_POLICIES.map((m) => {
            const active = approvalPolicy === m.id;
            return (
              <button
                key={m.id}
                onClick={() => setApprovalPolicy(m.id)}
                className={cn(
                  "flex w-full items-start gap-3 px-3.5 py-3 text-left transition",
                  active ? "bg-ink/5" : "hover:bg-ink/[0.035]",
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
                  active ? "bg-ink/5" : "hover:bg-ink/[0.035]",
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
            icon={<Brain className="size-4" />}
            name="Enable memories"
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
            icon={<Blocks className="size-4" />}
            name="Parallel coding agents"
            sub="Available by default, but used only when you explicitly ask and the task has independent parts. Writers work in safe copies and Clark asks before applying."
          >
            <Toggle
              on={orchestrationEnabled}
              onClick={() => setOrchestrationEnabled(!orchestrationEnabled)}
              label="Enable parallel coding agents"
            />
          </Row>
          <Row
            icon={<AlertTriangle className="size-4 text-warning" />}
            name="Enable browser tool"
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

      <SandboxSetupCard cwd={cwd} />

      <div>
        <GroupLabel>Default model</GroupLabel>
        <input
          value={model}
          onChange={(e) => setLocalSettings({ model: e.target.value })}
          placeholder="clark-code"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          className={cn(input, "font-mono")}
        />
        <p className="mt-1.5 text-xs text-ink-faint">
          Clark tier id for new chats (see GET /v1/models). Existing chats keep their own
          model.
        </p>
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
            autoCorrect="off"
            autoCapitalize="off"
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
            icon={<Blocks className="size-4" />}
            name="MCP servers"
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
            icon={<Server className="size-4" />}
            name="Remote hosts"
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
          autoCorrect="off"
          autoCapitalize="off"
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
  const activeBilling = effectiveBilling(billing);
  const isTeamBilling = activeBilling?.owner_kind === "organization";
  const sub = activeBilling?.subscription ?? null;
  const st = activeBilling?.coverage_status === "ready"
    ? { label: "Ready", tone: "text-success" }
    : activeBilling?.coverage_status === "action_needed"
      ? { label: "Action needed", tone: "text-warning" }
      : statusTone(sub?.status);
  const planLabel = activeBilling?.plan?.name
    ?? (isTeamBilling ? "Workspace coverage" : billingPlanLabel(sub?.plan_key));
  const credits = effectiveBalance(billing);
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
        <GroupLabel>Clark coverage</GroupLabel>
        <Card>
          {isTeamBilling && (
            <Row name="Billing account">
              <span className="text-sm font-medium text-ink">
                {activeBilling?.display_name ?? "Workspace"}
              </span>
            </Row>
          )}
          <Row name="Plan">
            {firstLoad ? (
              <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite] text-ink-muted" />
            ) : (
              <span className="flex items-center gap-1.5 text-sm">
                <span className="font-medium text-ink">{planLabel}</span>
                <span className={cn("text-xs", st.tone)}>· {st.label}</span>
              </span>
            )}
          </Row>
          <Row name={isTeamBilling ? "Team credits" : "Credits"}>
            <span className="text-sm font-medium tabular-nums text-ink">
              {credits?.is_unlimited
                ? "Unlimited"
                : credits
                  ? credits.available_credits.toLocaleString()
                  : "—"}
            </span>
          </Row>
          {isTeamBilling && activeBilling?.seat && (
            <Row name="Seats">
              <span className="text-sm text-ink-secondary">
                {activeBilling.seat.purchased} purchased · {activeBilling.seat.assigned} assigned
              </span>
            </Row>
          )}
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
          <CreditCard className="size-4" /> Review billing accounts
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
  const updateChecking = useSessionStore((s) => s.updateChecking);
  const updateWaiting = useSessionStore((s) => s.updateWaiting);
  const checkForUpdate = useSessionStore((s) => s.checkForUpdate);
  const applyUpdate = useSessionStore((s) => s.applyUpdate);
  const [checkFeedback, setCheckFeedback] = useState<"up-to-date" | "error" | null>(null);
  const [checkError, setCheckError] = useState<string | null>(null);

  const check = async () => {
    setCheckFeedback(null);
    setCheckError(null);
    const result = await checkForUpdate();
    if (result.status === "up-to-date") setCheckFeedback("up-to-date");
    if (result.status === "error") {
      setCheckFeedback("error");
      setCheckError(result.message);
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
            disabled={updateWaiting}
            aria-label={`Ready to update Clark Code to ${update.version}; restart now`}
            className="flex w-full items-center gap-2.5 rounded-lg bg-accent/15 px-3.5 py-2.5 text-sm font-medium text-accent transition hover:bg-accent/25"
          >
            <RefreshCw className={cn("size-4", updateWaiting && "animate-[spin_1.4s_linear_infinite]")} />{" "}
            {updateWaiting
              ? "Finishing active work before updating…"
              : `Clark Code ${update.version} is ready — restart to update`}
          </button>
        ) : (
          <button
            onClick={() => void check()}
            disabled={updateChecking}
            title={checkError ?? undefined}
            className="flex w-full items-center gap-2.5 rounded-lg border border-border-subtle px-3.5 py-2.5 text-sm text-ink-secondary transition hover:bg-bg-hover disabled:opacity-60"
          >
            {updateChecking ? (
              <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
            ) : checkFeedback === "up-to-date" ? (
              <Check className="size-4 text-success" />
            ) : checkFeedback === "error" ? (
              <AlertTriangle className="size-4 text-danger" />
            ) : (
              <RefreshCw className="size-4" />
            )}
            {updateChecking
              ? "Checking…"
              : checkFeedback === "up-to-date"
                ? "You're up to date"
                : checkFeedback === "error"
                  ? "Couldn't check — try again"
                  : "Check for updates"}
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
  textSize,
  onTextSizeChange,
}: {
  dark: boolean;
  onToggleTheme: () => void;
  colorblind: boolean;
  onToggleColorblind: () => void;
  textSize: TextSize;
  onTextSizeChange: (size: TextSize) => void;
}) {
  const open = useSessionStore((s) => s.settingsOpen);
  const section = useSessionStore((s) => s.settingsSection);
  const setOpen = useSessionStore((s) => s.setSettingsOpen);
  const [query, setQuery] = useState("");
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

  useEffect(() => {
    if (!open) setQuery("");
  }, [open]);

  const activeSection = SETTINGS_SECTIONS.find((item) => item.id === section);

  return (
    <AnimatePresence>
      {open && (
        <motion.section
          role="dialog"
          aria-modal="true"
          aria-labelledby="settings-title"
          initial={reduce ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: reduce ? 0 : DUR.fast }}
          className="fixed inset-0 z-50 flex overflow-hidden bg-bg"
        >
          <SettingsNavigation
            active={section}
            query={query}
            onQueryChange={setQuery}
            onSelect={(next) => setOpen(true, next)}
            onClose={() => setOpen(false)}
          />

          <main className="min-w-0 flex-1 overflow-y-auto bg-bg">
            <div className="mx-auto w-full max-w-[42rem] px-8 pb-20 pt-10 lg:px-10 lg:pt-12">
              <header className="mb-7">
                <h3 className="text-lg font-semibold tracking-tight text-ink">
                  {activeSection?.label}
                </h3>
                {activeSection?.description && (
                  <p className="mt-1 text-sm text-ink-muted">{activeSection.description}</p>
                )}
              </header>
              {section === "general" && (
                <GeneralSection
                  dark={dark}
                  onToggleTheme={onToggleTheme}
                  colorblind={colorblind}
                  onToggleColorblind={onToggleColorblind}
                  textSize={textSize}
                  onTextSizeChange={onTextSizeChange}
                />
              )}
              {section === "project" && <ProjectSection />}
              {section === "integrations" && <IntegrationsSection />}
              {section === "commands" && <CommandsSection />}
              {section === "account" && <AccountSection />}
              {section === "about" && <AboutSection />}
            </div>
          </main>
        </motion.section>
      )}
    </AnimatePresence>
  );
}
