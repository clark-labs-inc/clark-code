import {
  useEffect, useMemo, useRef, useState, type KeyboardEvent, type RefObject,
} from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  ArrowUp, Square, Plus, X, FileText, CornerDownRight, Pencil, Slash,
  Shield, ShieldCheck, ShieldAlert, ChevronDown, Check, ListChecks,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import type { QueuedMessage } from "../store/sessionStore";
import { useFileDrop, usePaste } from "../lib/attachmentSources";
import { prettySize } from "../lib/attachments";
import { CODING_MODELS, REASONING_EFFORTS, modelLabel, effectiveModelSettings } from "../lib/localAgent";
import { PERMISSION_MODES, type PermissionMode } from "../lib/permissions";
import { projectFiles } from "../lib/projectFiles";
import { slashCommands, type SlashCommand } from "../lib/slashCommands";
import { listCustomCommands } from "../lib/customCommands";
import { fuzzyFilter, fuzzyFilterFiles } from "../lib/fuzzy";
import { cn } from "../lib/cn";
import { EnvironmentPicker } from "./EnvironmentPicker";
/** What the user is mid-typing at the caret: an `@file` mention (anywhere) or a
 *  `/command` (only at the very start of the message). */
interface Trigger {
  type: "@" | "/";
  query: string;
  /** Index of the trigger character in the text. */
  start: number;
}

function detectTrigger(text: string, caret: number): Trigger | null {
  for (let i = caret - 1; i >= 0; i--) {
    const ch = text[i];
    if (ch === "@" || ch === "/") {
      const before = i === 0 ? "" : text[i - 1];
      if (i !== 0 && !/\s/.test(before)) return null; // mid-word @ (e.g. an email)
      if (ch === "/" && i !== 0) return null; // slash commands only start the line
      const query = text.slice(i + 1, caret);
      if (/\s/.test(query)) return null; // a space ends the token
      return { type: ch, query, start: i };
    }
    if (/\s/.test(ch)) return null; // ran into whitespace before any trigger
  }
  return null;
}

type Suggestion =
  | { kind: "file"; path: string }
  | { kind: "slash"; cmd: SlashCommand };

/** Close a popover when the user clicks outside of it or presses Escape. The
 *  listeners are registered once (not re-bound every render) and always call
 *  the latest `onClose` via a ref. */
function useOutsideClose(ref: RefObject<HTMLElement | null>, onClose: () => void) {
  const cb = useRef(onClose);
  cb.current = onClose;
  useEffect(() => {
    const handler = (e: Event) => {
      if (ref.current && !ref.current.contains(e.target as Node)) cb.current();
    };
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") cb.current();
    };
    document.addEventListener("mousedown", handler);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", handler);
      document.removeEventListener("keydown", onKey);
    };
  }, [ref]);
}

const MODE_ICON: Record<PermissionMode, typeof Shield> = {
  ask: Shield,
  auto: ShieldCheck,
  full: ShieldAlert,
  plan: ListChecks,
};

/** Codex-style approval policy selector. Full access is the default. */
function PermissionPill() {
  const mode = useSessionStore((s) => s.permissionMode);
  const setMode = useSessionStore((s) => s.setPermissionMode);
  // Permission modes govern the LOCAL engine's gate; a Clark cloud session
  // runs every tool server-side in its own sandbox and never consults them,
  // so showing the pill there would promise control that doesn't exist.
  const isLocalTarget = useSessionStore((s) =>
    s.session ? s.session.provider === "local" : s.activeProvider === "local",
  );
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useOutsideClose(ref, () => setOpen(false));

  const info = PERMISSION_MODES.find((m) => m.id === mode) ?? PERMISSION_MODES[2];
  const Icon = MODE_ICON[mode];
  if (!isLocalTarget) return null;

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        title="How Clark's actions are approved (Shift+Tab to cycle)"
        className={cn(
          "flex min-h-8 items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition duration-200 ease-clark hover:bg-accent-subtle",
          mode === "full" ? "text-warning" :
            mode === "plan" ? "bg-accent-subtle text-accent" : "text-ink-secondary",
        )}
      >
        <Icon className="size-3.5" />
        {info.label}
        <ChevronDown className="size-3 opacity-70" />
      </button>

      {/* Instant show/hide — no fade. A fading anchored popover renders
          half-opacity frames that read as flicker in WKWebView on rapid toggle. */}
      {open && (
        <div
          role="menu"
          className="popover-surface absolute bottom-full left-0 z-30 mb-2 w-72 rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
        >
          <div className="px-2.5 py-1.5 text-xs font-medium uppercase tracking-wide text-ink-faint">
            How should Clark act?
          </div>
          {PERMISSION_MODES.map((m) => {
              const I = MODE_ICON[m.id];
              return (
                <button
                  key={m.id}
                  type="button"
                  role="menuitemradio"
                  aria-checked={m.id === mode}
                  onClick={() => {
                    setMode(m.id);
                    setOpen(false);
                  }}
                  className={cn("flex w-full items-start gap-2.5 rounded-xl px-2.5 py-2.5 text-left transition duration-200 ease-clark hover:bg-accent-subtle", m.id === mode && "bg-accent-subtle")}
                >
                  <I className={cn("mt-0.5 size-4 shrink-0", m.id === "full" ? "text-warning" : m.id === "plan" ? "text-accent" : "text-ink-muted")} />
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm text-ink">{m.label}</span>
                    <span className="block text-xs leading-snug text-ink-muted">{m.description}</span>
                  </span>
                  {m.id === mode && <Check className="mt-0.5 size-4 shrink-0 text-accent" />}
                </button>
              );
            })}
        </div>
      )}
    </div>
  );
}

/** Fallback auto-compact threshold when the engine hasn't reported one yet —
 *  mirrors provider-local DEFAULT_AUTO_COMPACT_TOKEN_LIMIT. Runs stamp the
 *  real per-model limit into `usage.context_limit`, which always wins. */
const CONTEXT_BUDGET_FALLBACK = 300_000;

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1_000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

function fmtCost(usd: number): string {
  if (usd > 0 && usd < 0.01) return "<$0.01";
  return `$${usd.toFixed(2)}`;
}

/** Quiet context-size + cost meter for the viewed conversation. Selectors return
 *  primitives so token-streaming snapshot clones don't re-render this. */
function UsageChip() {
  const contextTokens = useSessionStore((s) => {
    const runs = Object.values(s.snapshot.runs);
    for (let i = runs.length - 1; i >= 0; i--) {
      const u = runs[i].outcome?.usage;
      if (u) return u.context_tokens;
    }
    return 0;
  });
  // The threshold the engine actually compacts at (per-model), reported with
  // each run's usage — so the meter measures against the real number.
  const contextLimit = useSessionStore((s) => {
    const runs = Object.values(s.snapshot.runs);
    for (let i = runs.length - 1; i >= 0; i--) {
      const limit = runs[i].outcome?.usage?.context_limit;
      if (limit) return limit;
    }
    return CONTEXT_BUDGET_FALLBACK;
  });
  const totalIn = useSessionStore((s) =>
    Object.values(s.snapshot.runs).reduce(
      (n, r) => n + (r.outcome?.usage?.input_tokens ?? 0), 0),
  );
  const totalOut = useSessionStore((s) =>
    Object.values(s.snapshot.runs).reduce(
      (n, r) => n + (r.outcome?.usage?.output_tokens ?? 0), 0),
  );
  const cost = useSessionStore((s) =>
    Object.values(s.snapshot.runs).reduce(
      (n, r) => n + (r.outcome?.usage?.cost_usd ?? 0), 0),
  );

  if (totalIn === 0 && totalOut === 0) return null;
  const pct = Math.min(100, Math.round((contextTokens / contextLimit) * 100));
  const high = pct >= 75;

  return (
    <span
      title={`Context: ${contextTokens.toLocaleString()} tokens — ${pct}% of the ${fmtTokens(contextLimit)} auto-compact threshold\nThis conversation: ${totalIn.toLocaleString()} in · ${totalOut.toLocaleString()} out${cost > 0 ? ` · ${fmtCost(cost)}` : ""}`}
      className="hidden items-center gap-1.5 font-mono text-xs tabular-nums text-ink-faint sm:flex"
    >
      {contextTokens > 0 && (
        <span className="flex items-center gap-1">
          {/* Tiny context gauge — fills toward the compaction threshold. */}
          <span className="relative h-1 w-7 overflow-hidden rounded-full bg-bg-tertiary">
            <span
              className={cn("absolute inset-y-0 left-0 rounded-full", high ? "bg-warning" : "bg-ink-faint")}
              style={{ width: `${Math.max(6, pct)}%` }}
            />
          </span>
          {fmtTokens(contextTokens)}
        </span>
      )}
      {cost > 0 && <span>{fmtCost(cost)}</span>}
    </span>
  );
}

/** Model + reasoning-effort picker. Mirrors the PermissionPill's form; a change
 *  mid-conversation hot-swaps the provider's LLM and keeps the transcript.
 *  The displayed model is the ACTIVE chat's effective choice (its per-chat
 *  override, else the global default), so switching models here never leaks
 *  into other chats — each conversation keeps its own. */
function ModelPill() {
  // With a chat open, show + edit THAT chat's model; with none (the start
  // screen) show + edit the global default, which new chats seed from.
  // Select primitives (not a derived object) so token-stream snapshot clones
  // don't re-render the pill — only an actual model/effort change does.
  const sessionId = useSessionStore((s) => s.session?.id ?? null);
  const model = useSessionStore((s) =>
    effectiveModelSettings(s.localSettings, s.chatModels, sessionId).model,
  );
  const effort = useSessionStore((s) =>
    effectiveModelSettings(s.localSettings, s.chatModels, sessionId).reasoningEffort,
  );
  const update = useSessionStore((s) => s.updateModelSettings);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useOutsideClose(ref, () => setOpen(false));

  const effortLabel = REASONING_EFFORTS.find((e) => e.id === effort)?.label;

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        title="Model & reasoning effort"
        className="flex min-h-8 items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium text-ink-secondary transition duration-200 ease-clark hover:bg-accent-subtle hover:text-accent"
      >
        {modelLabel(model)}
        {effort && effortLabel && <span className="text-ink-faint">· {effortLabel}</span>}
        <ChevronDown className="size-3 opacity-70" />
      </button>

      {/* Instant show/hide — no fade (avoids WKWebView half-opacity flicker). */}
      {open && (
        <div
          role="menu"
          className="popover-surface absolute bottom-full right-0 z-30 mb-2 w-72 rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
        >
          <div className="px-2.5 py-1.5 text-xs font-medium uppercase tracking-wide text-ink-faint">
            Model
          </div>
          {CODING_MODELS.map((m) => (
            <button
              key={m.id}
              type="button"
              role="menuitemradio"
              aria-checked={m.id === model}
              onClick={() => {
                void update({ model: m.id });
                setOpen(false);
              }}
              className={cn("flex w-full items-start gap-2.5 rounded-xl px-2.5 py-2.5 text-left transition duration-200 ease-clark hover:bg-accent-subtle", m.id === model && "bg-accent-subtle")}
            >
              <span className="min-w-0 flex-1">
                <span className="block text-sm text-ink">{m.label}</span>
                <span className="block text-xs leading-snug text-ink-muted">{m.hint}</span>
              </span>
              {m.id === model && <Check className="mt-0.5 size-4 shrink-0 text-accent" />}
            </button>
          ))}

          <div className="mx-1.5 my-1 border-t border-border-subtle" />

          <div className="px-2.5 py-1.5 text-xs font-medium uppercase tracking-wide text-ink-faint">
            Reasoning effort
          </div>
          <div className="flex gap-1 px-2.5 pb-2">
            {REASONING_EFFORTS.map((e) => (
              <button
                key={e.id}
                type="button"
                role="menuitemradio"
                aria-checked={e.id === effort}
                onClick={() => void update({ reasoningEffort: e.id })}
                className={cn(
                  "min-h-8 flex-1 rounded-lg px-1 py-1 text-xs font-medium transition duration-200 ease-clark",
                  e.id === effort
                    ? "bg-accent text-on-accent"
                    : "bg-bg-tertiary text-ink-secondary hover:bg-bg-hover",
                )}
              >
                {e.label}
              </button>
            ))}
          </div>
          <p className="px-2.5 pb-1.5 text-xs leading-snug text-ink-faint">
            Auto uses each model's provider default. Applies from the next
            message — the conversation keeps its context.
          </p>
        </div>
      )}
    </div>
  );
}

function AttachmentChips() {
  const attachments = useSessionStore((s) => s.attachments);
  const remove = useSessionStore((s) => s.removeAttachment);
  if (attachments.length === 0) return null;
  return (
    <div className="mb-2 flex flex-wrap gap-2">
      <AnimatePresence initial={false}>
        {attachments.map((a) => (
          <motion.div
            key={a.id}
            layout
            initial={{ opacity: 0, scale: 0.96, y: 3 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.96, y: 2 }}
            transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
            className="group relative flex items-center gap-2 rounded-xl bg-bg-tertiary py-1 pl-1 pr-2"
          >
            {a.previewUrl ? (
              <img src={a.previewUrl} alt="" className="size-8 rounded-md object-cover" />
            ) : (
              <span className="grid size-8 place-items-center rounded-md bg-bg-sunken text-ink-muted">
                <FileText className="size-4" />
              </span>
            )}
            <span className="max-w-40 truncate text-xs text-ink-secondary">{a.filename}</span>
            <span className="text-xs text-ink-faint">{prettySize(a.size)}</span>
            <button
              onClick={() => remove(a.id)}
              aria-label={`Remove ${a.filename}`}
              className="grid size-4 place-items-center rounded-full bg-ink/10 text-ink-muted transition hover:bg-danger/20 hover:text-danger"
            >
              <X className="size-3" />
            </button>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}

/** Messages typed while a run is active. They send automatically, in order,
 *  when the run finishes — no interruption. Each can be edited or dropped. */
function QueuedMessages({ onEdit }: { onEdit: (q: QueuedMessage) => void }) {
  const queued = useSessionStore((s) => s.queued);
  const removeQueued = useSessionStore((s) => s.removeQueued);
  if (queued.length === 0) return null;
  return (
    <div className="mx-auto mb-2 max-w-3xl">
      <div className="mb-1 px-1 text-xs font-medium uppercase tracking-wide text-ink-faint">
        Queued · sends when Clark finishes
      </div>
      <div className="space-y-1">
        <AnimatePresence initial={false}>
          {queued.map((q) => (
            <motion.div
              key={q.id}
              layout
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
              className="group flex items-center gap-2 overflow-hidden rounded-xl bg-accent-subtle py-2 pl-3 pr-2"
            >
              <CornerDownRight className="size-3.5 shrink-0 text-ink-faint" />
              <span className="min-w-0 flex-1 truncate text-xs text-ink-secondary">
                {q.text || "(attachments only)"}
              </span>
              <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition group-hover:opacity-100">
                <button
                  onClick={() => onEdit(q)}
                  aria-label="Edit queued message"
                  className="grid size-6 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
                >
                  <Pencil className="size-3.5" />
                </button>
                <button
                  onClick={() => removeQueued(q.id)}
                  aria-label="Remove queued message"
                  className="grid size-6 place-items-center rounded-md text-ink-muted transition hover:bg-danger/15 hover:text-danger"
                >
                  <X className="size-3.5" />
                </button>
              </span>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>
    </div>
  );
}

/** The `@`-file / `/`-command suggestion list, anchored above the textarea. */
function AutocompletePopover({
  suggestions,
  sel,
  onPick,
  onHover,
}: {
  suggestions: Suggestion[];
  sel: number;
  onPick: (s: Suggestion) => void;
  onHover: (i: number) => void;
}) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.12 }}
      className="popover-surface absolute bottom-full left-0 z-30 mb-2 max-h-64 w-80 overflow-y-auto rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
    >
      {suggestions.map((s, i) => {
        const key = s.kind === "file" ? s.path : `/${s.cmd.name}`;
        return (
          <button
            key={key}
            type="button"
            // Use mousedown so the pick fires before the textarea blurs.
            onMouseDown={(e) => {
              e.preventDefault();
              onPick(s);
            }}
            onMouseMove={() => onHover(i)}
            className={cn(
              "flex w-full items-center gap-2 rounded-xl px-2.5 py-2 text-left text-sm transition duration-200 ease-clark",
              i === sel ? "bg-accent-subtle text-ink" : "text-ink-secondary",
            )}
          >
            {s.kind === "file" ? (
              <>
                <FileText className="size-3.5 shrink-0 text-ink-faint" />
                <span className="min-w-0 flex-1 truncate font-mono text-xs">{s.path}</span>
              </>
            ) : (
              <>
                <Slash className="size-3.5 shrink-0 text-ink-faint" />
                <span className="shrink-0 font-mono text-xs text-ink">/{s.cmd.name}</span>
                <span className="min-w-0 flex-1 truncate text-xs text-ink-faint">{s.cmd.hint}</span>
              </>
            )}
          </button>
        );
      })}
    </motion.div>
  );
}

export function Composer() {
  const [value, setValue] = useState("");
  const [caret, setCaret] = useState(0);
  const [projFiles, setProjFiles] = useState<string[]>([]);
  const [customCommands, setCustomCommands] = useState<SlashCommand[]>([]);
  const [sel, setSel] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const session = useSessionStore((s) => s.session);
  const send = useSessionStore((s) => s.send);
  const removeQueued = useSessionStore((s) => s.removeQueued);
  const cancelActive = useSessionStore((s) => s.cancelActive);
  const cwd = useSessionStore((s) => s.activeProjectRoot ?? s.localSettings.cwd);
  const activeRemote = useSessionStore((s) => s.activeRemote);
  const remote = useMemo(
    () => activeRemote ? { ws_url: activeRemote.ws_url, token: activeRemote.token } : null,
    [activeRemote],
  );
  // Select the derived boolean, NOT the whole `runs` object: the snapshot is
  // re-cloned on every streamed token, so subscribing to `runs` would re-render
  // the composer (and any open popover) dozens of times a second — the flicker.
  // A boolean only re-renders when busy actually flips.
  const busy = useSessionStore((s) =>
    Object.values(s.snapshot.runs).some((r) => r.status === "running" || r.status === "queued"),
  );
  const attachments = useSessionStore((s) => s.attachments);
  const addFiles = useSessionStore((s) => s.addFiles);
  const prefill = useSessionStore((s) => s.composerPrefill);
  const setPrefill = useSessionStore((s) => s.setComposerPrefill);
  // Start-screen mode: with no active session the composer starts one on submit
  // (type a task → session begins), gated by the environment's readiness.
  const start = useSessionStore((s) => s.startSession);
  const connecting = useSessionStore((s) => s.connecting);
  const startBlocked = useSessionStore((s) => (s.session ? null : s.startBlockedReason()));
  const startError = useSessionStore((s) => (s.session ? null : s.error));

  const { dragging, handlers } = useFileDrop((files) => void addFiles(files));
  usePaste((files) => void addFiles(files), !!session);

  useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    const resize = () => {
      ta.style.height = "0px";
      ta.style.height = Math.min(ta.scrollHeight, 200) + "px";
    };
    resize();
    // Re-measure after layout settles: on first mount (the start screen mounts
    // the composer before the flex layout is final) the initial scrollHeight can
    // be stale and over-tall, so a post-layout pass snaps it to the real height.
    const raf = requestAnimationFrame(resize);
    return () => cancelAnimationFrame(raf);
  }, [value]);

  // "Edit & resend" staged text from a sent message: load it and focus.
  useEffect(() => {
    if (prefill === null) return;
    setValue(prefill);
    setPrefill(null);
    requestAnimationFrame(() => {
      const ta = taRef.current;
      if (ta) {
        ta.focus();
        ta.setSelectionRange(ta.value.length, ta.value.length);
      }
    });
  }, [prefill, setPrefill]);

  const hasContent = value.trim().length > 0 || attachments.length > 0;
  const canSend =
    hasContent && (!!session || (!startBlocked && !connecting));

  // --- @-file / slash autocomplete ----------------------------------------
  const trigger = useMemo(() => detectTrigger(value, caret), [value, caret]);

  // Lazily fetch the project file list the first time an @ is typed.
  useEffect(() => {
    if (trigger?.type === "@" && projFiles.length === 0) {
      void projectFiles(cwd, remote).then(setProjFiles);
    }
  }, [trigger, cwd, projFiles.length, remote]);

  // Custom user-authored commands (`.claude/commands/*.md`) — reloaded once
  // per project so newly-added command files show up without a restart.
  useEffect(() => {
    if (!cwd.trim()) {
      setCustomCommands([]);
      return;
    }
    void listCustomCommands(cwd, remote ?? undefined).then((cmds) =>
      setCustomCommands(
        cmds.map((c) => ({ name: c.name, hint: c.description || "Custom command", body: c.body })),
      ),
    );
  }, [cwd, remote]);

  const suggestions = useMemo<Suggestion[]>(() => {
    if (!trigger || dismissed) return [];
    if (trigger.type === "@") {
      return fuzzyFilterFiles(projFiles, trigger.query, 8).map((path) => ({
        kind: "file" as const,
        path,
      }));
    }
    const builtins = slashCommands().filter((c) => !c.needsSession || session);
    const builtinNames = new Set(builtins.map((c) => c.name));
    // Built-ins win name collisions.
    const custom = customCommands.filter((c) => !builtinNames.has(c.name));
    const cmds = [...builtins, ...custom];
    return fuzzyFilter(cmds, trigger.query, (c) => `${c.name} ${c.hint}`, 8).map((m) => ({
      kind: "slash" as const,
      cmd: m.item,
    }));
  }, [trigger, dismissed, projFiles, session, customCommands]);

  // Keep the highlighted row in range as results change; clear the Escape
  // dismissal once the user edits the trigger again.
  useEffect(() => setSel(0), [trigger?.type, trigger?.query]);
  useEffect(() => setDismissed(false), [value]);

  const syncCaret = () => setCaret(taRef.current?.selectionStart ?? 0);

  const accept = (s: Suggestion) => {
    if (!trigger) return;
    if (s.kind === "slash") {
      // A custom command (has a body) inserts its text for the user to review
      // and send; a built-in runs its action and clears the composer.
      if (s.cmd.body !== undefined) {
        const before = value.slice(0, trigger.start);
        const after = value.slice(caret).trimStart();
        const insert = after ? `${s.cmd.body} ${after}` : s.cmd.body;
        const next = before + insert;
        setValue(next);
        requestAnimationFrame(() => {
          const ta = taRef.current;
          if (ta) {
            ta.focus();
            ta.setSelectionRange(next.length, next.length);
            setCaret(next.length);
          }
        });
        return;
      }
      setValue("");
      s.cmd.run?.();
      return;
    }
    const insert = `@${s.path} `;
    const before = value.slice(0, trigger.start);
    const after = value.slice(caret);
    const next = before + insert + after;
    const pos = (before + insert).length;
    setValue(next);
    requestAnimationFrame(() => {
      const ta = taRef.current;
      if (ta) {
        ta.focus();
        ta.setSelectionRange(pos, pos);
        setCaret(pos);
      }
    });
  };

  const submit = async () => {
    if (!canSend) return;
    const t = value;
    setValue("");
    // No session yet → start one on the selected environment, then send. If the
    // connect fails (SSH down, bad folder…) the composer has remounted by then —
    // stage the text as a prefill so the user's task is never lost.
    if (!session) {
      await start();
      if (!useSessionStore.getState().session) {
        useSessionStore.getState().setComposerPrefill(t);
        return;
      }
    }
    await send(t.trim());
  };

  // Pull a queued message back into the composer to revise it.
  const editQueued = (q: QueuedMessage) => {
    setValue((v) => (v.trim() ? `${v}\n${q.text}` : q.text));
    removeQueued(q.id);
    taRef.current?.focus();
  };

  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // While the autocomplete is open, arrows/enter/tab/escape drive it.
    if (suggestions.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSel((s) => (s + 1) % suggestions.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSel((s) => (s - 1 + suggestions.length) % suggestions.length);
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        // Stop Shift+Tab from also reaching the global mode-cycle hotkey
        // while the autocomplete popover is driving Tab/Enter itself.
        e.stopPropagation();
        accept(suggestions[sel] ?? suggestions[0]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setDismissed(true);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    }
    // Esc with an empty composer stops the active run (matches ⌘.).
    if (e.key === "Escape" && busy && !value.trim()) {
      e.preventDefault();
      void cancelActive();
    }
  };

  return (
    <div className="bg-bg px-6 pb-4 pt-2.5" {...handlers}>
      <QueuedMessages onEdit={editQueued} />
      {!session && (
        <div className="mx-auto mb-2 max-w-3xl">
          <EnvironmentPicker />
        </div>
      )}
      <div
        className={cn(
          "relative mx-auto max-w-3xl rounded-[22px] border border-border-subtle bg-bg-elevated px-4 py-2.5 shadow-soft transition duration-200 ease-clark",
          dragging
            ? "ring-2 ring-accent/40"
            : "ring-4 ring-transparent focus-within:border-accent/30 focus-within:ring-accent-subtle",
        )}
      >
        {dragging && (
          <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center rounded-[22px] bg-bg-elevated/90 text-sm font-medium text-accent">
            Drop files to attach
          </div>
        )}

        <AnimatePresence>
          {suggestions.length > 0 && (
            <AutocompletePopover
              suggestions={suggestions}
              sel={sel}
              onPick={accept}
              onHover={setSel}
            />
          )}
        </AnimatePresence>

        <AttachmentChips />

        <input
          ref={fileRef}
          type="file"
          multiple
          hidden
          onChange={(e) => {
            const picked = Array.from(e.target.files ?? []);
            if (picked.length) void addFiles(picked);
            e.target.value = "";
          }}
        />

        <textarea
          ref={taRef}
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
            setCaret(e.target.selectionStart ?? 0);
          }}
          onKeyDown={onKey}
          onSelect={syncCaret}
          onClick={syncCaret}
          rows={1}
          aria-label="Message Clark"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          placeholder={
            !session
              ? "Describe what you want Clark to do…"
              : busy
                ? "Queue a follow-up…"
                : "Ask Clark anything about this project…"
          }
          disabled={connecting}
          className="composer-input max-h-52 w-full resize-none bg-transparent px-0.5 py-1.5 text-base leading-relaxed text-ink outline-none placeholder:text-ink-muted disabled:opacity-50"
        />

        <div className="mt-1.5 flex items-center justify-between gap-2">
          <div className="flex items-center gap-1">
            <button
              onClick={() => fileRef.current?.click()}
              disabled={!session}
              aria-label="Attach files"
              title="Attach files"
              className="grid size-8 shrink-0 place-items-center rounded-full bg-bg-tertiary text-ink-muted transition duration-200 ease-clark hover:bg-accent-subtle hover:text-accent disabled:opacity-40"
            >
              <Plus className="size-4" />
            </button>
            <PermissionPill />
          </div>

          <div className="flex min-w-0 items-center gap-2.5">
            <UsageChip />
            <ModelPill />
            {busy && !hasContent ? (
              <button
                onClick={() => void cancelActive()}
                aria-label="Stop"
                className="grid size-10 shrink-0 place-items-center rounded-full bg-danger/12 text-danger transition duration-200 ease-clark hover:bg-danger/20"
              >
                <Square className="size-3 fill-current" />
              </button>
            ) : (
              <button
                onClick={() => void submit()}
                disabled={!canSend}
                aria-label={busy ? "Queue message" : "Send"}
                title={busy ? "Queue message (sends when Clark finishes)" : "Send · ⇧↵ newline"}
                className="grid size-10 shrink-0 place-items-center rounded-full bg-accent text-on-accent shadow-soft transition duration-200 ease-clark hover:-translate-y-0.5 hover:bg-accent-hover active:translate-y-0 disabled:translate-y-0 disabled:bg-bg-tertiary disabled:text-ink-muted disabled:shadow-none"
              >
                {busy ? <CornerDownRight className="size-4" /> : <ArrowUp className="size-4" />}
              </button>
            )}
          </div>
        </div>
      </div>
      {/* One quiet status line: a connect failure (in red) wins over the
          "what's missing" readiness hint. Connecting itself never shows here —
          the OpeningScreen owns that state. */}
      {!session && (startError || startBlocked) && (
        <p
          className={cn(
            "mx-auto mt-2 max-w-3xl px-1 text-xs",
            startError ? "text-danger" : "text-ink-faint",
          )}
        >
          {startError ?? startBlocked}
        </p>
      )}
    </div>
  );
}
