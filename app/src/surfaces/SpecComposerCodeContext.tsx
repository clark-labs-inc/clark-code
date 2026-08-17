import { useEffect, useState } from "react";
import type { RefObject } from "react";
import { FileCode2, Folder, FolderGit2, X } from "lucide-react";
import type { ComposerSuggestion, ComposerTrigger } from "../lib/composerInput";
import { fuzzyFilterProjectPaths } from "../lib/fuzzy";
import { pickFolder } from "../lib/pickFolder";
import { siblingProjectDirectories } from "../lib/projectDirectories";
import { specRepositorySuggestions } from "../lib/specRepositoryAutocomplete";
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
  replaceReferences: (references: SpecCodeReference[]) => void;
  prompt: (message: string) => string;
  chooseRepository: () => Promise<void>;
  removeRepository: () => Promise<void>;
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
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const bridge = useSessionStore((state) => state.bridge);
  const activeRemote = useSessionStore((state) => state.activeRemote);
  const recentProjects = useSessionStore((state) => state.recentProjects);
  const context = useSpecialistStore((state) => state.active
    ? state.contexts[state.active]
    : undefined);
  const setSpecialistContext = useSpecialistStore((state) => state.setContext);
  const [references, setReferences] = useState<SpecCodeReference[]>([]);
  const [repositoryChoices, setRepositoryChoices] = useState<
    { path: string; current: boolean }[]
  >([]);
  const repositoryPath = enabled ? context?.repositoryPath?.trim() ?? "" : "";
  const repositoryRoot = enabled ? repositoryPath : "";

  useEffect(() => {
    if (!enabled) {
      setRepositoryChoices([]);
      return;
    }
    const current = (activeRemote ? cwd : repositoryPath || localCwd).trim();
    if (!current) {
      setRepositoryChoices([]);
      return;
    }
    let cancelled = false;
    void siblingProjectDirectories(
      current,
      activeRemote ? { id: activeRemote.id } : null,
    ).then((siblings) => {
      if (cancelled) return;
      const paths = [current, ...siblings.map((entry) => entry.path), ...recentProjects];
      const seen = new Set<string>();
      setRepositoryChoices(paths.flatMap((path) => {
        const normalized = path.trim().replace(/[\\/]+$/, "");
        if (!normalized || seen.has(normalized)) return [];
        seen.add(normalized);
        return [{ path: normalized, current: normalized === current.replace(/[\\/]+$/, "") }];
      }));
    });
    return () => {
      cancelled = true;
    };
  }, [activeRemote, cwd, enabled, localCwd, recentProjects, repositoryPath]);

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

  const chooseRepository = async (suggestedPath?: string) => {
    const picked = suggestedPath
      ?? await pickFolder(repositoryPath || localCwd || undefined);
    if (!picked) return;
    if (session) {
      if (!bridge?.addReadRoots) {
        flashNotice("This Clark Code build cannot attach a repository to a live spec.");
        return;
      }
      try {
        await bridge.addReadRoots(session.id, [picked]);
      } catch (error) {
        flashNotice(`Could not attach that repository: ${String(error)}`);
        return;
      }
      if (
        repositoryPath
        && repositoryPath !== picked
        && bridge.removeReadRoots
      ) {
        try {
          await bridge.removeReadRoots(session.id, [repositoryPath]);
        } catch (error) {
          flashNotice(`Changed focus, but could not revoke the previous repository: ${String(error)}`);
        }
      }
    }
    setSpecialistContext({ repositoryPath: picked });
    setReferences([]);
    removeTrigger();
  };

  const removeRepository = async () => {
    if (!repositoryPath) return;
    if (session) {
      if (!bridge?.removeReadRoots) {
        flashNotice("This Clark Code build cannot remove repository access from a live spec.");
        return;
      }
      try {
        await bridge.removeReadRoots(session.id, [repositoryPath]);
      } catch (error) {
        flashNotice(`Could not remove repository focus: ${String(error)}`);
        return;
      }
    }
    setSpecialistContext({ repositoryPath: undefined });
    setReferences([]);
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
    const actions = specRepositorySuggestions(query, repositoryChoices, repositoryRoot);
    return [
      ...actions,
      ...fuzzyFilterProjectPaths(repositoryRoot ? projectPaths : [], query, 8 - actions.length),
    ];
  };

  const acceptSuggestion = (suggestion: ComposerSuggestion): boolean => {
    if (!enabled) return false;
    if (suggestion.kind === "spec_repository") {
      void chooseRepository(suggestion.path);
      return true;
    }
    if (suggestion.kind === "spec_repository_picker") {
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
    replaceReferences: setReferences,
    prompt: (message) => enabled
      ? specCodeContextPrompt(message, repositoryRoot, references)
      : message.trim(),
    chooseRepository: () => chooseRepository(),
    removeRepository,
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
        <span
          title={`Change repository focus · current: ${controller.repositoryPath}`}
          className="flex items-center rounded-lg bg-accent-subtle text-xs font-medium text-accent"
        >
          <button
            type="button"
            onClick={() => void controller.chooseRepository()}
            className="flex items-center gap-1.5 rounded-l-lg py-1 pl-2 pr-1.5 transition hover:bg-accent-soft"
          >
            <FolderGit2 className="size-3.5" />
            @repo {specRepositoryLabel(controller.repositoryPath)}
          </button>
          <button
            type="button"
            onClick={() => void controller.removeRepository()}
            aria-label={`Remove repository focus ${specRepositoryLabel(controller.repositoryPath)}`}
            title="Remove repository focus"
            className="rounded-r-lg py-1 pl-1 pr-1.5 transition hover:bg-accent-soft"
          >
            <X className="size-3" />
          </button>
        </span>
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
