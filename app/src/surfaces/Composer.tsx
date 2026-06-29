import {
  useEffect, useMemo, useRef, useState, type KeyboardEvent, type RefObject,
} from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  ArrowUp, Square, Plus, X, FileText, CornerDownRight, Pencil, Slash,
  Shield, ShieldCheck, ShieldAlert, ChevronDown, Check,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import type { QueuedMessage } from "../store/sessionStore";
import { useFileDrop, usePaste } from "../lib/attachmentSources";
import { prettySize } from "../lib/attachments";
import { PERMISSION_MODES, type PermissionMode } from "../lib/permissions";
import { projectFiles } from "../lib/projectFiles";
import { slashCommands, type SlashCommand } from "../lib/slashCommands";
import { fuzzyFilter, fuzzyFilterFiles } from "../lib/fuzzy";
import { cn } from "../lib/cn";

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

/** Close a popover when the user clicks outside of it. The listener is
 *  registered once (not re-bound every render) and always calls the latest
 *  `onClose` via a ref. */
function useOutsideClose(ref: RefObject<HTMLElement | null>, onClose: () => void) {
  const cb = useRef(onClose);
  cb.current = onClose;
  useEffect(() => {
    const handler = (e: Event) => {
      if (ref.current && !ref.current.contains(e.target as Node)) cb.current();
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [ref]);
}

const MODE_ICON: Record<PermissionMode, typeof Shield> = {
  ask: Shield,
  auto: ShieldCheck,
  full: ShieldAlert,
};

/** Codex-style approval policy selector. Full access is the default. */
function PermissionPill() {
  const mode = useSessionStore((s) => s.permissionMode);
  const setMode = useSessionStore((s) => s.setPermissionMode);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useOutsideClose(ref, () => setOpen(false));

  const info = PERMISSION_MODES.find((m) => m.id === mode) ?? PERMISSION_MODES[2];
  const Icon = MODE_ICON[mode];

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        title="How Clark's actions are approved"
        className={cn(
          "flex items-center gap-1.5 rounded-lg px-2 py-1 text-xs font-medium transition hover:bg-bg-hover",
          mode === "full" ? "text-warning" : "text-ink-secondary",
        )}
      >
        <Icon className="size-3.5" />
        {info.label}
        <ChevronDown className="size-3 opacity-70" />
      </button>

      <AnimatePresence>
        {open && (
          <motion.div
            key="menu"
            role="menu"
            initial={{ opacity: 0, y: 4, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 4, scale: 0.98 }}
            transition={{ duration: 0.12 }}
            className="absolute bottom-full left-0 z-30 mb-2 w-72 rounded-xl bg-bg-elevated p-1 shadow-lg ring-1 ring-border-subtle"
          >
            <div className="px-2.5 py-1.5 text-[0.7rem] font-medium uppercase tracking-wide text-ink-faint">
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
                  className="flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left transition hover:bg-bg-hover"
                >
                  <I className={cn("mt-0.5 size-4 shrink-0", m.id === "full" ? "text-warning" : "text-ink-muted")} />
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm text-ink">{m.label}</span>
                    <span className="block text-xs leading-snug text-ink-muted">{m.description}</span>
                  </span>
                  {m.id === mode && <Check className="mt-0.5 size-4 shrink-0 text-accent" />}
                </button>
              );
            })}
          </motion.div>
        )}
      </AnimatePresence>
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
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.9 }}
            transition={{ duration: 0.15 }}
            className="group relative flex items-center gap-2 rounded-lg bg-bg-tertiary py-1 pl-1 pr-2"
          >
            {a.previewUrl ? (
              <img src={a.previewUrl} alt="" className="size-8 rounded-md object-cover" />
            ) : (
              <span className="grid size-8 place-items-center rounded-md bg-bg-sunken text-ink-muted">
                <FileText className="size-4" />
              </span>
            )}
            <span className="max-w-40 truncate text-xs text-ink-secondary">{a.filename}</span>
            <span className="text-[0.7rem] text-ink-faint">{prettySize(a.size)}</span>
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
      <div className="mb-1 px-1 text-[0.7rem] font-medium uppercase tracking-wide text-ink-faint">
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
              transition={{ duration: 0.16, ease: [0.4, 0, 0.2, 1] }}
              className="group flex items-center gap-2 overflow-hidden rounded-lg bg-bg-secondary py-1.5 pl-2.5 pr-1.5"
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
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 4 }}
      transition={{ duration: 0.1 }}
      className="absolute bottom-full left-0 z-30 mb-2 max-h-64 w-80 overflow-y-auto rounded-xl bg-bg-elevated p-1 shadow-lg ring-1 ring-border-subtle"
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
              "flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm transition",
              i === sel ? "bg-bg-hover text-ink" : "text-ink-secondary",
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
  const [sel, setSel] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const session = useSessionStore((s) => s.session);
  const send = useSessionStore((s) => s.send);
  const removeQueued = useSessionStore((s) => s.removeQueued);
  const cancelActive = useSessionStore((s) => s.cancelActive);
  const cwd = useSessionStore((s) => s.localSettings.cwd);
  // Select the derived boolean, NOT the whole `runs` object: the snapshot is
  // re-cloned on every streamed token, so subscribing to `runs` would re-render
  // the composer (and any open popover) dozens of times a second — the flicker.
  // A boolean only re-renders when busy actually flips.
  const busy = useSessionStore((s) =>
    Object.values(s.snapshot.runs).some((r) => r.status === "running" || r.status === "queued"),
  );
  const attachments = useSessionStore((s) => s.attachments);
  const addFiles = useSessionStore((s) => s.addFiles);
  const model = useSessionStore((s) => s.localSettings.model);

  const { dragging, handlers } = useFileDrop((files) => void addFiles(files));
  usePaste((files) => void addFiles(files), !!session);

  useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "0px";
    ta.style.height = Math.min(ta.scrollHeight, 200) + "px";
  }, [value]);

  const hasContent = value.trim().length > 0 || attachments.length > 0;
  const canSend = !!session && hasContent;

  // --- @-file / slash autocomplete ----------------------------------------
  const trigger = useMemo(() => detectTrigger(value, caret), [value, caret]);

  // Lazily fetch the project file list the first time an @ is typed.
  useEffect(() => {
    if (trigger?.type === "@" && projFiles.length === 0) {
      void projectFiles(cwd).then(setProjFiles);
    }
  }, [trigger, cwd, projFiles.length]);

  const suggestions = useMemo<Suggestion[]>(() => {
    if (!trigger || dismissed) return [];
    if (trigger.type === "@") {
      return fuzzyFilterFiles(projFiles, trigger.query, 8).map((path) => ({
        kind: "file" as const,
        path,
      }));
    }
    const cmds = slashCommands().filter((c) => !c.needsSession || session);
    return fuzzyFilter(cmds, trigger.query, (c) => `${c.name} ${c.hint}`, 8).map((m) => ({
      kind: "slash" as const,
      cmd: m.item,
    }));
  }, [trigger, dismissed, projFiles, session]);

  // Keep the highlighted row in range as results change; clear the Escape
  // dismissal once the user edits the trigger again.
  useEffect(() => setSel(0), [trigger?.type, trigger?.query]);
  useEffect(() => setDismissed(false), [value]);

  const syncCaret = () => setCaret(taRef.current?.selectionStart ?? 0);

  const accept = (s: Suggestion) => {
    if (!trigger) return;
    if (s.kind === "slash") {
      setValue("");
      s.cmd.run();
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
  };

  return (
    <div className="bg-bg px-5 py-3.5" {...handlers}>
      <QueuedMessages onEdit={editQueued} />
      <div
        className={cn(
          "relative mx-auto max-w-3xl rounded-2xl bg-bg-elevated px-3 py-2.5 shadow-sm transition",
          dragging
            ? "ring-2 ring-accent/40"
            : "ring-1 ring-transparent focus-within:ring-border-subtle",
        )}
      >
        {dragging && (
          <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center rounded-2xl bg-bg-elevated/90 text-sm font-medium text-ink">
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
          placeholder={
            !session
              ? "Start a session to begin"
              : busy
                ? "Queue a follow-up…"
                : "Ask Clark to make a change…"
          }
          disabled={!session}
          className="composer-input max-h-52 w-full resize-none bg-transparent px-0.5 py-1 text-sm leading-relaxed text-ink outline-none placeholder:text-ink-muted disabled:opacity-50"
        />

        <div className="mt-1.5 flex items-center justify-between gap-2">
          <div className="flex items-center gap-1">
            <button
              onClick={() => fileRef.current?.click()}
              disabled={!session}
              aria-label="Attach files"
              title="Attach files"
              className="grid size-7 shrink-0 place-items-center rounded-full bg-bg-tertiary text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-40"
            >
              <Plus className="size-4" />
            </button>
            <PermissionPill />
          </div>

          <div className="flex min-w-0 items-center gap-2.5">
            <span className="hidden truncate font-mono text-xs text-ink-faint sm:inline">
              {model || "clark-code"}
            </span>
            {busy && !hasContent ? (
              <button
                onClick={() => void cancelActive()}
                aria-label="Stop"
                className="grid size-8 shrink-0 place-items-center rounded-full bg-danger/12 text-danger transition hover:bg-danger/20"
              >
                <Square className="size-3 fill-current" />
              </button>
            ) : (
              <button
                onClick={() => void submit()}
                disabled={!canSend}
                aria-label={busy ? "Queue message" : "Send"}
                title={busy ? "Queue message (sends when Clark finishes)" : "Send · ⇧↵ newline"}
                className="grid size-8 shrink-0 place-items-center rounded-full bg-accent text-on-accent transition hover:bg-accent-hover disabled:bg-bg-tertiary disabled:text-ink-muted"
              >
                {busy ? <CornerDownRight className="size-4" /> : <ArrowUp className="size-4" />}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
