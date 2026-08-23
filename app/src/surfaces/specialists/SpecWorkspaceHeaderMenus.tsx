import { FileText } from "lucide-react";
import { specFilename } from "../../lib/specDocuments";
import type { SpecPromptHistoryItem } from "../../lib/specPromptHistory";

const PANEL =
  "popover-surface absolute right-0 top-full z-30 mt-1 rounded-xl bg-bg-elevated shadow-lifted ring-1 ring-border-subtle";

/** Recent prompts for this spec, newest first, each restorable into the composer. */
export function SpecPromptHistoryMenu({
  prompts,
  onPick,
}: {
  prompts: readonly SpecPromptHistoryItem[];
  onPick: (text: string) => void;
}) {
  return (
    <div
      data-qa="spec-prompt-history"
      className={`${PANEL} w-80 max-w-[calc(100vw-2rem)] p-2`}
    >
      <div className="flex items-center justify-between px-2 py-1.5">
        <p className="text-xs font-semibold text-ink">Recent prompts</p>
        <span className="text-xs text-ink-faint">Last {prompts.length}</span>
      </div>
      {prompts.length === 0 ? (
        <p className="px-2 py-3 text-xs leading-5 text-ink-faint">
          Your latest prompts will stay here for context.
        </p>
      ) : (
        <ol className="max-h-72 space-y-1 overflow-y-auto">
          {[...prompts].reverse().map((prompt, index) => (
            <li key={`${prompt.submittedAt}:${prompt.text}`}>
              <button
                type="button"
                onClick={() => onPick(prompt.text)}
                title="Put this prompt back in the composer"
                className="w-full rounded-lg px-2.5 py-2 text-left text-xs leading-5 text-ink-secondary hover:bg-bg-hover hover:text-ink"
              >
                <span className="mr-2 text-ink-faint">{prompts.length - index}.</span>
                {prompt.text}
              </button>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

/** Export the spec document as Markdown or PDF. */
export function SpecDownloadMenu({
  documentTitle,
  onDownload,
}: {
  documentTitle: string | null;
  onDownload: (format: "md" | "pdf") => void;
}) {
  return (
    <div className={`${PANEL} w-52 p-1.5`}>
      {(["md", "pdf"] as const).map((format) => (
        <button
          key={format}
          type="button"
          onClick={() => onDownload(format)}
          className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-xs text-ink hover:bg-bg-hover"
        >
          <FileText className="size-4 text-ink-muted" /> {specFilename(documentTitle, format)}
        </button>
      ))}
    </div>
  );
}
