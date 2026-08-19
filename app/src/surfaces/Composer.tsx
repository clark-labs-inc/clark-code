import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ClipboardEvent, KeyboardEvent } from "react";
import { AnimatePresence } from "motion/react";
import { X, Pencil, Target, Sparkles } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import type { QueuedMessage, SkillReferenceBlock } from "../store/sessionStore";
import { effectiveApprovalPolicy, openRemote } from "../store/sessionStore.runtime";
import {
  getBridge,
  type LocalSandboxStatus,
  type ProjectDirectory,
  type SkillCatalogEntry,
} from "../core-bridge/bridge";
import { useFileDrop, usePaste } from "../lib/attachmentSources";
import { projectFiles } from "../lib/projectFiles";
import {
  parentDirectoryReadRoots,
  parentDirectorySuggestions,
} from "../lib/parentDirectoryAutocomplete";
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
import { fuzzyFilter } from "../lib/fuzzy";
import { cn } from "../lib/cn";
import { humanizeError } from "../lib/errors";
import { inTauri, pickFolder } from "../lib/pickFolder";
import { codeKeyAccountBinding } from "../lib/account";
import { loadSshHosts } from "../lib/sshHosts";
import { useComposerAutosize } from "../lib/composerAutosize";
import {
  composerDraftRef,
  composerDraftOwner,
  loadComposerDraft,
  moveComposerDraft,
  saveComposerDraft,
  specialistStartComposerDraftId,
} from "../lib/composerDraft";
import { useComposerDraftState } from "../lib/useComposerDraftState";
import { useSkillCatalog } from "../lib/useSkillCatalog";
import { useGatedWorkflowGate } from "../lib/useGatedWorkflowGate";
import { AttachmentChips } from "./ComposerAttachments";
import { ComposerContextBar } from "./ComposerContextBar";
import { ComposerAttachmentMenu } from "./ComposerAttachmentMenu";
import { ComposerVoiceButton } from "./ComposerVoiceButton";
import { ComposerAutocomplete } from "./ComposerAutocomplete";
import { ComposerParentFolderDialog } from "./ComposerParentFolderDialog";
import { ComposerPermissionPill } from "./ComposerPermissionPill";
import { ComposerCollaborationPill } from "./ComposerCollaborationPill";
import { ComposerQueuedMessages } from "./ComposerQueuedMessages";
import { ComposerGatedWorkflowGate } from "./ComposerGatedWorkflowGate";
import { ModelPill, QuickChatModelLabel } from "./ComposerControls";
import { ComposerSendAction } from "./ComposerSendAction";
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
import { mergeVoiceTranscriptDraft, type VoiceDraftSession } from "../lib/voiceNarration";
import {
  isSpecComposerSession,
  specialistSlashIntent,
  specialistWorkflowAvailable,
  withActiveSpecialistSkill,
} from "../lib/specialists";
import { useSpecialistStore } from "../store/specialistStore";
import { productModule, productName } from "../product/productModule";
import { composerBrandingCopy } from "./composerBranding";
import { SpecComposerCodeContext, useSpecComposerCodeContext } from "./SpecComposerCodeContext";
import { recordSpecPrompt } from "../lib/specPromptHistory";
import { initialSpecDocument, preparedSpecDocumentPrompt } from "../lib/specDocuments";
import { approvalPolicyForSpecialist } from "../lib/permissions";
import { specialistModelSettings } from "../lib/specialistModel";
import { isQuickChatProject } from "../lib/projectSidebar";

export function Composer() {
  const sessionId = useSessionStore((state) => state.session?.id ?? null);
  const auth = useSessionStore((state) => state.auth);
  const activeSpecialist = useSpecialistStore((state) => state.active);
  const owner = composerDraftOwner(auth?.user ?? null);
  const conversationId = sessionId
    ?? (activeSpecialist ? specialistStartComposerDraftId(activeSpecialist) : null);

  // Draft state owns asynchronous local/cloud hydration. A React component
  // whose draft key changes can otherwise render the previous scope's value
  // once before its effects hydrate the new key, allowing cloud sync to save
  // that stale value under the new conversation. Remount at the ownership
  // boundary so state and in-flight synchronization never cross draft keys.
  return <ScopedComposer key={`${owner}\u0000${conversationId ?? "new"}`} />;
}

function ScopedComposer() {
  const branding = composerBrandingCopy(productName());
  const session = useSessionStore((s) => s.session);
  const auth = useSessionStore((s) => s.auth);
  const draftOwner = composerDraftOwner(auth?.user ?? null);
  const sessionId = session?.id ?? null;
  const activeSpecialist = useSpecialistStore((state) => state.active);
  const specialistContext = useSpecialistStore((state) =>
    state.active ? state.contexts[state.active] : undefined,
  );
  const draftConversationId = sessionId
    ?? (activeSpecialist ? specialistStartComposerDraftId(activeSpecialist) : null);
  const draft = useComposerDraftState(draftOwner, draftConversationId);
  const { value } = draft;
  const draftValueRef = draft.valueRef;
  const setDraftValue = draft.setValue;
  const [caret, setCaret] = useState(0);
  const [projFiles, setProjFiles] = useState<string[]>([]);
  const [parentDirectories, setParentDirectories] = useState<ProjectDirectory[]>([]);
  const [parentReferences, setParentReferences] = useState<
    { path: string; root: string }[]
  >([]);
  const [parentFolderRequest, setParentFolderRequest] = useState<{
    suggestedBase: string;
    remoteHost: string | null;
    before: string;
    after: string;
  } | null>(null);
  const [customCommands, setCustomCommands] = useState<SlashCommand[]>([]);
  const [selectedSkills, setSelectedSkills] = useState<SkillCatalogEntry[]>([]);
  const [skillsOpen, setSkillsOpen] = useState(false);
  const [sel, setSel] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const [pendingPastes, setPendingPastes] = useState<PendingPaste[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const voiceDraftSessionRef = useRef<VoiceDraftSession | null>(null);
  const bridge = useSessionStore((s) => s.bridge);
  const activeProvider = useSessionStore((s) => s.activeProvider);
  const approvalPolicy = useSessionStore((s) => approvalPolicyForSpecialist(
    effectiveApprovalPolicy(s.approvalPolicy, s.approvalPolicies, s.session?.id),
    activeSpecialist,
  ));
  const projectMode = useSessionStore((s) => s.projectMode);
  const localSettings = useSessionStore((s) => s.localSettings);
  const selectedSpecialistSettings = specialistModelSettings(specialistContext);
  const executionSettings = selectedSpecialistSettings
    ? { ...localSettings, ...selectedSpecialistSettings }
    : localSettings;
  const localCwd = localSettings.cwd;
  const selectedHostId = useSessionStore((s) => s.selectedHostId);
  const send = useSessionStore((s) => s.send);
  const compactConversation = useSessionStore((s) => s.compactConversation);
  const pickProjectFolder = useSessionStore((s) => s.pickProjectFolder);
  const flashNotice = useSessionStore((s) => s.flashNotice);
  const removeQueued = useSessionStore((s) => s.removeQueued);
  const cancelActive = useSessionStore((s) => s.cancelActive);
  const askSideQuestion = useSessionStore((s) => s.askSideQuestion);
  const activeProjectRoot = useSessionStore((s) => s.activeProjectRoot);
  const activeRemote = useSessionStore((s) => s.activeRemote);
  const activeRemoteHost = useSessionStore((s) => s.activeRemoteHost);
  const selectedRemoteHost = loadSshHosts(codeKeyAccountBinding(auth))
    .find((host) => host.id === selectedHostId) ?? null;
  const isRemoteSelection = !session
    && activeProvider === "local"
    && projectMode === "remote";
  const cwd = session
    ? activeProjectRoot?.trim() || localSettings.cwd.trim()
    : isRemoteSelection
      ? selectedRemoteHost?.remoteRoot.trim() ?? ""
      : localSettings.cwd.trim();
  const [inspectionRemote, setInspectionRemote] = useState<typeof activeRemote>(null);

  useEffect(() => {
    let current = true;
    setInspectionRemote(null);
    if (!isRemoteSelection || !selectedRemoteHost || !cwd) {
      return () => { current = false; };
    }
    void openRemote(selectedRemoteHost, executionSettings, cwd).then((next) => {
      if (current) setInspectionRemote(next);
    }).catch(() => {
      // The normal start/connect surface owns connection errors. Autocomplete
      // remains usable through the explicit absolute-path entry fallback.
    });
    return () => { current = false; };
  }, [
    cwd,
    isRemoteSelection,
    executionSettings.model,
    executionSettings.reasoningEffort,
    selectedRemoteHost?.host,
    selectedRemoteHost?.id,
  ]);
  const remote = useMemo(
    () => {
      const target = session ? activeRemote : inspectionRemote;
      return target ? { id: target.id } : null;
    },
    [activeRemote, inspectionRemote, session],
  );
  const isRemoteContext = Boolean(activeRemoteHost) || isRemoteSelection;
  const remoteHostLabel = activeRemoteHost
    ?? selectedRemoteHost?.host.trim()
    ?? "SSH host";
  const projectInspectionReady = !isRemoteContext || Boolean(remote);
  const localTarget = session ? session.provider === "local" : activeProvider === "local";
  const specialistSession = Boolean(activeSpecialist) || session?.provider === "specialist";
  const quickChatSession = Boolean(
    session && isQuickChatProject(activeProjectRoot ?? undefined, session.id),
  );
  const specSession = isSpecComposerSession(activeSpecialist);
  const usesConversationWorkspace = Boolean(
    activeSpecialist
    && productModule().specialistWorkspace?.isConversationBound(activeSpecialist),
  );
  const startsScoutRun = activeSpecialist === "scout" && !session;
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
  } = useSkillCatalog(bridge, cwd, remote, localTarget || specSession);
  const busy = useSessionStore((s) =>
    Object.values(s.snapshot.runs).some((r) => r.status === "running" || r.status === "queued"),
  );
  const attachments = useSessionStore((s) => s.attachments);
  const addFiles = useSessionStore((s) => s.addFiles);
  const prefill = useSessionStore((s) => s.composerPrefill);
  const setPrefill = useSessionStore((s) => s.setComposerPrefill);
  const resendFrom = useSessionStore((s) => s.resendFrom);
  const [editTimelineIndex, setEditTimelineIndex] = useState<number | null>(null);
  const workflowAccess = useGatedWorkflowGate(sessionId);

  useEffect(() => {
    setCaret(0);
    setPendingPastes([]);
    setSelectedSkills([]);
    setEditTimelineIndex(null);
    setProjFiles([]);
    setParentDirectories([]);
    setParentReferences([]);
    setParentFolderRequest(null);
    setCustomCommands([]);
  }, [draftConversationId, draftOwner]);
  useEffect(() => setProjFiles([]), [cwd, remote?.id]);
  const start = useSessionStore((s) => s.startSession);
  const connecting = useSessionStore((s) => s.connecting);
  const startBlocked = useSessionStore((s) => (s.session ? null : s.startBlockedReason()));
  const startError = useSessionStore((s) => (s.session ? null : s.error));
  const { dragging, dropTargetRef } = useFileDrop((files) => {
    if (!connecting && !submitting) void addFiles(files);
  });
  usePaste((files) => void addFiles(files), !connecting && !submitting);

  useComposerAutosize(taRef, value);

  // Mirror the exact scoped draft into a non-reactive ref so store actions can
  // read it without making the textarea value reactive (typing must not
  // re-render the store). It must never be copied into another draft scope.
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
              origin: "bundled",
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

  const trigger = useMemo(() => detectComposerTrigger(value, caret), [value, caret]);
  const specCodeContext = useSpecComposerCodeContext({
    enabled: specSession,
    draftKey: draftConversationId,
    trigger,
    value,
    caret,
    textareaRef: taRef,
    setValue: setDraftValue,
    setCaret,
  });
  const hasContent = value.trim().length > 0
    || attachments.length > 0
    || pendingPastes.length > 0
    || selectedSkills.length > 0
    || specCodeContext.references.length > 0;
  const submission = composerSubmissionState({
    hasContent,
    hasSession: !!session,
    connecting,
    activeProvider,
    projectMode,
    localCwd,
    startBlocked,
    canPickProjectFolder: inTauri(),
    usesConversationWorkspace,
  });
  const visibleStartBlocked = usesConversationWorkspace && projectMode === "local"
    ? null
    : startBlocked;
  const sandboxCheckRequired = !usesConversationWorkspace && sandboxGateRequired({
    localTarget,
    remoteTarget: isRemoteContext,
    fullAccess: approvalPolicy === "full",
    cwd,
    nativeHost: inTauri(),
    statusSupported: bridge?.localSandboxStatus !== undefined,
  });
  const sandboxBlocked = sandboxBlocksSubmission(sandboxCheckRequired, sandboxStatus);
  const canSend = submission.canSubmit && !sandboxBlocked && !submitting;

  const mentionProjectRoot = specSession ? specCodeContext.repositoryRoot : cwd;

  useEffect(() => setProjFiles([]), [mentionProjectRoot, remote?.id]);
  useEffect(() => setParentDirectories([]), [cwd, remote?.id]);

  useEffect(() => {
    if (trigger?.type !== "@" || projFiles.length > 0 || !projectInspectionReady) return;
    let cancelled = false;
    void projectFiles(mentionProjectRoot, remote).then((files) => {
      if (!cancelled) setProjFiles(files);
    });
    return () => {
      cancelled = true;
    };
  }, [trigger, mentionProjectRoot, projFiles.length, projectInspectionReady, remote]);

  useEffect(() => {
    if (trigger?.type !== "@" || !cwd.trim() || !projectInspectionReady) return;
    let cancelled = false;
    void (bridge?.listSiblingDirectories?.(cwd, remote) ?? Promise.resolve([])).then((directories) => {
      if (!cancelled) setParentDirectories(directories);
    });
    return () => {
      cancelled = true;
    };
  }, [bridge, cwd, projectInspectionReady, remote, trigger?.type]);

  useEffect(() => {
    if (!cwd.trim() || !projectInspectionReady) {
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
  }, [cwd, projectInspectionReady, remote]);

  const suggestions = useMemo<ComposerSuggestion[]>(() => {
    if (!trigger || dismissed) return [];
    if (trigger.type === "@") {
      const parentSuggestions = specialistSession
        ? []
        : parentDirectorySuggestions(trigger.query, parentDirectories);
      if (trigger.query.startsWith("..")) return parentSuggestions;
      if (specSession) return specCodeContext.suggestions(trigger.query, projFiles);
      return [
        ...parentSuggestions,
        ...specCodeContext.suggestions(trigger.query, projFiles),
      ].slice(0, 8);
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
    parentDirectories,
    session,
    activeProvider,
    customCommands,
    skillCatalog,
    specCodeContext,
    specSession,
    specialistSession,
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

  const insertParentFolder = (
    picked: string,
    insertion: { before: string; after: string },
  ) => {
    setParentReferences((current) => current.some(({ root }) => root === picked)
      ? current
      : [...current, { path: picked, root: picked }]);
    const insert = `@${picked.replace(/[\\/]+$/, "")}/ `;
    const next = insertion.before + insert + insertion.after;
    const pos = (insertion.before + insert).length;
    setDraftValue(next);
    requestAnimationFrame(() => {
      taRef.current?.focus();
      taRef.current?.setSelectionRange(pos, pos);
      setCaret(pos);
    });
  };

  const accept = (s: ComposerSuggestion) => {
    if (!trigger) return;
    if (specCodeContext.acceptSuggestion(s)) return;
    if (s.kind === "parent_directory_menu") {
      const before = value.slice(0, trigger.start);
      const after = value.slice(caret);
      const next = `${before}@../${after}`;
      const pos = before.length + 4;
      setDraftValue(next);
      requestAnimationFrame(() => {
        taRef.current?.focus();
        taRef.current?.setSelectionRange(pos, pos);
        setCaret(pos);
      });
      return;
    }
    if (s.kind === "parent_directory_picker") {
      const parent = cwd.replace(/[\\/][^\\/]+[\\/]?$/, "");
      const insertion = {
        before: value.slice(0, trigger.start),
        after: value.slice(caret),
      };
      if (isRemoteContext || !inTauri()) {
        setParentFolderRequest({
          suggestedBase: parent || cwd,
          remoteHost: isRemoteContext ? remoteHostLabel : null,
          ...insertion,
        });
        return;
      }
      void pickFolder(parent || cwd)
        .then((picked) => {
          if (picked) insertParentFolder(picked, insertion);
        })
        .catch((error) => {
          flashNotice(`Could not open the folder picker: ${humanizeError(String(error))}`);
        });
      return;
    }
    if (s.kind === "parent_directory") {
      setParentReferences((current) => current.some(({ root }) => root === s.root)
        ? current
        : [...current, { path: s.path, root: s.root }]);
    }
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
    if (
      s.kind === "spec_repository"
      || s.kind === "spec_repository_picker"
      || s.kind === "spec_folder"
    ) return;
    const insert = `@${s.path}${
      s.kind === "directory" || s.kind === "parent_directory" ? "/" : ""
    } `;
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

  const submit = async ({ workflowAccessApproved = false } = {}) => {
    if (!canSend) return;
    const expandedPastes = expandPendingPastes(value, pendingPastes);
    const specialistIntent = specialistSlashIntent(expandedPastes);
    if (specialistIntent) {
      const targetDraftId = specialistStartComposerDraftId(specialistIntent.kind);
      const existing = loadComposerDraft(draftOwner, targetDraftId).trim();
      const nextDraft = [existing, specialistIntent.prompt].filter(Boolean).join("\n");
      moveComposerDraft(draftOwner, draftConversationId, targetDraftId, nextDraft);
      try {
        const result = await draft.cloud.acceptSubmitted(draftConversationId, value);
        if (result?.outcome === "preserved_newer") {
          flashNotice("A newer cloud draft was preserved in the previous composer.");
        }
      } catch {
        flashNotice("The specialist request moved, but its previous cloud draft still needs to sync.");
      }
      if (useSpecialistStore.getState().active !== specialistIntent.kind) {
        useSessionStore.getState().endSession();
      }
      useSpecialistStore.getState().open(specialistIntent.kind, { workflow: specialistIntent.workflow });
      useSpecialistStore.getState().setTab(specialistIntent.tab);
      setPendingPastes([]);
      setSelectedSkills([]);
      specCodeContext.reset();
      return;
    }
    const t = expandPromptSlashCommand(expandedPastes);
    if (workflowAccess.shouldGate(
      t,
      selectedSkills.flatMap((skill) => [skill.name, skill.invocationName]),
      workflowAccessApproved,
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
      flashNotice(branding.goalHelp);
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
      specCodeContext.reset();
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
    let availableSkills = skillCatalog?.skills ?? [];
    if (
      specialist.active === "spec"
      && !availableSkills.some((skill) => skill.enabled && skill.invocationName === "spec:spec")
      && bridge?.reloadSkills
    ) {
      try {
        const refreshed = await bridge.reloadSkills(cwd, remote);
        setSkillCatalog(refreshed);
        availableSkills = refreshed.skills;
      } catch (error) {
        flashNotice(`Could not load the Spec workflow: ${humanizeError(String(error))}`);
        return;
      }
    }
    const skillReferences = withActiveSpecialistSkill(
      selectedSkillReferences, availableSkills, specialist.active, workflow,
    );
    if (!specialistWorkflowAvailable(skillReferences, specialist.active, workflow)) {
      flashNotice("The selected specialist workflow is unavailable. Reload skills and try again.");
      return;
    }
    const composerReadRoots = parentDirectoryReadRoots(
      t,
      parentReferences,
      parentDirectories,
    );
    if (session && composerReadRoots.length > 0) {
      if (!bridge?.addReadRoots) {
        flashNotice("This Clark Code build cannot attach parent folders to a live chat.");
        return;
      }
      try {
        await bridge.addReadRoots(session.id, composerReadRoots);
      } catch (error) {
        flashNotice(`Could not attach that parent folder: ${humanizeError(String(error))}`);
        return;
      }
    }
    const submittedDraftText = value;
    const submittedPastes = pendingPastes;
    const submittedSkills = selectedSkills;
    const submittedParentReferences = parentReferences;
    const submittedSpecReferences = specCodeContext.references;
    const acceptCurrentDraft = () => {
      if (!draft.acceptSubmitted(submittedDraftText)) return false;
      setPendingPastes([]);
      setEditTimelineIndex(null);
      setSelectedSkills([]);
      setParentReferences([]);
      specCodeContext.reset();
      return true;
    };
    const restoreCurrentDraft = () => {
      setPrefill(null);
      setDraftValue(submittedDraftText);
      setPendingPastes(submittedPastes);
      setEditTimelineIndex(editIndex);
      setSelectedSkills(submittedSkills);
      setParentReferences(submittedParentReferences);
      specCodeContext.replaceReferences(submittedSpecReferences);
    };
    const startedNewSession = !session;
    let composerReleased = false;
    const releaseComposer = () => {
      if (startedNewSession || composerReleased) return;
      composerReleased = true;
      setSubmitting(false);
      requestAnimationFrame(() => taRef.current?.focus());
    };
    const settleAcceptedDraft = async (): Promise<"preserved_newer" | "failed" | null> => {
      // Clear locally again after provider acceptance, settle any serialized
      // writer, and remove only this submitted text from cloud persistence. A
      // newer cross-device edit is preserved; a repeatedly rejected current
      // revision is visible to the user instead of spinning in a 409 loop.
      saveComposerDraft(draftOwner, draftConversationId, "");
      try {
        const result = await draft.cloud.acceptSubmitted(
          draftConversationId,
          submittedDraftText,
        );
        return result?.outcome === "preserved_newer" ? "preserved_newer" : null;
      } catch {
        return "failed";
      }
    };
    const notifyDraftSettle = (result: Awaited<ReturnType<typeof settleAcceptedDraft>>) => {
      if (result === "preserved_newer") {
        flashNotice("Message sent. A newer cloud draft from another device was preserved.");
      } else if (result === "failed") {
        flashNotice("Message sent, but its cloud draft could not be cleared. Your local draft state is preserved.");
      }
    };
    let startDraftSettled = false;
    let startDraftSettleResult: Awaited<ReturnType<typeof settleAcceptedDraft>> = null;
    if (startedNewSession) {
      // Starting a session replaces the start-screen Composer with a new
      // instance. Do not move an accepted first prompt into that instance's
      // persisted draft: the new Composer would hydrate it (including from
      // cloud draft sync) after this submit handler's old instance unmounts.
      // Clear the start-screen draft first; if session creation stops or
      // fails, the prefill below restores the user's text. Await both the
      // serialized writer and the residue check before `start()` unmounts this
      // Composer. A long native input arrives in chunks, and letting the
      // component unmount with a prefix PUT still in flight can leave that
      // accepted prefix in the cloud to rehydrate the next start screen.
      draft.acceptSubmitted(submittedDraftText);
      // `startSession` can carry a non-reactive draft across the Composer
      // remount. This prompt is already being submitted, so clear that mirror
      // before opening the session or it would reappear in the new composer.
      composerDraftRef.current = "";
      startDraftSettleResult = await settleAcceptedDraft();
      startDraftSettled = true;
      await start({ submittedDraft: t, readRoots: composerReadRoots });
      const startedSession = useSessionStore.getState().session;
      if (!startedSession) {
        useSessionStore.getState().setComposerPrefill(t);
        return;
      }
    }
    let prompt = specCodeContext.prompt(t);
    if (specSession) {
      const liveConversationId = useSessionStore.getState().session?.id;
      if (liveConversationId) {
        try {
          const prepared = await productModule().specialistWorkspace?.prepareDocument?.(
            "spec",
            liveConversationId,
            initialSpecDocument(t),
          );
          if (prepared) prompt = preparedSpecDocumentPrompt(prompt, prepared.filename);
        } catch (error) {
          flashNotice(`Could not load the saved spec: ${humanizeError(String(error))}`);
          if (startedNewSession) useSessionStore.getState().setComposerPrefill(t);
          return;
        }
      }
    }
    if (!startedNewSession) {
      if (!acceptCurrentDraft()) {
        flashNotice("The draft changed before it could be sent. Review it and try again.");
        return;
      }
      composerDraftRef.current = "";
      setSubmitting(true);
    }
    try {
      if (editIndex !== null) {
        const receipt = await resendFrom(editIndex, prompt, skillReferences);
        if (receipt === null) {
          if (!startedNewSession) restoreCurrentDraft();
          return;
        }
        if (specSession) {
          recordSpecPrompt(
            draftOwner,
            useSessionStore.getState().session?.id ?? null,
            t,
          );
        }
        const settling = startDraftSettled
          ? Promise.resolve(startDraftSettleResult)
          : settleAcceptedDraft();
        releaseComposer();
        const result = await settling;
        notifyDraftSettle(result);
        return;
      }
      const outcome = await send(prompt, skillReferences);
      if (outcome.kind === "not_sent") {
        if (startedNewSession) useSessionStore.getState().setComposerPrefill(t);
        else restoreCurrentDraft();
        return;
      }
      if (specSession) {
        recordSpecPrompt(
          draftOwner,
          useSessionStore.getState().session?.id ?? null,
          t,
        );
      }
      const settling = startDraftSettled
        ? Promise.resolve(startDraftSettleResult)
        : settleAcceptedDraft();
      releaseComposer();
      const result = await settling;
      notifyDraftSettle(result);
    } finally {
      releaseComposer();
    }
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
      if (busy && editTimelineIndex !== null) {
        flashNotice("Stopping Clark — send the edit when the current run has stopped.");
        void cancelActive();
        return;
      }
      void submit();
    }
    if (e.key === "Escape" && busy && !value.trim()) {
      e.preventDefault();
      void cancelActive();
    }
  };

  return (
    <div
      ref={dropTargetRef}
      data-file-drop-target="composer"
      className={cn(
        "min-w-0 bg-bg px-3 pb-4 pt-2.5 sm:px-6",
        specialistSession && "specialist-composer",
      )}
    >
      <ComposerQueuedMessages onEdit={editQueued} />
      <ComposerParentFolderDialog
        open={parentFolderRequest !== null}
        suggestedBase={parentFolderRequest?.suggestedBase ?? ""}
        remoteHost={parentFolderRequest?.remoteHost}
        onCancel={() => {
          setParentFolderRequest(null);
          requestAnimationFrame(() => taRef.current?.focus());
        }}
        onChoose={(path) => {
          const request = parentFolderRequest;
          if (!request) return;
          setParentFolderRequest(null);
          insertParentFolder(path, request);
        }}
      />
      {sandboxCheckRequired && (
        <SandboxSetupCard
          compact
          cwd={cwd}
          onStatusChange={updateSandboxStatus}
        />
      )}
      <ComposerGatedWorkflowGate
        access={workflowAccess}
        onRun={() => submit({ workflowAccessApproved: true })}
        onDismissed={() => taRef.current?.focus()}
      />
      {/* Keep suggestions in normal layout flow. The old absolute menu was
          trapped below the context bar's stacking layer and visibly painted
          through the checkout chips at compact window heights. */}
      <AnimatePresence>
        {suggestions.length > 0 && (
          <div className="conversation-column-width relative z-30 mx-auto mb-2 w-full">
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
          "relative z-10 mx-auto w-full rounded-lg border px-2.5 transition duration-base ease-agent",
          specSession ? "max-w-[70rem] py-3" : "conversation-column-width py-[1.375rem]",
          specialistSession && "specialist-composer-surface",
          "border-border bg-composer-surface shadow-none",
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

        {specSession && <SpecComposerCodeContext controller={specCodeContext} />}

        {goalIntent !== null && (
          <div className="flex items-center gap-1.5 pb-1 pt-0.5 text-xs font-medium text-accent">
            <Target className="size-3.5" />
            <span>Standing goal</span>
            <span className="font-normal text-ink-faint">{branding.goalStatus}</span>
          </div>
        )}

        {editTimelineIndex !== null && (
          <div className="flex items-center gap-1.5 pb-1 pt-0.5 text-xs text-ink-muted">
            <Pencil className="size-3" />
            <span>
              {busy
                ? "Editing message — stop Clark before replacing this turn"
                : "Editing message — later turns will be replaced"}
            </span>
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
          aria-label={branding.ariaLabel}
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          placeholder={
            specSession
              ? "Describe what should change, ask a question, or dictate an idea…"
              : !session
                ? branding.initialPlaceholder
              : busy
                ? "Queue a follow-up…"
                : branding.projectPlaceholder
          }
          disabled={connecting || submitting}
          aria-busy={submitting || undefined}
          className="composer-input max-h-52 w-full resize-none overflow-y-auto bg-transparent px-0.5 py-0.5 text-base leading-[1.5] text-ink outline-none placeholder:text-ink-muted disabled:opacity-50"
        />

        <div className="mt-0.5 flex min-w-0 items-center gap-2">
          <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
            <ComposerAttachmentMenu
              disabled={connecting || submitting}
              paperclip={Boolean(specSession)}
              onFiles={(files) => void addFiles(files)}
            />
            {specSession && (
              <ComposerVoiceButton
                disabled={connecting || submitting}
                onError={(message) => {
                  voiceDraftSessionRef.current = null;
                  flashNotice(message);
                }}
                onTranscript={(transcript, state) => {
                  const merged = mergeVoiceTranscriptDraft(
                    draftValueRef.current,
                    voiceDraftSessionRef.current,
                    transcript,
                  );
                  voiceDraftSessionRef.current = state === "partial" ? merged.session : null;
                  setDraftValue(merged.value);
                  requestAnimationFrame(() => {
                    const textarea = taRef.current;
                    textarea?.focus();
                    textarea?.setSelectionRange(merged.value.length, merged.value.length);
                  });
                }}
              />
            )}
            {!specSession && <ComposerPermissionPill />}
            {!specSession && activeSpecialist !== "scout" && <ComposerCollaborationPill />}
          </div>

          <div className="flex shrink-0 items-center gap-2.5">
            {specSession
              ? <span
                aria-label="Whole specification scope"
                className="hidden shrink-0 px-1 text-xs font-medium text-ink-faint sm:inline-flex"
              >
                Whole spec
              </span>
              : specialistSession
                ? <span
                title={`${activeSpecialist ?? "Specialist"} specialist`}
                aria-label={`${activeSpecialist ?? "Specialist"} specialist`}
                className="hidden shrink-0 rounded-lg bg-accent-soft px-2.5 py-1.5 text-xs font-medium capitalize text-accent sm:inline-flex"
              >
                {activeSpecialist ?? "Specialist"}
                </span>
                : quickChatSession
                  ? <QuickChatModelLabel />
                  : <ModelPill />}
            <ComposerSendAction
              submitting={submitting}
              busy={busy}
              editing={editTimelineIndex !== null}
              hasContent={hasContent}
              canSend={canSend}
              shouldPickProjectFolder={submission.shouldPickProjectFolder}
              startsScoutRun={startsScoutRun}
              queuedTitle={branding.queuedTitle}
              onCancel={() => void cancelActive()}
              onSubmit={() => void submit()}
            />
          </div>
        </div>
      </div>
      <p className="sr-only" role="status" aria-live="polite" aria-atomic="true">
        {submitting ? "Sending message." : ""}
      </p>
      {/* One quiet status line: a connect failure (in red) wins over the
          "what's missing" readiness hint. Connecting itself never shows here —
          the OpeningScreen owns that state. */}
      {!session && (startError || visibleStartBlocked) && (
        <p
          className={cn(
            "mx-auto mt-2 w-full px-1 text-xs",
            specSession ? "max-w-[70rem]" : "conversation-column-width",
            startError ? "text-danger" : "text-ink-faint",
          )}
        >
          {startError ? humanizeError(startError) : visibleStartBlocked}
        </p>
      )}
      {draft.cloud.status === "conflict" && (
        <p
          role="status"
          className="conversation-column-width mx-auto mt-2 w-full px-1 text-xs text-warning"
        >
                Draft cloud sync is paused. Your text is safe on this device.
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
