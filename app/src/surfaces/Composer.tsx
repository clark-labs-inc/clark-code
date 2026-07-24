import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type KeyboardEvent,
} from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  ArrowUp, Square, X, CornerDownRight, Pencil, Target, Sparkles,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import type { QueuedMessage, SkillReferenceBlock } from "../store/sessionStore";
import type { SkillCatalogEntry } from "../core-bridge/bridge";
import { useFileDrop, usePaste } from "../lib/attachmentSources";
import { projectFiles } from "../lib/projectFiles";
import {
  composerSubmissionState,
  detectComposerTrigger,
  type ComposerSuggestion,
} from "../lib/composerInput";
import {
  expandPromptSlashCommand,
  goalCommandObjective,
  isCompactCommand,
  slashCommands,
  type SlashCommand,
} from "../lib/slashCommands";
import { listCustomCommands } from "../lib/customCommands";
import { fuzzyFilter, fuzzyFilterFiles } from "../lib/fuzzy";
import { cn } from "../lib/cn";
import { DUR, EASE } from "../lib/motion";
import { inTauri } from "../lib/pickFolder";
import { useComposerAutosize } from "../lib/composerAutosize";
import { useSkillCatalog } from "../lib/useSkillCatalog";
import { AttachmentChips } from "./ComposerAttachments";
import { ComposerContextBar } from "./ComposerContextBar";
import { ComposerAttachmentMenu } from "./ComposerAttachmentMenu";
import { ComposerAutocomplete } from "./ComposerAutocomplete";
import { ComposerPermissionPill } from "./ComposerPermissionPill";
import { ComposerCollaborationPill } from "./ComposerCollaborationPill";
import { ModelPill, UsageChip } from "./ComposerControls";
import { SkillsPanel } from "./SkillsPanel";
import {
  createPendingPaste,
  expandPendingPastes,
  shouldThumbnailPastedText,
  type PendingPaste,
} from "../lib/attachments";
/** Messages typed while a run is active. They send automatically, in order,
 *  when the run finishes — no interruption. Each can be edited or dropped. */
function QueuedMessages({ onEdit }: { onEdit: (q: QueuedMessage) => void }) {
  const queued = useSessionStore((s) => s.queued);
  const session = useSessionStore((s) => s.session);
  const busy = useSessionStore((s) =>
    Object.values(s.snapshot.runs).some((r) => r.status === "running" || r.status === "queued"),
  );
  const steerQueued = useSessionStore((s) => s.steerQueued);
  const removeQueued = useSessionStore((s) => s.removeQueued);
  if (queued.length === 0) return null;
  return (
    <div className="composer-column-width mx-auto mb-2 w-full">
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
              transition={{ duration: DUR.base, ease: EASE.out }}
              className="group flex items-center gap-2 overflow-hidden rounded-xl bg-accent-subtle py-2 pl-3 pr-2"
            >
              <CornerDownRight className="size-3.5 shrink-0 text-ink-faint" />
              <span className="min-w-0 flex-1 truncate text-xs text-ink-secondary">
                {q.text || (q.skills.length > 0 ? "(skills selected)" : "(attachments only)")}
              </span>
              <span className="flex shrink-0 items-center gap-0.5">
                {session?.provider === "local"
                  && busy
                  && q.uploads.length === 0
                  && q.skills.length === 0
                  && (
                  <button
                    onClick={() => void steerQueued(q.id)}
                    aria-label="Steer active run with queued message"
                    title="Send now and steer the active run"
                    className="flex h-6 items-center gap-1 rounded-md px-1.5 text-xs text-ink-muted transition hover:bg-bg-hover hover:text-ink"
                  >
                    <CornerDownRight className="size-3" />
                    Steer
                  </button>
                )}
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

export function Composer() {
  const [value, setValue] = useState("");
  const [caret, setCaret] = useState(0);
  const [projFiles, setProjFiles] = useState<string[]>([]);
  const [customCommands, setCustomCommands] = useState<SlashCommand[]>([]);
  const [selectedSkills, setSelectedSkills] = useState<SkillCatalogEntry[]>([]);
  const [skillsOpen, setSkillsOpen] = useState(false);
  const [sel, setSel] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const [pendingPastes, setPendingPastes] = useState<PendingPaste[]>([]);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const session = useSessionStore((s) => s.session);
  const bridge = useSessionStore((s) => s.bridge);
  const activeProvider = useSessionStore((s) => s.activeProvider);
  const projectMode = useSessionStore((s) => s.projectMode);
  const localCwd = useSessionStore((s) => s.localSettings.cwd);
  const send = useSessionStore((s) => s.send);
  const compactConversation = useSessionStore((s) => s.compactConversation);
  const pickProjectFolder = useSessionStore((s) => s.pickProjectFolder);
  const flashNotice = useSessionStore((s) => s.flashNotice);
  const removeQueued = useSessionStore((s) => s.removeQueued);
  const cancelActive = useSessionStore((s) => s.cancelActive);
  const askSideQuestion = useSessionStore((s) => s.askSideQuestion);
  const cwd = useSessionStore((s) => s.activeProjectRoot ?? s.localSettings.cwd);
  const activeRemote = useSessionStore((s) => s.activeRemote);
  const remote = useMemo(
    () => activeRemote ? { ws_url: activeRemote.ws_url, token: activeRemote.token } : null,
    [activeRemote],
  );
  const localTarget = session ? session.provider === "local" : activeProvider === "local";
  const {
    catalog: skillCatalog,
    setCatalog: setSkillCatalog,
    error: skillCatalogError,
    loading: skillsLoading,
    reload: reloadSkills,
  } = useSkillCatalog(bridge, cwd, remote, localTarget);
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
  const resendFrom = useSessionStore((s) => s.resendFrom);
  const [editTimelineIndex, setEditTimelineIndex] = useState<number | null>(null);
  // Start-screen mode: with no active session the composer starts one on submit
  // (type a task → session begins), gated by the environment's readiness.
  const start = useSessionStore((s) => s.startSession);
  const connecting = useSessionStore((s) => s.connecting);
  const startBlocked = useSessionStore((s) => (s.session ? null : s.startBlockedReason()));
  const startError = useSessionStore((s) => (s.session ? null : s.error));
  const { dragging, handlers } = useFileDrop((files) => void addFiles(files));
  usePaste((files) => void addFiles(files), !connecting);

  useComposerAutosize(
    taRef,
    value,
    attachments.length > 0 || pendingPastes.length > 0,
  );

  // "Edit & resend" staged text from a sent message: load it and focus.
  useEffect(() => {
    if (prefill === null) return;
    setValue(prefill.text);
    setEditTimelineIndex(prefill.timelineIndex ?? null);
    if (prefill.timelineIndex !== undefined) {
      const item = useSessionStore.getState().snapshot.timeline[prefill.timelineIndex];
      if (item?.item === "message" && item.role === "user") {
        const references = item.blocks.filter(
          (block): block is SkillReferenceBlock => block.type === "skill_reference",
        );
        setSelectedSkills(
          references.map((reference) => {
            const current = skillCatalog?.skills.find((skill) => skill.id === reference.id);
            return current ?? {
              id: reference.id,
              revision: reference.revision,
              name: reference.name,
              invocationName: reference.name,
              description: "Pinned skill from conversation history",
              scope: "project",
              origin: "clark",
              source: "conversation history",
              requiredTools: [],
              missingTools: [],
              allowImplicitInvocation: true,
              enabled: true,
              disabledReason: null,
              hasNameCollision: false,
            };
          }),
        );
      }
    }
    setPrefill(null);
    requestAnimationFrame(() => {
      const ta = taRef.current;
      if (ta) {
        ta.focus();
        ta.setSelectionRange(ta.value.length, ta.value.length);
      }
    });
  }, [prefill, setPrefill, skillCatalog]);

  const hasContent = value.trim().length > 0
    || attachments.length > 0
    || pendingPastes.length > 0
    || selectedSkills.length > 0;
  const submission = composerSubmissionState({
    hasContent,
    hasSession: !!session,
    connecting,
    activeProvider,
    projectMode,
    localCwd,
    startBlocked,
    canPickProjectFolder: inTauri(),
  });
  const canSend = submission.canSubmit;

  // --- @-file / slash autocomplete ----------------------------------------
  const trigger = useMemo(() => detectComposerTrigger(value, caret), [value, caret]);

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

  const suggestions = useMemo<ComposerSuggestion[]>(() => {
    if (!trigger || dismissed) return [];
    if (trigger.type === "@") {
      return fuzzyFilterFiles(projFiles, trigger.query, 8).map((path) => ({
        kind: "file" as const,
        path,
      }));
    }
    if (trigger.type === "$") {
      return fuzzyFilter(
        (skillCatalog?.skills ?? []).filter((skill) => skill.enabled),
        trigger.query,
        (skill) => `${skill.invocationName} ${skill.name} ${skill.description}`,
        8,
      ).map((match) => ({ kind: "skill" as const, skill: match.item }));
    }
    const builtins = slashCommands().filter(
      (c) => (!c.needsSession || session) && (!c.localOnly || localTarget),
    );
    const builtinNames = new Set(builtins.map((c) => c.name));
    // Built-ins win name collisions.
    const custom = customCommands.filter((c) => !builtinNames.has(c.name));
    const cmds = [...builtins, ...custom];
    return fuzzyFilter(cmds, trigger.query, (c) => `${c.name} ${c.hint}`, 8).map((m) => ({
      kind: "slash" as const,
      cmd: m.item,
    }));
  }, [
    trigger,
    dismissed,
    projFiles,
    session,
    activeProvider,
    customCommands,
    skillCatalog,
  ]);

  // Keep the highlighted row in range as results change; clear the Escape
  // dismissal once the user edits the trigger again.
  useEffect(() => setSel(0), [trigger?.type, trigger?.query]);
  useEffect(() => setDismissed(false), [value]);

  const syncCaret = () => setCaret(taRef.current?.selectionStart ?? 0);

  const onPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    if (event.clipboardData.files.length > 0) return;
    const text = event.clipboardData.getData("text/plain");
    if (!shouldThumbnailPastedText(text)) return;

    event.preventDefault();
    const paste = createPendingPaste(text, pendingPastes);
    setPendingPastes((current) => [...current, paste]);
  };

  const removePendingPaste = (id: string) => {
    setPendingPastes((current) => current.filter((paste) => paste.id !== id));
  };

  const accept = (s: ComposerSuggestion) => {
    if (!trigger) return;
    if (s.kind === "skill") {
      const before = value.slice(0, trigger.start);
      const after = value.slice(caret);
      const next = before + after;
      setValue(next);
      setSelectedSkills((current) =>
        current.some((skill) => skill.id === s.skill.id) ? current : [...current, s.skill],
      );
      requestAnimationFrame(() => {
        const ta = taRef.current;
        if (ta) {
          ta.focus();
          ta.setSelectionRange(before.length, before.length);
          setCaret(before.length);
        }
      });
      return;
    }
    if (s.kind === "slash") {
      // A prompt-style command (has a body) inserts its text for the user to
      // finish/review; an action-style built-in runs and clears the composer.
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
    if (submission.shouldPickProjectFolder) {
      await pickProjectFolder();
      if (useSessionStore.getState().startBlockedReason()) return;
    }
    const t = expandPromptSlashCommand(expandPendingPastes(value, pendingPastes));
    if (/^\s*\/skills\s*$/.test(t)) {
      setValue("");
      setSkillsOpen(true);
      return;
    }
    if (isCompactCommand(t)) {
      if (!session) {
        flashNotice("Start a conversation before compacting its context.");
        return;
      }
      if (attachments.length > 0 || selectedSkills.length > 0) {
        flashNotice("Remove attachments and selected skills before compacting this conversation.");
        return;
      }
      setValue("");
      setPendingPastes([]);
      setEditTimelineIndex(null);
      await compactConversation();
      return;
    }
    const goalObjective = goalCommandObjective(t);
    if (goalObjective === "") {
      setValue("/goal ");
      flashNotice("Describe what Clark should keep working toward after /goal.");
      requestAnimationFrame(() => {
        taRef.current?.focus();
        taRef.current?.setSelectionRange(6, 6);
        setCaret(6);
      });
      return;
    }
    const editIndex = editTimelineIndex;
    const staleSkill = selectedSkills.find((selected) => {
      const current = skillCatalog?.skills.find((skill) => skill.id === selected.id);
      return !current || !current.enabled || current.revision !== selected.revision;
    });
    if (staleSkill) {
      flashNotice(`$${staleSkill.invocationName} changed or became unavailable. Remove and select it again.`);
      return;
    }
    const skillReferences: SkillReferenceBlock[] = selectedSkills.map((skill) => ({
      type: "skill_reference",
      id: skill.id,
      revision: skill.revision,
      name: skill.invocationName,
    }));
    setValue("");
    setPendingPastes([]);
    setEditTimelineIndex(null);
    setSelectedSkills([]);
    // `/btw <question>` — a forked side question that never interrupts the
    // active run. Needs an open session (the fork reads its transcript), so
    // route it only once a session exists; otherwise let the normal start
    // flow open one and the user re-sends. Takes precedence over edit/resend.
    const btw = t.match(/^\s*\/btw\s+(\S[\s\S]*)/);
    if (btw && session) {
      await askSideQuestion(btw[1].trim());
      return;
    }
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
    if (editIndex !== null) await resendFrom(editIndex, t.trim(), skillReferences);
    else await send(t.trim(), skillReferences);
  };

  const goalIntent = goalCommandObjective(value);

  // Pull a queued message back into the composer to revise it.
  const editQueued = (q: QueuedMessage) => {
    setValue((v) => (v.trim() ? `${v}\n${q.text}` : q.text));
    setSelectedSkills((current) => {
      const byId = new Map(current.map((skill) => [skill.id, skill]));
      for (const reference of q.skills) {
        const catalogSkill = skillCatalog?.skills.find((skill) => skill.id === reference.id);
        if (catalogSkill) byId.set(reference.id, catalogSkill);
      }
      return [...byId.values()];
    });
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
      {/* Keep suggestions in normal layout flow. The old absolute menu was
          trapped below the context bar's stacking layer and visibly painted
          through the checkout chips at compact window heights. */}
      <AnimatePresence>
        {suggestions.length > 0 && (
          <div className="composer-column-width relative z-30 mx-auto mb-2 w-full">
            <ComposerAutocomplete
              suggestions={suggestions}
              selectedIndex={sel}
              onPick={accept}
              onHover={setSel}
            />
          </div>
        )}
      </AnimatePresence>
      <ComposerContextBar />
      <div
        className={cn(
          "composer-column-width relative z-10 mx-auto w-full rounded-[20px] border border-border-subtle bg-composer-surface px-2.5 py-[1.375rem] shadow-soft transition duration-200 ease-clark",
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

        <AttachmentChips pastes={pendingPastes} onRemovePaste={removePendingPaste} />

        {selectedSkills.length > 0 && (
          <div className="flex flex-wrap gap-1.5 pb-1.5">
            {selectedSkills.map((skill) => {
              const current = skillCatalog?.skills.find((candidate) => candidate.id === skill.id);
              const stale = !current || !current.enabled || current.revision !== skill.revision;
              return (
                <span
                  key={skill.id}
                  className={cn(
                    "flex items-center gap-1.5 rounded-lg bg-accent-subtle px-2 py-1 text-xs text-accent",
                    stale && "bg-warning/10 text-warning",
                  )}
                  title={stale ? "This skill changed. Remove and select it again." : skill.description}
                >
                  <Sparkles className="size-3.5" />
                  ${skill.invocationName}
                  <button
                    type="button"
                    onClick={() =>
                      setSelectedSkills((current) =>
                        current.filter((candidate) => candidate.id !== skill.id),
                      )
                    }
                    aria-label={`Remove ${skill.invocationName}`}
                  >
                    <X className="size-3" />
                  </button>
                </span>
              );
            })}
          </div>
        )}

        {goalIntent !== null && (
          <div className="flex items-center gap-1.5 pb-1 pt-0.5 text-xs font-medium text-accent">
            <Target className="size-3.5" />
            <span>Standing goal</span>
            <span className="font-normal text-ink-faint">Clark keeps going until it is done</span>
          </div>
        )}

        {editTimelineIndex !== null && (
          <div className="flex items-center gap-1.5 pb-1 pt-0.5 text-xs text-ink-muted">
            <Pencil className="size-3" />
            <span>Editing message — later turns will be replaced</span>
            <button
              type="button"
              onClick={() => {
                setEditTimelineIndex(null);
                setValue("");
                setSelectedSkills([]);
              }}
              aria-label="Cancel editing message"
              title="Cancel edit"
              className="ml-auto grid size-5 place-items-center rounded text-ink-faint transition hover:bg-bg-hover hover:text-ink-secondary"
            >
              <X className="size-3" />
            </button>
          </div>
        )}

        <textarea
          ref={taRef}
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
            setCaret(e.target.selectionStart ?? 0);
          }}
          onPaste={onPaste}
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
          className="composer-input max-h-52 w-full resize-none overflow-y-auto bg-transparent px-0.5 py-0.5 text-base leading-[1.5] text-ink outline-none placeholder:text-ink-muted disabled:opacity-50"
        />

        <div className="mt-0.5 flex items-center justify-between gap-2">
          <div className="flex items-center gap-1">
            <ComposerAttachmentMenu
              disabled={connecting}
              onFiles={(files) => void addFiles(files)}
            />
            <ComposerPermissionPill />
            <ComposerCollaborationPill />
          </div>

          <div className="flex min-w-0 items-center gap-2.5">
            <UsageChip />
            <ModelPill />
            {busy && !hasContent ? (
              <button
                onClick={() => void cancelActive()}
                aria-label="Stop"
                className="grid size-8 shrink-0 place-items-center rounded-full bg-danger/12 text-danger transition duration-200 ease-clark hover:bg-danger/20"
              >
                <Square className="size-3 fill-current" />
              </button>
            ) : (
              <button
                onClick={() => void submit()}
                disabled={!canSend}
                aria-label={
                  submission.shouldPickProjectFolder
                    ? "Choose project folder and send"
                    : busy
                      ? "Queue message"
                      : "Send"
                }
                title={
                  submission.shouldPickProjectFolder
                    ? "Choose project folder and send"
                    : busy
                      ? "Queue message (sends when Clark finishes)"
                      : "Send · ⇧↵ newline"
                }
                className="grid size-8 shrink-0 place-items-center rounded-full bg-accent text-on-accent shadow-soft transition duration-200 ease-clark hover:-translate-y-0.5 hover:bg-accent-hover active:translate-y-0 disabled:translate-y-0 disabled:bg-bg-tertiary disabled:text-ink-muted disabled:shadow-none"
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
            "composer-column-width mx-auto mt-2 w-full px-1 text-xs",
            startError ? "text-danger" : "text-ink-faint",
          )}
        >
          {startError ?? startBlocked}
        </p>
      )}
      <SkillsPanel
        open={skillsOpen}
        bridge={bridge}
        cwd={cwd}
        remote={remote}
        catalog={skillCatalog}
        loading={skillsLoading}
        error={skillCatalogError}
        onClose={() => setSkillsOpen(false)}
        onReload={reloadSkills}
        onCatalog={setSkillCatalog}
        onSelect={(skill) => {
          setSelectedSkills((current) =>
            current.some((candidate) => candidate.id === skill.id)
              ? current
              : [...current, skill],
          );
          setSkillsOpen(false);
          requestAnimationFrame(() => taRef.current?.focus());
        }}
      />
    </div>
  );
}
