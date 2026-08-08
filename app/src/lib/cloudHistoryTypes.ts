import type { WireSnapshot } from "../core-bridge/types";
import type { ConversationMeta } from "./history";
import type { SpecialistContext } from "./specialists";

export interface CloudSummary {
  id: string;
  title: string;
  provider: string;
  project?: string;
  remoteHost?: string;
  mode?: string;
  titleLocked?: boolean;
  archived?: boolean;
  createdAt: string | number;
  updatedAt: string | number;
  rev: number;
  specialistContext?: SpecialistContext;
}

export interface CloudDetail extends Partial<CloudSummary> {
  snapshot?: WireSnapshot;
  snapshotRecoveryRequired?: boolean;
}

export function metaFromSummary(row: CloudSummary): ConversationMeta {
  const timestamp = (value: string | number) =>
    typeof value === "number" ? value : Date.parse(value) || Date.now();
  return {
    id: row.id,
    title: row.title,
    provider: row.provider,
    project: row.project || undefined,
    remoteHost: row.remoteHost || undefined,
    mode: row.mode || undefined,
    titleLocked: row.titleLocked || undefined,
    archived: row.archived || undefined,
    createdAt: timestamp(row.createdAt),
    updatedAt: timestamp(row.updatedAt),
    rev: row.rev,
    specialist: row.specialistContext,
  };
}

export function metaFromDetail(detail: CloudDetail): ConversationMeta | null {
  if (
    typeof detail.id !== "string"
    || typeof detail.title !== "string"
    || typeof detail.provider !== "string"
    || (typeof detail.createdAt !== "string" && typeof detail.createdAt !== "number")
    || (typeof detail.updatedAt !== "string" && typeof detail.updatedAt !== "number")
    || typeof detail.rev !== "number"
  ) {
    return null;
  }
  return metaFromSummary({
    id: detail.id,
    title: detail.title,
    provider: detail.provider,
    project: detail.project,
    remoteHost: detail.remoteHost,
    mode: detail.mode,
    titleLocked: detail.titleLocked,
    archived: detail.archived,
    createdAt: detail.createdAt,
    updatedAt: detail.updatedAt,
    rev: detail.rev,
    specialistContext: detail.specialistContext,
  });
}
