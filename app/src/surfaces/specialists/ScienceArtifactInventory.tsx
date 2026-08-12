import { BookMarked, Cloud, Database } from "lucide-react";

import type { ScienceArtifactSegment } from "../../lib/specialistCloud";
import { MetricCard, SectionCard, StatusPill } from "./SpecialistPrimitives";

interface CloudFile {
  key: string;
  logicalPath: string;
  contentType: string;
  sourceResidency: ScienceArtifactSegment["sourceResidency"];
  isJournal: boolean;
  fileSizeBytes: number;
  fileSha256: string;
  segmentCount: number;
  verifiedAt: string;
}

export function ScienceArtifactInventory({
  artifacts,
}: {
  artifacts: ScienceArtifactSegment[];
}) {
  const files = latestCloudFiles(artifacts);
  const totalBytes = files.reduce((total, file) => total + file.fileSizeBytes, 0);
  const journals = files.filter((file) => file.isJournal).length;
  return (
    <>
      <div className="grid gap-3 sm:grid-cols-3">
        <MetricCard label="Cloud files" value={files.length} tone="good" />
        <MetricCard label="Journals" value={journals} />
        <MetricCard label="Verified bytes" value={formatBytes(totalBytes)} />
      </div>
      <SectionCard
        title="product cloud science inventory"
        detail="Every row is content-addressed, versioned, and verified before its Scientist run can finish."
      >
        {files.length === 0 ? (
          <div className="flex items-start gap-3 px-4 pb-4 text-xs leading-5 text-ink-muted">
            <Cloud className="mt-0.5 size-4 shrink-0 text-accent" />
            Cloud artifacts will appear after the next GUI or headless science action completes.
          </div>
        ) : (
          <div className="divide-y divide-border-subtle">
            {files.map((file) => (
              <div key={file.key} className="flex items-start gap-3 px-4 py-3">
                <span className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-lg bg-accent-soft text-accent">
                  {file.isJournal
                    ? <BookMarked className="size-4" />
                    : <Database className="size-4" />}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium text-ink" title={file.logicalPath}>
                    {file.logicalPath}
                  </div>
                  <div className="mt-0.5 text-xs text-ink-muted">
                    {formatBytes(file.fileSizeBytes)} · {file.segmentCount} verified {file.segmentCount === 1 ? "segment" : "segments"}
                  </div>
                  <div className="mt-1 truncate font-mono text-xs text-ink-faint">
                    {file.sourceResidency.replaceAll("_", " ")} · sha256:{file.fileSha256}
                  </div>
                </div>
                <StatusPill status="verified" />
              </div>
            ))}
          </div>
        )}
      </SectionCard>
    </>
  );
}

function latestCloudFiles(segments: ScienceArtifactSegment[]): CloudFile[] {
  const versions = new Map<string, CloudFile>();
  for (const segment of segments) {
    if (segment.state !== "verified") continue;
    const key = `${segment.logicalPath}\u0000${segment.fileSha256}`;
    const existing = versions.get(key);
    if (!existing || segment.verifiedAt > existing.verifiedAt) {
      versions.set(key, {
        key,
        logicalPath: segment.logicalPath,
        contentType: segment.contentType,
        sourceResidency: segment.sourceResidency,
        isJournal: segment.isJournal,
        fileSizeBytes: segment.fileSizeBytes,
        fileSha256: segment.fileSha256,
        segmentCount: segment.segmentCount,
        verifiedAt: segment.verifiedAt,
      });
    }
  }
  const latest = new Map<string, CloudFile>();
  for (const file of versions.values()) {
    const existing = latest.get(file.logicalPath);
    if (!existing || file.verifiedAt > existing.verifiedAt) latest.set(file.logicalPath, file);
  }
  return [...latest.values()].sort((left, right) =>
    right.verifiedAt.localeCompare(left.verifiedAt) || left.logicalPath.localeCompare(right.logicalPath));
}

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let scaled = value;
  let unit = -1;
  do {
    scaled /= 1024;
    unit += 1;
  } while (scaled >= 1024 && unit < units.length - 1);
  return `${scaled >= 10 ? scaled.toFixed(0) : scaled.toFixed(1)} ${units[unit]}`;
}
