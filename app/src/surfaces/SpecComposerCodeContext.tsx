import { useEffect, useState } from "react";
import type { RefObject } from "react";
import { FileCode2, Folder, FolderGit2, X } from "lucide-react";
import type { ComposerSuggestion, ComposerTrigger } from "../lib/composerInput";
import { fuzzyFilterProjectPaths } from "../lib/fuzzy";
import { pickFolder } from "../lib/pickFolder";
import {
  specCodeContextPrompt,
  specRelativePath,
  specRepositoryLabel,
  type SpecCodeReference,
} from "../lib/specDocuments";
import { useSessionStore } from "../store/sessionStore";
import { useSpecialistStore } from "../store/specialistStore";

interface SpecComposerCodeContextInput {
  enabled: boolean;
  draftKey: string | null;
  trigger: ComposerTrigger | null;
  value: string;
  caret: number;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  setValue: (value: string) => void;
  setCaret: (caret: number) => void;
}

export interface SpecComposerCodeContextController {
  references: SpecCodeReference[];
  repositoryPath: string;
  repositoryRoot: string;
  suggestions: (query: string, projectPaths: string[]) => ComposerSuggestion[];
  acceptSuggestion: (suggestion: ComposerSuggestion) => boolean;
  reset: () => void;
  prompt: (message: string) => string;
  chooseRepository: () => Promise<void>;
  removeReference: (reference: SpecCodeReference) => void;
}

export function useSpecComposerCodeContext({
  enabled,
  draftKey,
  trigger,
  value,
  caret,
  textareaRef,
  setValue,
  setCaret,
}: SpecComposerCodeContextInput): SpecComposerCodeContextController {
  const session = useSessionStore((state) => state.session);
  const cwd = useSessionStore((state) => state.activeProjectRoot ?? state.localSettings.cwd);
  const localCwd = useSessionStore((state) => state.localSettings.cwd);
  const setProjectFolder = useSessionStore((state) => state.setProjectFolder);
  const setProjectMode = useSessionStore((state) => state.setProjectMode);
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const context = useSpecialistStore((state) => state.active
    ? state.contexts[state.active]
    : undefined);
  const setSpecialistContext = useSpecialistStore((state) => state.setContext);
  const [references, setReferences] = useState<SpecCodeReference[]>([]);
  const repositoryPath = enabled ? context?.repositoryPath?.trim() ?? "" : "";
  const repositoryRoot = enabled ? (session ? cwd : repositoryPath) : "";

  useEffect(() => setReferences([]), [draftKey]);

  const removeTrigger = () => {
    if (!trigger) return;
    const before = value.slice(0, trigger.start);
    const next = before + value.slice(caret);
    setValue(next);
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
      textareaRef.current?.setSelectionRange(before.length, before.length);
      setCaret(before.length);
    });
  };

  const chooseRepository = async () => {
    if (session) {
      flashNotice("Repository focus is locked for this spec. Start a new spec to use another repository.");
      return;
    }
    const picked = await pickFolder(repositoryPath || localCwd || undefined);
    if (!picked) return;
    setProjectMode("local");
    setProjectFolder(picked);
    setSpecialistContext({ repositoryPath: picked });
    removeTrigger();
  };

  const chooseFolder = async () => {
    if (!repositoryRoot) {
      flashNotice("Add @repo first so Clark can safely read code inside that repository.");
      return;
    }
    const picked = await pickFolder(repositoryRoot);
    if (!picked) return;
    const relative = specRelativePath(repositoryRoot, picked);
    if (relative === null) {
      flashNotice("Choose a folder inside the focused repository.");
      return;
    }
    setReferences((current) => current.some(
      (reference) => reference.kind === "folder" && reference.path === relative,
    ) ? current : [...current, { kind: "folder", path: relative }]);
    removeTrigger();
  };

  const suggestions = (query: string, projectPaths: string[]): ComposerSuggestion[] => {
    if (!enabled) return fuzzyFilterProjectPaths(projectPaths, query, 8);
    const normalized = query.toLowerCase();
    const actions: ComposerSuggestion[] = [
      ...("repo".includes(normalized) ? [{ kind: "spec_repository" as const }] : []),
      ...("folder".includes(normalized) ? [{ kind: "spec_folder" as const }] : []),
    ];
    return [
      ...actions,
      ...fuzzyFilterProjectPaths(repositoryRoot ? projectPaths : [], query, 8 - actions.length),
    ];
  };

  const acceptSuggestion = (suggestion: ComposerSuggestion): boolean => {
    if (!enabled) return false;
    if (suggestion.kind === "spec_repository") {
      void chooseRepository();
      return true;
    }
    if (suggestion.kind === "spec_folder") {
      void chooseFolder();
      return true;
    }
    if (suggestion.kind !== "directory" && suggestion.kind !== "file") return false;
    const reference: SpecCodeReference = {
      kind: suggestion.kind === "directory" ? "folder" : "file",
      path: suggestion.path.replace(/\/$/, ""),
    };
    setReferences((current) => current.some(
      (candidate) => candidate.kind === reference.kind && candidate.path === reference.path,
    ) ? current : [...current, reference]);
    removeTrigger();
    return true;
  };

  return {
    references,
    repositoryPath,
    repositoryRoot,
    suggestions,
    acceptSuggestion,
    reset: () => setReferences([]),
    prompt: (message) => enabled
      ? specCodeContextPrompt(message, repositoryRoot, references)
      : message.trim(),
    chooseRepository,
    removeReference: (reference) => setReferences((current) => current.filter(
      (candidate) => candidate.kind !== reference.kind || candidate.path !== reference.path,
    )),
  };
}

export function SpecComposerCodeContext({
  controller,
}: {
  controller: SpecComposerCodeContextController;
}) {
  if (!controller.repositoryPath && controller.references.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5 pb-1.5" data-qa="spec-code-context">
      {controller.repositoryPath && (
        <button
          type="button"
          onClick={() => void controller.chooseRepository()}
          disabled={Boolean(useSessionStore.getState().session)}
          title={useSessionStore.getState().session
            ? `This spec is grounded in ${controller.repositoryPath}`
            : "Change repository focus"}
          className="flex items-center gap-1.5 rounded-lg bg-accent-subtle px-2 py-1 text-xs font-medium text-accent disabled:cursor-default disabled:opacity-100"
        >
          <FolderGit2 className="size-3.5" />
          @repo {specRepositoryLabel(controller.repositoryPath)}
        </button>
      )}
      {controller.references.map((reference) => (
        <span
          key={`${reference.kind}:${reference.path}`}
          className="flex items-center gap-1.5 rounded-lg bg-bg-secondary px-2 py-1 text-xs text-ink-secondary"
          title={`${reference.kind === "folder" ? "Folder" : "File"} inside ${controller.repositoryRoot}`}
        >
          {reference.kind === "folder"
            ? <Folder className="size-3.5 text-accent" />
            : <FileCode2 className="size-3.5 text-accent" />}
          @{reference.kind} {reference.path}
          <button
            type="button"
            onClick={() => controller.removeReference(reference)}
            aria-label={`Remove ${reference.kind} ${reference.path}`}
          >
            <X className="size-3" />
          </button>
        </span>
      ))}
    </div>
  );
}
