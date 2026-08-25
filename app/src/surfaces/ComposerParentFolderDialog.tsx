import { useEffect, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { FolderOpen, X } from "lucide-react";
import { DIALOG, OVERLAY, accessibleMotion } from "../lib/motion";

export function isAbsoluteFolderPath(value: string): boolean {
  const path = value.trim();
  return path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path) || path.startsWith("\\\\");
}

export function ComposerParentFolderDialog({
  open,
  suggestedBase,
  remoteHost,
  onCancel,
  onChoose,
}: {
  open: boolean;
  suggestedBase: string;
  remoteHost?: string | null;
  onCancel: () => void;
  onChoose: (path: string) => void;
}) {
  const reduceMotion = useReducedMotion();
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setPath("");
    setError(null);
  }, [open, suggestedBase]);

  const submit = () => {
    const selected = path.trim().replace(/[\\/]+$/, "");
    if (!isAbsoluteFolderPath(selected)) {
      setError("Enter an absolute folder path.");
      return;
    }
    onChoose(selected);
  };

  return (
    <AnimatePresence>
      {open && (
        <m.div
          {...accessibleMotion(OVERLAY, reduceMotion)}
          className="fixed inset-0 z-50 grid place-items-center bg-scrim p-6"
          onMouseDown={(event) => event.target === event.currentTarget && onCancel()}
        >
          <m.div
            {...accessibleMotion(DIALOG, reduceMotion)}
            role="dialog"
            aria-modal="true"
            aria-labelledby="parent-folder-dialog-title"
            className="popover-surface w-full max-w-md rounded-2xl border border-border bg-bg-elevated shadow-2xl"
          >
            <div className="flex items-start gap-3 border-b border-border-subtle px-5 py-4">
              <span className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-xl bg-accent-subtle text-accent">
                <FolderOpen className="size-4" aria-hidden="true" />
              </span>
              <div className="min-w-0 flex-1">
                <h2 id="parent-folder-dialog-title" className="text-base font-semibold text-ink">
                  Attach a read-only folder
                </h2>
                <p className="mt-1 text-xs leading-5 text-ink-muted">
                  {remoteHost
                    ? <>Enter an absolute path on <span className="font-mono text-ink-secondary">{remoteHost}</span>.</>
                    : "The browser preview cannot open the system folder picker. Paste an absolute path instead."}
                </p>
              </div>
              <button
                type="button"
                onClick={onCancel}
                aria-label="Close folder path dialog"
                className="-mr-1 grid size-8 shrink-0 place-items-center rounded-lg text-ink-muted transition duration-fast ease-agent hover:bg-bg-hover hover:text-ink"
              >
                <X className="size-4" />
              </button>
            </div>

            <form
              className="space-y-3 px-5 py-4"
              onSubmit={(event) => {
                event.preventDefault();
                submit();
              }}
            >
              <label className="block text-xs font-medium text-ink-secondary" htmlFor="parent-folder-path">
                {remoteHost ? "Remote folder path" : "Folder path"}
              </label>
              <input
                id="parent-folder-path"
                autoFocus
                value={path}
                onChange={(event) => {
                  setPath(event.target.value);
                  if (error) setError(null);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Escape") onCancel();
                }}
                placeholder={suggestedBase ? `${suggestedBase}/folder` : "/Users/you/code/project"}
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                className="w-full rounded-xl border border-border bg-bg px-3 py-2.5 font-mono text-sm text-ink outline-none transition duration-fast ease-agent placeholder:text-ink-faint focus:border-accent focus:ring-2 focus:ring-accent/15"
              />
              {error && <p role="alert" className="text-xs text-danger">{error}</p>}
              <p className="text-xs leading-5 text-ink-faint">
                Clark adds this location to the current message as a read-only reference. It does not change the writable {remoteHost ? "remote " : ""}checkout.
              </p>
              <div className="flex justify-end gap-2 pt-1">
                <button
                  type="button"
                  onClick={onCancel}
                  className="rounded-xl px-3 py-2 text-sm font-medium text-ink-muted transition duration-fast ease-agent hover:bg-bg-hover hover:text-ink"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={!path.trim()}
                  className="rounded-xl bg-accent px-3 py-2 text-sm font-semibold text-on-accent transition duration-fast ease-agent hover:bg-accent-hover disabled:cursor-not-allowed disabled:bg-bg-tertiary disabled:text-ink-faint"
                >
                  Attach folder
                </button>
              </div>
            </form>
          </m.div>
        </m.div>
      )}
    </AnimatePresence>
  );
}
