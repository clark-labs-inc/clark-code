import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Blocks, Info, Sun, Moon, AlertTriangle, LogOut,
  Trash2, Plus, Minus, Brain, FolderOpen,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { cn } from "../lib/cn";
import { isAccountReconnectError } from "../lib/errors";
import { authConnection } from "../lib/auth";
import { OVERLAY, accessibleMotion } from "../lib/motion";
import { useModalFocus } from "../lib/modalFocus";
import { APPROVAL_POLICIES } from "../lib/permissions";
import { OUTPUT_STYLES } from "../lib/outputStyle";
import { loadRecentProjects, projectName } from "../lib/localAgent";
import {
  loadAllowlist, loadDenylist, allowCommand, denyCommand, removeAllowed, removeDenied,
} from "../lib/commandPolicy";
import { codeKeyAccountBinding } from "../lib/account";
import { productModule } from "../product/productModule";
import { useProductAccess } from "../lib/useProductAccess";
import { stepTextSize, TEXT_SIZES, type TextSize } from "../lib/useTextSize";
import type { InterfaceContrast } from "../lib/useAppearance";
import { OrganizationKnowledgeSettings } from "./OrganizationKnowledgeSettings";
import { SandboxSetupCard } from "./SandboxSetupCard";
import { GroupLabel, Card, Row, Toggle } from "./settings/Primitives";
import {
  SETTINGS_SECTIONS,
  SettingsNavigation,
} from "./settings/SettingsNavigation";
import { AboutSection } from "./settings/AboutSection";
import { ComputerUseSection } from "./settings/ComputerUseSection";
import { InterfaceContrastControl } from "./settings/InterfaceContrastControl";
import { IntegrationsSection } from "./settings/IntegrationsSection";

const input =
  "w-full rounded-lg bg-bg-secondary px-2.5 py-1.5 text-sm text-ink outline-none transition placeholder:text-ink-muted focus:ring-2 focus:ring-accent/20";

// --- General ---------------------------------------------------------------

function GeneralSection({
  dark,
  onToggleTheme,
  colorblind,
  onToggleColorblind,
  interfaceContrast,
  onInterfaceContrastChange,
  textSize,
  onTextSizeChange,
}: {
  dark: boolean;
  onToggleTheme: () => void;
  colorblind: boolean;
  onToggleColorblind: () => void;
  interfaceContrast: InterfaceContrast;
  onInterfaceContrastChange: (contrast: InterfaceContrast) => void;
  textSize: TextSize;
  onTextSizeChange: (size: TextSize) => void;
}) {
  const approvalPolicy = useSessionStore((s) => s.approvalPolicy);
  const setApprovalPolicy = useSessionStore((s) => s.setDefaultApprovalPolicy);
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
        dark === isDark ? "bg-bg-elevated text-ink" : "text-ink-muted hover:text-ink-secondary",
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
            <div className="flex max-w-full flex-wrap justify-end rounded-lg bg-bg-sunken p-0.5 text-xs">
              {themeBtn(false, Sun, "Light")}
              {themeBtn(true, Moon, "Dark")}
            </div>
          </Row>
          <Row name="Text size" sub="Messages, code, terminal · 100–200% · Ctrl/⌘ +/− · Ctrl/⌘ 0 resets">
            <div
              role="group"
              aria-label="Text size"
              className="flex max-w-full flex-wrap justify-end rounded-lg bg-bg-sunken p-0.5 text-xs"
            >
              <button
                type="button"
                aria-label="Decrease text size"
                disabled={textSize === TEXT_SIZES[0]}
                onClick={() => onTextSizeChange(stepTextSize(textSize, -1))}
                className="grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:pointer-events-none disabled:opacity-35"
              >
                <Minus className="size-3.5" />
              </button>
              <button
                type="button"
                aria-label={`Text size ${textSize}%. Reset to 100%`}
                title="Reset to 100%"
                onClick={() => onTextSizeChange(100)}
                className={cn(
                  "min-w-14 rounded-md px-2 py-1 font-mono tabular-nums transition",
                  textSize === 100
                    ? "text-ink"
                    : "bg-bg-elevated text-ink shadow-sm hover:bg-bg-hover",
                )}
              >
                {textSize}%
              </button>
              <button
                type="button"
                aria-label="Increase text size"
                disabled={textSize === TEXT_SIZES[TEXT_SIZES.length - 1]}
                onClick={() => onTextSizeChange(stepTextSize(textSize, 1))}
                className="grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:pointer-events-none disabled:opacity-35"
              >
                <Plus className="size-3.5" />
              </button>
            </div>
          </Row>
          <InterfaceContrastControl
            value={interfaceContrast}
            onChange={onInterfaceContrastChange}
          />
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
            sub="Available by default, but used only when you explicitly ask and the task has independent parts. Writers work in safe copies and the agent asks before applying."
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
            sub="Downloads a ~150-300MB browser (managed browser) on first use. Every action needs your approval."
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
  const auth = useSessionStore((s) => s.auth);
  const accountScope = codeKeyAccountBinding(auth);
  const cwd = useSessionStore((s) => s.localSettings.cwd);
  const model = useSessionStore((s) => s.localSettings.model);
  const setLocalSettings = useSessionStore((s) => s.setLocalSettings);
  const setProjectFolder = useSessionStore((s) => s.setProjectFolder);
  const pickProjectFolder = useSessionStore((s) => s.pickProjectFolder);
  const recents = useMemo(
    () => loadRecentProjects(accountScope).filter((p) => p !== cwd),
    [accountScope, cwd],
  );

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
          placeholder="local-model"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          className={cn(input, "font-mono")}
        />
        <p className="mt-1.5 text-xs text-ink-faint">
          the agent tier id for new chats (see GET /v1/models). Existing chats keep their own
          model.
        </p>
      </div>

      <OrganizationKnowledgeSettings />

      <p className="flex items-center gap-1.5 text-xs text-ink-faint">
        <Info className="size-3.5" /> Changes apply to new sessions.
      </p>
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
              <code className="min-w-0 flex-1 truncate text-ink-secondary">{c}</code>
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
  const auth = useSessionStore((s) => s.auth);
  const accountScope = codeKeyAccountBinding(auth);
  const project = cwd.trim();
  const [allow, setAllow] = useState<string[]>([]);
  const [deny, setDeny] = useState<string[]>([]);
  const [allowInput, setAllowInput] = useState("");
  const [denyInput, setDenyInput] = useState("");

  const reload = () => {
    setAllow(loadAllowlist(project, accountScope));
    setDeny(loadDenylist(project, accountScope));
  };
  useEffect(reload, [accountScope, project]); // eslint-disable-line react-hooks/exhaustive-deps

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
          allowCommand(project, allowInput, accountScope);
          setAllowInput("");
          reload();
        }}
        onRemove={(c) => {
          removeAllowed(project, c, accountScope);
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
          denyCommand(project, denyInput, accountScope);
          setDenyInput("");
          reload();
        }}
        onRemove={(c) => {
          removeDenied(project, c, accountScope);
          reload();
        }}
      />
    </div>
  );
}

// --- Account ---------------------------------------------------------------

function AccountSection() {
  const auth = useSessionStore((s) => s.auth);
  const error = useSessionStore((s) => s.error);
  const signOut = useSessionStore((s) => s.signOutAuth);
  const reconnect = useSessionStore((s) => s.reconnectAuth);
  const setSettingsOpen = useSessionStore((s) => s.setSettingsOpen);
  const SettingsSlot = productModule().slots.settings;
  const productAccess = useProductAccess(Boolean(SettingsSlot));

  if (!auth) return null;
  const user = auth.user;
  const connection = authConnection(auth);
  const accountNeedsReconnect = connection === "reconnect_required" || isAccountReconnectError(error);
  const accountOffline = connection === "offline";

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
          {(accountNeedsReconnect || accountOffline) && (
            <div
              role="alert"
              className="flex gap-2.5 border-t border-danger/15 bg-danger/10 px-3.5 py-3"
            >
              <AlertTriangle className="mt-0.5 size-4 shrink-0 text-danger" />
              <div>
                <div className="text-sm font-medium text-danger">
                  {accountNeedsReconnect ? "Account needs reconnecting" : "Account service unavailable"}
                </div>
                <div className="mt-0.5 text-xs leading-4 text-ink-secondary">
                  {accountNeedsReconnect
                    ? "Local work is safe. Reconnect this account to restore cloud features."
                    : "Local work remains available while Clark reconnects."}
                </div>
                {accountNeedsReconnect && (
                  <button
                    type="button"
                    onClick={() => void reconnect()}
                    className="mt-2 text-xs font-semibold text-accent hover:underline"
                  >
                    Reconnect account
                  </button>
                )}
              </div>
            </div>
          )}
        </Card>
      </div>

      {SettingsSlot && (
        <SettingsSlot
          access={productAccess.access}
          accessLoading={productAccess.loading}
          accessError={productAccess.error}
          reloadAccess={productAccess.reload}
        />
      )}

      <div className="space-y-2">
        <button
          onClick={() => {
            setSettingsOpen(false);
            signOut();
          }}
          className="flex w-full items-center gap-2.5 rounded-lg border border-border-subtle px-3.5 py-2.5 text-sm text-ink-secondary transition hover:bg-danger/10 hover:text-danger"
        >
          <LogOut className="size-4" />
          Sign out
        </button>
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
  interfaceContrast,
  onInterfaceContrastChange,
  textSize,
  onTextSizeChange,
}: {
  dark: boolean;
  onToggleTheme: () => void;
  colorblind: boolean;
  onToggleColorblind: () => void;
  interfaceContrast: InterfaceContrast;
  onInterfaceContrastChange: (contrast: InterfaceContrast) => void;
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
  const dialogRef = useModalFocus<HTMLElement>(open);

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
        <m.section
          ref={dialogRef}
          {...accessibleMotion(OVERLAY, reduce)}
          role="dialog"
          aria-modal="true"
          aria-labelledby="settings-title"
          tabIndex={-1}
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
                  interfaceContrast={interfaceContrast}
                  onInterfaceContrastChange={onInterfaceContrastChange}
                  textSize={textSize}
                  onTextSizeChange={onTextSizeChange}
                />
              )}
              {section === "project" && <ProjectSection />}
              {section === "integrations" && <IntegrationsSection />}
              {section === "commands" && <CommandsSection />}
              {section === "account" && <AccountSection />}
              {section === "computer-use" && <ComputerUseSection />}
              {section === "about" && <AboutSection />}
            </div>
          </main>
        </m.section>
      )}
    </AnimatePresence>
  );
}
