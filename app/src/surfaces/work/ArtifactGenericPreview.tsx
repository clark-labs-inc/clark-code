import { ExternalLink, type LucideIcon } from "lucide-react";
import type { Artifact } from "../../core-bridge/types";
import {
  artifactAvailability,
  artifactLocationLabel,
  canOpenArtifactExternally,
  readableArtifactLocation,
} from "../../lib/artifactPresentation";
import { openArtifactExternally } from "./ArtifactFileActions";

export function ArtifactGenericPreview({ artifact, Icon, kindLabel }: {
  artifact: Artifact;
  Icon: LucideIcon;
  kindLabel: string;
}) {
  const location = readableArtifactLocation(artifact);
  const available = artifactAvailability(artifact) !== "unavailable";
  return (
    <div className="mx-auto flex min-h-full max-w-3xl items-center justify-center px-8 py-12">
      <div className="w-full rounded-xl border border-border bg-bg-elevated px-7 py-8 text-center shadow-soft">
        <span className="mx-auto grid size-14 place-items-center rounded-xl bg-accent-subtle text-accent"><Icon className="size-6" /></span>
        <h1 className="mt-4 font-display text-2xl text-ink">{artifact.title}</h1>
        <p className="mt-1 text-sm text-ink-muted">{kindLabel} · {artifactLocationLabel(artifact)}</p>
        {location && <p className="mx-auto mt-3 max-w-lg truncate font-mono text-xs text-ink-faint">{location}</p>}
        <p className="mx-auto mt-5 max-w-md text-sm leading-relaxed text-ink-muted">
          {available ? "A preview is unavailable. Open the source file to inspect this artifact." : "The source file is unavailable. Its record remains in this session."}
        </p>
        {canOpenArtifactExternally(artifact) && (
          <button type="button" onClick={() => void openArtifactExternally(artifact)}
            className="mt-6 inline-flex h-10 items-center gap-2 rounded-lg bg-accent px-4 text-sm font-medium text-on-accent transition hover:bg-accent-hover">
            Open {artifact.title} <ExternalLink className="size-3.5" />
          </button>
        )}
      </div>
    </div>
  );
}
