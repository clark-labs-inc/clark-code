import type { Artifact } from "../core-bridge/types";
import { isCloudArtifactUri, isWorkspaceArtifactUri } from "./cloudArtifacts";
import { isLocalDocUri } from "./docs";

export type ArtifactAvailability = "saved" | "available" | "unavailable";

export function artifactAvailability(artifact: Artifact): ArtifactAvailability {
  if (!artifact.uri) return "unavailable";
  return isLocalDocUri(artifact.uri) || isWorkspaceArtifactUri(artifact.uri)
    ? "saved"
    : "available";
}

export function artifactLocationLabel(artifact: Artifact): string {
  const uri = artifact.uri;
  if (!uri) return "Unavailable";
  if (/^data:/i.test(uri)) return "Embedded";
  if (/^blob:/i.test(uri)) return "Temporary";
  if (isCloudArtifactUri(uri)) return "Clark cloud";
  if (isWorkspaceArtifactUri(uri)) return "Local";
  return isLocalDocUri(uri) ? "Local" : "Remote";
}

export function readableArtifactLocation(artifact: Artifact): string | null {
  const uri = artifact.uri;
  if (!uri) return null;
  if (/^data:/i.test(uri)) return "Embedded in this task";
  if (/^blob:/i.test(uri)) return "Temporary browser preview";
  if (isCloudArtifactUri(uri)) return "Saved securely in Clark cloud";
  if (isWorkspaceArtifactUri(uri)) return "Clark workspace on this device";
  if (isLocalDocUri(uri)) {
    const parts = uri.split(/[\\/]/).filter(Boolean);
    return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : uri;
  }
  try {
    const url = new URL(uri);
    if (url.protocol === "http:" || url.protocol === "https:") {
      return `${url.hostname}${url.pathname === "/" ? "" : url.pathname}`;
    }
    return `${url.protocol.slice(0, -1)} resource`;
  } catch {
    return uri;
  }
}

export function canOpenArtifactExternally(artifact: Artifact): boolean {
  return !!artifact.uri
    && !/^(?:data|blob):/i.test(artifact.uri)
    && !isCloudArtifactUri(artifact.uri)
    && !isWorkspaceArtifactUri(artifact.uri);
}
