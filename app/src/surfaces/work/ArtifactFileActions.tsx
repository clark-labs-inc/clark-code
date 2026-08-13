import { Copy, Download, ExternalLink, FolderOpen } from "lucide-react";
import type { Artifact } from "../../core-bridge/types";
import { openExternal } from "../../lib/account";
import { copyText } from "../../lib/clipboard";
import { cn } from "../../lib/cn";
import { isLocalDocUri, toPath } from "../../lib/docs";
import { openLocalPath, saveArtifactCopy } from "../../lib/fileLinks";
import { useSessionStore } from "../../store/sessionStore";

export function fileManagerLabel(): string {
  if (typeof navigator === "undefined") return "Show in File Manager";
  if (/mac/i.test(navigator.userAgent)) return "Show in Finder";
  if (/windows/i.test(navigator.userAgent)) return "Show in File Explorer";
  return "Show in File Manager";
}

export async function openArtifactExternally(artifact: Artifact): Promise<void> {
  if (!artifact.uri) return;
  try {
    if (isLocalDocUri(artifact.uri)) await openLocalPath(toPath(artifact.uri));
    else await openExternal(artifact.uri);
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    useSessionStore.getState().flashNotice(`Could not open ${artifact.title}: ${detail}`);
  }
}

function actionClass(compact: boolean): string {
  return cn(
    "flex items-center gap-1.5 whitespace-nowrap rounded-lg text-xs font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent",
    compact ? "h-7 px-2" : "h-8 px-2.5",
  );
}

/** Visible, consistent actions for every artifact source we can deliver. */
export function ArtifactFileActions({
  artifact,
  compact = false,
  className,
}: {
  artifact: Artifact;
  compact?: boolean;
  className?: string;
}) {
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const uri = artifact.uri;
  if (!uri) return null;

  const localPath = isLocalDocUri(uri) ? toPath(uri) : null;
  const embedded = /^(?:data|blob):/i.test(uri);
  const web = /^https?:/i.test(uri);
  const revealLabel = fileManagerLabel();

  const run = (action: () => Promise<void>, failure: string) => {
    void action().catch((error: unknown) => {
      const detail = error instanceof Error ? error.message : String(error);
      flashNotice(`${failure}: ${detail}`);
    });
  };

  return (
    <div className={cn("flex flex-wrap items-center gap-0.5", className)} aria-label={`Actions for ${artifact.title}`}>
      {(localPath || web) && (
        <button
          type="button"
          className={actionClass(compact)}
          onClick={() => run(
            async () => localPath ? openLocalPath(localPath) : openExternal(uri),
            `Could not open ${artifact.title}`,
          )}
        >
          <ExternalLink className="size-3.5" /> Open
        </button>
      )}
      {localPath && (
        <button
          type="button"
          className={actionClass(compact)}
          onClick={() => run(
            () => openLocalPath(localPath, true),
            `Could not ${revealLabel.toLowerCase()}`,
          )}
        >
          <FolderOpen className="size-3.5" /> {revealLabel}
        </button>
      )}
      {(localPath || embedded) && (
        <button
          type="button"
          className={actionClass(compact)}
          onClick={() => run(async () => {
            if (await saveArtifactCopy(uri, artifact.title)) flashNotice("Artifact copy saved.");
          }, `Could not save ${artifact.title}`)}
        >
          <Download className="size-3.5" /> Save a Copy
        </button>
      )}
      {localPath && (
        <button
          type="button"
          className={actionClass(compact)}
          onClick={() => run(async () => {
            if (!(await copyText(localPath))) throw new Error("clipboard unavailable");
            flashNotice("File path copied.");
          }, "Could not copy path")}
        >
          <Copy className="size-3.5" /> Copy Path
        </button>
      )}
    </div>
  );
}
