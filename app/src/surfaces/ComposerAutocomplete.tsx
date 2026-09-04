import {
  FileText,
  Folder,
  FolderOpen,
  Slash,
  Sparkles,
} from "lucide-react";
import { useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { cn } from "../lib/cn";
import type { ComposerSuggestion } from "../lib/composerInput";
import { DUR, EASE, REDUCED_EXIT } from "../lib/motion";
import { productModule } from "../product/productModule";

/** The `@`-file / `/`-command suggestion list, anchored above the textarea. */
export function ComposerAutocomplete({
  suggestions,
  selectedIndex,
  onPick,
  onHover,
}: {
  suggestions: ComposerSuggestion[];
  selectedIndex: number;
  onPick: (suggestion: ComposerSuggestion) => void;
  onHover: (index: number) => void;
}) {
  const reduce = useReducedMotion() ?? false;

  return (
    <m.div
      // Normal motion appears instantly to avoid shadow flicker in WKWebView;
      // reduced motion keeps the product-wide short opacity cue.
      initial={reduce ? { opacity: 0 } : false}
      animate={{ opacity: 1 }}
      exit={reduce ? REDUCED_EXIT : { opacity: 0 }}
      transition={{ duration: DUR.fast, ease: EASE.out }}
      className="popover-surface max-h-64 w-full overflow-y-auto rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle sm:w-80"
    >
      {suggestions.map((suggestion, index) => {
        const key = suggestion.kind === "parent_directory"
            ? `${suggestion.kind}:${suggestion.root}`
          : suggestion.kind === "parent_directory_menu"
            || suggestion.kind === "parent_directory_picker"
          ? suggestion.kind
          : suggestion.kind === "file" || suggestion.kind === "directory"
          ? `${suggestion.kind}:${suggestion.path}`
          : suggestion.kind === "slash"
            ? `/${suggestion.cmd.name}`
            : suggestion.skill.id;
        return (
          <button
            key={key}
            type="button"
            // Use mousedown so the pick fires before the textarea blurs.
            onMouseDown={(event) => {
              event.preventDefault();
              onPick(suggestion);
            }}
            onMouseMove={() => onHover(index)}
            className={cn(
              "flex w-full items-center gap-2 rounded-xl px-2.5 py-2 text-left text-sm transition duration-base ease-agent",
              index === selectedIndex
                ? "bg-accent-subtle text-ink"
                : "text-ink-secondary",
            )}
          >
            {suggestion.kind === "parent_directory_menu" ? (
              <>
                <FolderOpen className="size-3.5 shrink-0 text-accent" />
                <span className="shrink-0 font-mono text-xs text-ink">../</span>
                <span className="min-w-0 flex-1 truncate text-xs text-ink-faint">
                  Browse folders beside this checkout
                </span>
              </>
            ) : suggestion.kind === "parent_directory_picker" ? (
              <>
                <FolderOpen className="size-3.5 shrink-0 text-accent" />
                <span className="shrink-0 text-xs font-medium text-ink">
                  Choose another folder…
                </span>
                <span className="min-w-0 flex-1 truncate text-xs text-ink-faint">
                  Read-only
                </span>
              </>
            ) : suggestion.kind === "parent_directory" ? (
              <>
                <FolderOpen className="size-3.5 shrink-0 text-accent" />
                <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink">
                  {suggestion.path}
                </span>
                <span className="min-w-0 flex-1 truncate text-xs text-ink-faint">
                  Read-only
                </span>
              </>
            ) : suggestion.kind === "file" || suggestion.kind === "directory" ? (
              <>
                {suggestion.kind === "directory" ? (
                  <Folder className="size-3.5 shrink-0 text-ink-faint" />
                ) : (
                  <FileText className="size-3.5 shrink-0 text-ink-faint" />
                )}
                <span className="min-w-0 flex-1 truncate font-mono text-xs">
                  {suggestion.path}{suggestion.kind === "directory" ? "/" : ""}
                </span>
              </>
            ) : suggestion.kind === "slash" ? (
              <>
                <Slash className="size-3.5 shrink-0 text-ink-faint" />
                <span className="shrink-0 font-mono text-xs text-ink">
                  /{suggestion.cmd.name}
                </span>
                <span className="min-w-0 flex-1 truncate text-xs text-ink-faint">
                  {suggestion.cmd.hint}
                </span>
                {suggestion.cmd.gatedWorkflow && (
                  <span className="shrink-0 rounded-md bg-accent/10 px-1.5 py-0.5 text-xs font-medium text-accent">
                    {productModule().localAgent.workflowAccess?.badge ?? "Restricted"}
                  </span>
                )}
              </>
            ) : (
              <>
                <Sparkles className="size-3.5 shrink-0 text-ink-faint" />
                <span className="shrink-0 font-mono text-xs text-ink">
                  ${suggestion.skill.invocationName}
                </span>
                <span className="min-w-0 flex-1 truncate text-xs text-ink-faint">
                  {suggestion.skill.enabled
                    ? suggestion.skill.description
                    : suggestion.skill.disabledReason}
                </span>
              </>
            )}
          </button>
        );
      })}
    </m.div>
  );
}
