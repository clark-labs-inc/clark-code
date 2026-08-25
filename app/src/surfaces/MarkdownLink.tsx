import { useEffect, useRef, useState, type AnchorHTMLAttributes } from "react";
import { createPortal } from "react-dom";
import { Copy, Download, ExternalLink, FolderOpen } from "lucide-react";
import { openExternal } from "../lib/account";
import { copyText } from "../lib/clipboard";
import {
  localPathFromHref,
  openLocalPath,
  saveLocalFileCopy,
} from "../lib/fileLinks";
import { useSessionStore } from "../store/sessionStore";
import { fileManagerLabel } from "./work/ArtifactFileActions";

type MenuPosition = { x: number; y: number };

function menuPosition(clientX: number, clientY: number): MenuPosition {
  const width = 224;
  const height = 164;
  return {
    x: Math.max(8, Math.min(clientX, window.innerWidth - width - 8)),
    y: Math.max(8, Math.min(clientY, window.innerHeight - height - 8)),
  };
}

function MenuItem({
  icon: Icon,
  children,
  onClick,
}: {
  icon: typeof ExternalLink;
  children: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-sm text-ink-secondary transition hover:bg-accent-subtle hover:text-ink focus-visible:bg-accent-subtle focus-visible:text-ink focus-visible:outline-none"
    >
      <Icon className="size-4 shrink-0" />
      {children}
    </button>
  );
}

function LocalFileMenu({
  path,
  position,
  onClose,
}: {
  path: string;
  position: MenuPosition;
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const revealLabel = fileManagerLabel();

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      menuRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
    });
    const closeOnPointer = (event: PointerEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) onClose();
    };
    const closeOnKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    const closeOnMove = () => onClose();
    document.addEventListener("pointerdown", closeOnPointer);
    document.addEventListener("keydown", closeOnKey);
    window.addEventListener("resize", closeOnMove);
    window.addEventListener("scroll", closeOnMove, true);
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener("pointerdown", closeOnPointer);
      document.removeEventListener("keydown", closeOnKey);
      window.removeEventListener("resize", closeOnMove);
      window.removeEventListener("scroll", closeOnMove, true);
    };
  }, [onClose]);

  const run = (action: () => Promise<void>, failure: string) => {
    onClose();
    void action().catch((error: unknown) => {
      const detail = error instanceof Error ? error.message : String(error);
      flashNotice(`${failure}: ${detail}`);
    });
  };

  return createPortal(
    <div
      ref={menuRef}
      role="menu"
      aria-label="File actions"
      style={{ left: position.x, top: position.y }}
      className="popover-surface fixed z-critical w-56 rounded-xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
    >
      <MenuItem
        icon={ExternalLink}
        onClick={() => run(() => openLocalPath(path), "Couldn't open file")}
      >
        Open
      </MenuItem>
      <MenuItem
        icon={FolderOpen}
        onClick={() =>
          run(() => openLocalPath(path, true), `Couldn't ${revealLabel.toLowerCase()}`)
        }
      >
        {revealLabel}
      </MenuItem>
      <MenuItem
        icon={Download}
        onClick={() =>
          run(async () => {
            const saved = await saveLocalFileCopy(path);
            if (saved) flashNotice("File copy saved.");
          }, "Couldn't save file")
        }
      >
        Save a Copy…
      </MenuItem>
      <div className="my-1 border-t border-border-subtle" />
      <MenuItem
        icon={Copy}
        onClick={() =>
          run(async () => {
            if (!(await copyText(path))) throw new Error("clipboard unavailable");
            flashNotice("File path copied.");
          }, "Couldn't copy path")
        }
      >
        Copy Path
      </MenuItem>
    </div>,
    document.body,
  );
}

/** Markdown anchor that routes web URLs to the system browser and filesystem
 * paths to native desktop actions instead of asking WebKit to navigate them. */
export function MarkdownLink({ href, children, ...props }: AnchorHTMLAttributes<HTMLAnchorElement>) {
  const cwd = useSessionStore((state) => state.activeProjectRoot ?? state.localSettings.cwd);
  const remote = useSessionStore((state) => state.activeRemote !== null);
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const [menu, setMenu] = useState<MenuPosition | null>(null);
  const path = remote ? null : localPathFromHref(href, cwd);

  if (!path) {
    return (
      <a
        {...props}
        href={href}
        target="_blank"
        rel="noreferrer noopener"
        onClick={(event) => {
          props.onClick?.(event);
          if (event.defaultPrevented || !href || href.startsWith("#")) return;
          event.preventDefault();
          void openExternal(href);
        }}
      >
        {children}
      </a>
    );
  }

  return (
    <>
      <a
        {...props}
        href={href}
        data-local-file={path}
        title={`${path}\nRight-click for file actions`}
        onClick={(event) => {
          props.onClick?.(event);
          if (event.defaultPrevented) return;
          event.preventDefault();
          void openLocalPath(path).catch((error: unknown) => {
            const detail = error instanceof Error ? error.message : String(error);
            flashNotice(`Couldn't open file: ${detail}`);
          });
        }}
        onContextMenu={(event) => {
          props.onContextMenu?.(event);
          if (event.defaultPrevented) return;
          event.preventDefault();
          setMenu(menuPosition(event.clientX, event.clientY));
        }}
      >
        {children}
      </a>
      {menu && <LocalFileMenu path={path} position={menu} onClose={() => setMenu(null)} />}
    </>
  );
}
