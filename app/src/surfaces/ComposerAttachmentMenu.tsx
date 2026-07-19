import { useEffect, useRef, useState } from "react";
import { FileUp, FolderOpen, Plus } from "lucide-react";

interface ComposerAttachmentMenuProps {
  disabled?: boolean;
  onFiles: (files: File[]) => void;
}

function pickedFiles(input: HTMLInputElement): File[] {
  return Array.from(input.files ?? []);
}

/** Codex-style Add menu. Files and folders share the same attachment pipeline,
 * so every source gets the existing size limits, image processing, chips, and
 * provider upload behavior. */
export function ComposerAttachmentMenu({
  disabled = false,
  onFiles,
}: ComposerAttachmentMenuProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const folderRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    folderRef.current?.setAttribute("webkitdirectory", "");
  }, []);

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setOpen(false);
      triggerRef.current?.focus();
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  const stage = (input: HTMLInputElement) => {
    const files = pickedFiles(input);
    if (files.length > 0) onFiles(files);
    input.value = "";
  };

  const choose = (input: HTMLInputElement | null) => {
    setOpen(false);
    input?.click();
  };

  return (
    <div ref={rootRef} className="relative">
      <input
        ref={fileRef}
        data-testid="composer-file-input"
        type="file"
        multiple
        hidden
        onChange={(event) => stage(event.currentTarget)}
      />
      <input
        ref={folderRef}
        data-testid="composer-folder-input"
        type="file"
        multiple
        hidden
        onChange={(event) => stage(event.currentTarget)}
      />

      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((value) => !value)}
        disabled={disabled}
        aria-label="Add attachments"
        aria-haspopup="menu"
        aria-expanded={open}
        title="Add files or a folder"
        className="grid size-8 shrink-0 place-items-center rounded-full bg-bg-tertiary text-ink-muted transition duration-200 ease-clark hover:bg-accent-subtle hover:text-accent disabled:opacity-40"
      >
        <Plus className="size-4" />
      </button>

      {open && (
        <div
          role="menu"
          aria-label="Add"
          className="popover-surface absolute bottom-full left-0 z-40 mb-2 w-[40rem] max-w-[calc(100vw-4rem)] rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
        >
          <div className="px-2.5 pb-1 pt-1.5 text-xs font-medium text-ink-faint">
            Add
          </div>
          <button
            type="button"
            role="menuitem"
            onClick={() => choose(fileRef.current)}
            className="flex min-h-9 w-full items-center gap-2.5 rounded-xl px-2.5 py-1.5 text-left text-sm text-ink transition duration-200 ease-clark hover:bg-bg-hover focus-visible:bg-bg-hover"
          >
            <FileUp className="size-4 shrink-0 text-ink-muted" />
            <span className="shrink-0">Files</span>
            <span className="min-w-0 truncate text-xs text-ink-faint">
              Images, documents, and other project context
            </span>
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => choose(folderRef.current)}
            className="flex min-h-9 w-full items-center gap-2.5 rounded-xl px-2.5 py-1.5 text-left text-sm text-ink transition duration-200 ease-clark hover:bg-bg-hover focus-visible:bg-bg-hover"
          >
            <FolderOpen className="size-4 shrink-0 text-ink-muted" />
            <span className="shrink-0">Folder</span>
            <span className="min-w-0 truncate text-xs text-ink-faint">
              Attach the files inside a folder
            </span>
          </button>
        </div>
      )}
    </div>
  );
}
