import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ClipboardEvent, KeyboardEvent } from "react";
import { AnimatePresence } from "motion/react";
import {
  ArrowUp, Square, X, CornerDownRight, Pencil, Target, Sparkles,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import type { QueuedMessage, SkillReferenceBlock } from "../store/sessionStore";
import { effectiveApprovalPolicy } from "../store/sessionStore.runtime";
import {
  getBridge,
  type LocalSandboxStatus,
  type SkillCatalogEntry,
} from "../core-bridge/bridge";
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
  sideQuestionCommandQuestion,
  slashCommands,
  type SlashCommand,
} from "../lib/slashCommands";
import { listCustomCommands } from "../lib/customCommands";
import { fuzzyFilter, fuzzyFilterProjectPaths } from "../lib/fuzzy";
import { cn } from "../lib/cn";
import { humanizeError } from "../lib/errors";
import { inTauri } from "../lib/pickFolder";
import { useComposerAutosize } from "../lib/composerAutosize";
import {
  composerDraftRef,
  composerDraftOwner,
  loadComposerDraft,
  moveComposerDraft,
} from "../lib/composerDraft";
import { useComposerDraftState } from "../lib/useComposerDraftState";
import { useSkillCatalog } from "../lib/useSkillCatalog";
import { useSubscriptionWorkflowGate } from "../lib/useSubscriptionWorkflowGate";
import { AttachmentChips } from "./ComposerAttachments";
import { ComposerContextBar } from "./ComposerContextBar";
import { ComposerAttachmentMenu } from "./ComposerAttachmentMenu";
import { ComposerAutocomplete } from "./ComposerAutocomplete";
import { ComposerPermissionPill } from "./ComposerPermissionPill";
import { ComposerCollaborationPill } from "./ComposerCollaborationPill";
import { ComposerQueuedMessages } from "./ComposerQueuedMessages";
import { ComposerSubscriptionWorkflowGate } from "./ComposerSubscriptionWorkflowGate";
import { ModelPill } from "./ComposerControls";
import {
  SandboxSetupCard,
  sandboxBlocksSubmission,
  sandboxGateRequired,
  sandboxStatusForCwd,
  type LocalSandboxObservation,
} from "./SandboxSetupCard";
import { SkillsPanel } from "./SkillsPanel";
import {
  createPendingPaste,
  expandPendingPastes,
  shouldThumbnailPastedText,
  type PendingPaste,
} from "../lib/attachments";
import { specialistSlashIntent, specialistWorkflowAvailable, withActiveSpecialistSkill } from "../lib/specialists";
import { useSpecialistStore } from "../store/specialistStore";

export function Composer() {
  const session = useSessionStore((s) => s.session);
  const auth = useSessionStore((s) => s.auth);
  const draftOwner = composerDraftOwner(auth?.user ?? null);
  const sessionId = session?.id ?? null;
  const activeSpecialist = useSpecialistStore((state) => state.active);
  const draftConversationId = sessionId ?? (activeSpecialist ? `specialist:${activeSpecialist}:new` : null);
  const draft = useComposerDraftState(draftOwner, draftConversationId);
  const { value } = draft;
  const draftValueRef = draft.valueRef;
  const setDraftValue = draft.setValue;
  const [caret, setCaret] = useState(0);
  const [projFiles, setProjFiles] = useState<string[]>([]);
  const [customCommands, setCustomCommands] = useState<SlashCommand[]>([]);
  const [selectedSkills, setSelectedSkills] = useState<SkillCatalogEntry[]>([]);
  const [skillsOpen, setSkillsOpen] = useState(false);
  const [sel, setSel] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const [pendingPastes, setPendingPastes] = useState<PendingPaste[]>([]);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const bridge = useSessionStore((s) => s.bridge);
  const activeProvider = useSessionStore((s) => s.activeProvider);
  const approvalPolicy = useSessionStore((s) =>
    effectiveApprovalPolicy(s.approvalPolicy, s.approvalPolicies, s.session?.id),
  );
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
    () => activeRemote ? { id: activeRemote.id } : null,
    [activeRemote],
  );
  const localTarget = session ? session.provider === "local" : activeProvider === "local";
  const specialistSession = Boolean(activeSpecialist) || session?.provider === "specialist";
  const [sandboxObservation, setSandboxObservation] =
    useState<LocalSandboxObservation | null>(null);
  const sandboxStatus = sandboxStatusForCwd(sandboxObservation, cwd);
  const updateSandboxStatus = useCallback(
    (status: LocalSandboxStatus | null) => {
      setSandboxObservation({ cwd, status });
    },
    [cwd],
  );
  const {
    catalog: skillCatalog,
    setCatalog: setSkillCatalog,
    error: skillCatalogError,
    loading: skillsLoading,
    reload: reloadSkills,
  } = useSkillCatalog(bridge, cwd, remote, localTarget);
  const busy = useSessionStore((s) =>
    Object.values(s.snapshot.runs).some((r) => r.status === "running" || r.status === "queued"),
  );
  const attachments = useSessionStore((s) => s.attachments);
  const addFiles = useSessionStore((s) => s.addFiles);
  const prefill = useSessionStore((s) => s.composerPrefill);
  const setPrefill = useSessionStore((s) => s.setComposerPrefill);
  const resendFrom = useSessionStore((s) => s.resendFrom);
  const [editTimelineIndex, setEditTimelineIndex] = useState<number | null>(null);
  const subscriptionAccess = useSubscriptionWorkflowGate(sessionId);

  useEffect(() => {
    setCaret(0);
    setPendingPastes([]);
    setSelectedSkills([]);
    setEditTimelineIndex(null);
    setProjFiles([]);
    setCustomCommands([]);
  }, [draftConversationId, draftOwner]);
  const start = useSessionStore((s) => s.startSession);
  const connecting = useSessionStore((s) => s.connecting);
  const startBlocked = useSessionStore((s) => (s.session ? null : s.startBlockedReason()));
  const startError = useSessionStore((s) => (s.session ? null : s.error));
  const { dragging, handlers } = useFileDrop((files) => void addFiles(files));
  usePaste((files) => void addFiles(files), !connecting);

  useComposerAutosize(taRef, value);

  // Mirror the draft into a non-reactive ref so store actions can read the
  // unsent text without making the textarea value reactive (typing must not
  // re-render the store). `endSession` stages this as a prefill to carry a
  // half-typed message across the composer remount a new session forces.
  useEffect(() => {
    composerDraftRef.current = value;
  }, [value]);

  // "Edit & resend" staged text from a sent message: load it and focus.
  useEffect(() => {
    if (prefill === null) return;
    setDraftValue(prefill.text);
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
  }, [prefill, setDraftValue, setPrefill, skillCatalog]);

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
  const sandboxCheckRequired = sandboxGateRequired({
    localTarget,
    remoteTarget: remote !== null,
    fullAccess: approvalPolicy === "full",
    cwd,
    nativeHost: inTauri(),
    statusSupported: bridge?.localSandboxStatus !== undefined,
  });
  const sandboxBlocked = sandboxBlocksSubmission(sandboxCheckRequired, sandboxStatus);
  const canSend = submission.canSubmit && !sandboxBlocked;

  const trigger = useMemo(() => detectComposerTrigger(value, caret), [value, caret]);

  useEffect(() => {
    if (trigger?.type !== "@" || projFiles.length > 0) return;
    let cancelled = false;
    void projectFiles(cwd, remote).then((files) => {
      if (!cancelled) setProjFiles(files);
    });
    return () => {
      cancelled = true;
    };
  }, [trigger, cwd, projFiles.length, remote]);

  useEffect(() => {
    if (!cwd.trim()) {
      setCustomCommands([]);
      return;
    }
    let cancelled = false;
    void listCustomCommands(cwd, remote ?? undefined).then((cmds) => {
      if (cancelled) return;
      setCustomCommands(
        cmds.map((c) => ({ name: c.name, hint: c.description || "Custom command", body: c.body })),
      );
    });
    return () => {
      cancelled = true;
    };
  }, [cwd, remote]);

  const suggestions = useMemo<ComposerSuggestion[]>(() => {
    if (!trigger || dismissed) return [];
    if (trigger.type === "@") {
      return fuzzyFilterProjectPaths(projFiles, trigger.query, 8);
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
      setDraftValue(next);
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
      if (s.cmd.body !== undefined) {
        const before = value.slice(0, trigger.start);
        const after = value.slice(caret).trimStart();
        const insert = after ? `${s.cmd.body} ${after}` : s.cmd.body;
        const next = before + insert;
        setDraftValue(next);
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
      setDraftValue("");
      s.cmd.run?.();
      return;
    }
    const insert = `@${s.path}${s.kind === "directory" ? "/" : ""} `;
    const before = value.slice(0, trigger.start);
    const after = value.slice(caret);
    const next = before + insert + after;
    const pos = (before + insert).length;
    setDraftValue(next);
    requestAnimationFrame(() => {
      const ta = taRef.current;
      if (ta) {
        ta.focus();
        ta.setSelectionRange(pos, pos);
        setCaret(pos);
      }
    });
  };

  const submit = async ({ subscriptionApproved = false } = {}) => {
    if (!canSend) return;
    const expandedPastes = expandPendingPastes(value, pendingPastes);
    const specialistIntent = specialistSlashIntent(expandedPastes);
    if (specialistIntent) {
      const targetDraftId = `specialist:${specialistIntent.kind}:new`;
      const existing = loadComposerDraft(draftOwner, targetDraftId).trim();
      const nextDraft = [existing, specialistIntent.prompt].filter(Boolean).join("\n");
      moveComposerDraft(draftOwner, draftConversationId, targetDraftId, nextDraft);
      void draft.cloud.clear(draftConversationId);
      if (useSpecialistStore.getState().active !== specialistIntent.kind) {
        useSessionStore.getState().endSession();
      }
      useSpecialistStore.getState().open(specialistIntent.kind, { workflow: specialistIntent.workflow });
      useSpecialistStore.getState().setTab(specialistIntent.tab);
      draftValueRef.current = nextDraft;
      draft.setVisibleValue(nextDraft);
      setPendingPastes([]);
      setSelectedSkills([]);
      return;
    }
    const t = expandPromptSlashCommand(expandedPastes);
    if (subscriptionAccess.shouldGate(
      t,
      selectedSkills.flatMap((skill) => [skill.name, skill.invocationName]),
      subscriptionApproved,
    )) {
      return;
    }
    if (submission.shouldPickProjectFolder) {
      await pickProjectFolder();
      if (useSessionStore.getState().startBlockedReason()) return;
      const current = useSessionStore.getState();
      const selectedCwd = current.activeProjectRoot ?? current.localSettings.cwd;
      const gateSelectedFolder = sandboxGateRequired({
        localTarget,
        remoteTarget: remote !== null,
        fullAccess: approvalPolicy === "full",
        cwd: selectedCwd,
        nativeHost: inTauri(),
        statusSupported: bridge?.localSandboxStatus !== undefined,
      });
      if (gateSelectedFolder) {
        let selectedStatus: LocalSandboxStatus | null = null;
        try {
          const selectedBridge = bridge ?? await getBridge();
          selectedStatus = await selectedBridge.localSandboxStatus?.(selectedCwd) ?? null;
        } catch {
          // Fail closed. The inline card performs its own query and renders the
          // actionable setup or repair error after the selected cwd propagates.
        }
        setSandboxObservation({ cwd: selectedCwd, status: selectedStatus });
        if (sandboxBlocksSubmission(true, selectedStatus)) return;
      }
    }
    if (/^\s*\/skills\s*$/.test(t)) {
      setDraftValue("");
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
      setDraftValue("");
      setPendingPastes([]);
      setEditTimelineIndex(null);
      await compactConversation();
      return;
    }
    const goalObjective = goalCommandObjective(t);
    if (goalObjective === "") {
      setDraftValue("/goal ");
      flashNotice("Describe what Clark should keep working toward after /goal.");
      requestAnimationFrame(() => {
        taRef.current?.focus();
        taRef.current?.setSelectionRange(6, 6);
        setCaret(6);
      });
      return;
    }
    const sideQuestion = sideQuestionCommandQuestion(t);
    if (sideQuestion !== null) {
      if (sideQuestion === "") {
        setDraftValue("/btw ");
        flashNotice("Ask a question after /btw.");
        requestAnimationFrame(() => {
          taRef.current?.focus();
          taRef.current?.setSelectionRange(5, 5);
          setCaret(5);
        });
        return;
      }
      if (!session) {
        flashNotice("Start a conversation before asking a side question.");
        return;
      }
      setDraftValue("");
      setPendingPastes([]);
      setEditTimelineIndex(null);
      setSelectedSkills([]);
      await askSideQuestion(sideQuestion);
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
    const selectedSkillReferences: SkillReferenceBlock[] = selectedSkills.map((skill) => ({
      type: "skill_reference",
      id: skill.id,
      revision: skill.revision,
      name: skill.invocationName,
    }));
    const specialist = useSpecialistStore.getState();
    const workflow = specialist.active ? specialist.contexts[specialist.active]?.workflow : undefined;
    const skillReferences = withActiveSpecialistSkill(
      selectedSkillReferences, skillCatalog?.skills ?? [], specialist.active, workflow,
    );
    if (!specialistWorkflowAvailable(skillReferences, specialist.active, workflow)) {
      flashNotice("The selected specialist workflow is unavailable. Reload skills and try again.");
      return;
    }
    const submittedDraftText = value;
    const acceptCurrentDraft = () => {
      if (!draft.acceptSubmitted(submittedDraftText)) return false;
      setPendingPastes([]);
      setEditTimelineIndex(null);
      setSelectedSkills([]);
      return true;
    };
    const startedNewSession = !session;
    if (startedNewSession) {
      // Starting a session replaces the start-screen Composer with a new
      // instance. Do not move an accepted first prompt into that instance's
      // persisted draft: the new Composer would hydrate it (including from
      // cloud draft sync) after this submit handler's old instance unmounts.
      // Clear the start-screen draft first; if session creation stops or
      // fails, the prefill below restores the user's text.
      draft.acceptSubmitted(submittedDraftText);
      await start();
      const startedSession = useSessionStore.getState().session;
      if (!startedSession) {
        useSessionStore.getState().setComposerPrefill(t);
        return;
      }
    }
    if (editIndex !== null) {
      const receipt = await resendFrom(editIndex, t.trim(), skillReferences);
      if (receipt !== null && !startedNewSession) acceptCurrentDraft();
      return;
    }
    const outcome = await send(t.trim(), skillReferences);
    if (outcome.kind === "not_sent") {
      if (startedNewSession) useSessionStore.getState().setComposerPrefill(t);
      return;
    }
    if (!startedNewSession) acceptCurrentDraft();
  };

  const goalIntent = goalCommandObjective(value);

  const editQueued = (q: QueuedMessage) => {
    setDraftValue((v) => (v.trim() ? `${v}\n${q.text}` : q.text));
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
    if (e.key === "Escape" && busy && !value.trim()) {
      e.preventDefault();
      void cancelActive();
    }
  };

  return (
    <div className="min-w-0 bg-bg px-3 pb-4 pt-2.5 sm:px-6" {...handlers}>
      <ComposerQueuedMessages onEdit={editQueued} />
      {sandboxCheckRequired && (
        <SandboxSetupCard
          compact
          cwd={cwd}
          onStatusChange={updateSandboxStatus}
        />
      )}
      <ComposerSubscriptionWorkflowGate
        access={subscriptionAccess}
        onRun={() => submit({ subscriptionApproved: true })}
        onDismissed={() => taRef.current?.focus()}
      />
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
          "composer-column-width relative z-10 mx-auto w-full border-t px-2.5 py-[1.375rem] transition duration-200 ease-clark",
          "border-border bg-bg-secondary/45 shadow-none",
          dragging
            ? "border-accent bg-accent-subtle"
            : "focus-within:border-accent/50",
        )}
      >
        {dragging && (
          <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center bg-bg-elevated/90 text-sm font-medium text-accent">
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
                setDraftValue("");
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
            setDraftValue(e.target.value);
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

        <div className="mt-0.5 flex min-w-0 items-center gap-2">
          <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
            <ComposerAttachmentMenu
              disabled={connecting}
              onFiles={(files) => void addFiles(files)}
            />
            <ComposerPermissionPill />
            <ComposerCollaborationPill />
          </div>

          <div className="flex shrink-0 items-center gap-2.5">
            {specialistSession
              ? <span
                title={`${activeSpecialist ?? "Specialist"} specialist`}
                aria-label={`${activeSpecialist ?? "Specialist"} specialist`}
                className="hidden shrink-0 rounded-lg bg-accent-soft px-2.5 py-1.5 text-xs font-medium capitalize text-accent sm:inline-flex"
              >
                {activeSpecialist ?? "Specialist"}
              </span>
              : <ModelPill />}
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
          {startError ? humanizeError(startError) : startBlocked}
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
