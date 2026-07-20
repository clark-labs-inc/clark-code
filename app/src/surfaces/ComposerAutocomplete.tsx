import { FileText, Slash } from "lucide-react";
import { motion } from "motion/react";
import { cn } from "../lib/cn";
import type { ComposerSuggestion } from "../lib/composerInput";
import { DUR, EASE } from "../lib/motion";

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
  return (
    <motion.div
      // Appear instantly: fading a shadowed popover in WKWebView reads as flicker.
      initial={false}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: DUR.fast, ease: EASE.out }}
      className="popover-surface max-h-64 w-full overflow-y-auto rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle sm:w-80"
    >
      {suggestions.map((suggestion, index) => {
        const key = suggestion.kind === "file" ? suggestion.path : `/${suggestion.cmd.name}`;
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
              "flex w-full items-center gap-2 rounded-xl px-2.5 py-2 text-left text-sm transition duration-200 ease-clark",
              index === selectedIndex
                ? "bg-accent-subtle text-ink"
                : "text-ink-secondary",
            )}
          >
            {suggestion.kind === "file" ? (
              <>
                <FileText className="size-3.5 shrink-0 text-ink-faint" />
                <span className="min-w-0 flex-1 truncate font-mono text-xs">
                  {suggestion.path}
                </span>
              </>
            ) : (
              <>
                <Slash className="size-3.5 shrink-0 text-ink-faint" />
                <span className="shrink-0 font-mono text-xs text-ink">
                  /{suggestion.cmd.name}
                </span>
                <span className="min-w-0 flex-1 truncate text-xs text-ink-faint">
                  {suggestion.cmd.hint}
                </span>
              </>
            )}
          </button>
        );
      })}
    </motion.div>
  );
}
