export type SpecDiffKind = "equal" | "add" | "remove";

export interface SpecDiffRow {
  kind: SpecDiffKind;
  text: string;
  previousLine: number | null;
  nextLine: number | null;
}

export interface SpecDocumentDiff {
  added: number;
  removed: number;
  rows: SpecDiffRow[];
}

export interface SpecDocumentInteraction {
  ariaBusy: boolean;
  className: "cursor-wait select-none" | "cursor-text select-text";
  canSelect: boolean;
}

export function specDocumentInteraction(busy: boolean): SpecDocumentInteraction {
  return {
    ariaBusy: busy,
    className: busy ? "cursor-wait select-none" : "cursor-text select-text",
    canSelect: !busy,
  };
}

const MAX_LCS_LINES = 600;

function replacementRows(
  previous: string[],
  next: string[],
  previousOffset: number,
  nextOffset: number,
): SpecDiffRow[] {
  const operations: SpecDiffRow[] = [];
  const length = Math.max(previous.length, next.length);
  for (let index = 0; index < length; index += 1) {
    if (index < previous.length) {
      operations.push({
        kind: "remove",
        text: previous[index],
        previousLine: previousOffset + index + 1,
        nextLine: null,
      });
    }
    if (index < next.length) {
      operations.push({
        kind: "add",
        text: next[index],
        previousLine: null,
        nextLine: nextOffset + index + 1,
      });
    }
  }
  return operations;
}

function changedMiddle(previous: string[], next: string[]): SpecDiffRow[] {
  let prefix = 0;
  while (prefix < previous.length && prefix < next.length && previous[prefix] === next[prefix]) {
    prefix += 1;
  }

  let suffix = 0;
  while (
    suffix < previous.length - prefix
    && suffix < next.length - prefix
    && previous[previous.length - 1 - suffix] === next[next.length - 1 - suffix]
  ) {
    suffix += 1;
  }

  return [
    ...previous.slice(0, prefix).map((text, index) => ({
      kind: "equal" as const,
      text,
      previousLine: index + 1,
      nextLine: index + 1,
    })),
    ...replacementRows(
      previous.slice(prefix, previous.length - suffix),
      next.slice(prefix, next.length - suffix),
      prefix,
      prefix,
    ),
    ...next.slice(next.length - suffix).map((text, index) => ({
      kind: "equal" as const,
      text,
      previousLine: previous.length - suffix + index + 1,
      nextLine: next.length - suffix + index + 1,
    })),
  ];
}

function lineOperations(previous: string[], next: string[]): SpecDiffRow[] {
  if (previous.length > MAX_LCS_LINES || next.length > MAX_LCS_LINES) {
    return changedMiddle(previous, next);
  }

  const rows = Array.from(
    { length: previous.length + 1 },
    () => new Uint16Array(next.length + 1),
  );
  for (let previousIndex = previous.length - 1; previousIndex >= 0; previousIndex -= 1) {
    for (let nextIndex = next.length - 1; nextIndex >= 0; nextIndex -= 1) {
      rows[previousIndex][nextIndex] = previous[previousIndex] === next[nextIndex]
        ? rows[previousIndex + 1][nextIndex + 1] + 1
        : Math.max(rows[previousIndex + 1][nextIndex], rows[previousIndex][nextIndex + 1]);
    }
  }

  const operations: SpecDiffRow[] = [];
  let previousIndex = 0;
  let nextIndex = 0;
  while (previousIndex < previous.length || nextIndex < next.length) {
    if (
      previousIndex < previous.length
      && nextIndex < next.length
      && previous[previousIndex] === next[nextIndex]
    ) {
      operations.push({
        kind: "equal",
        text: previous[previousIndex],
        previousLine: previousIndex + 1,
        nextLine: nextIndex + 1,
      });
      previousIndex += 1;
      nextIndex += 1;
    } else if (
      previousIndex < previous.length
      && nextIndex < next.length
      && rows[previousIndex + 1][nextIndex] === rows[previousIndex][nextIndex + 1]
      && rows[previousIndex + 1][nextIndex + 1] === rows[previousIndex + 1][nextIndex]
    ) {
      operations.push(
        {
          kind: "remove",
          text: previous[previousIndex],
          previousLine: previousIndex + 1,
          nextLine: null,
        },
        {
          kind: "add",
          text: next[nextIndex],
          previousLine: null,
          nextLine: nextIndex + 1,
        },
      );
      previousIndex += 1;
      nextIndex += 1;
    } else if (
      previousIndex < previous.length
      && (nextIndex === next.length
        || rows[previousIndex + 1][nextIndex] >= rows[previousIndex][nextIndex + 1])
    ) {
      operations.push({
        kind: "remove",
        text: previous[previousIndex],
        previousLine: previousIndex + 1,
        nextLine: null,
      });
      previousIndex += 1;
    } else {
      operations.push({
        kind: "add",
        text: next[nextIndex],
        previousLine: null,
        nextLine: nextIndex + 1,
      });
      nextIndex += 1;
    }
  }
  return operations;
}

/** Builds the complete in-document transition for one saved Spec revision.
 * Equal rows preserve the document's shape while changed rows animate in situ;
 * the clean Markdown remains the durable source of truth after it settles. */
export function specDocumentDiff(
  previousMarkdown: string,
  nextMarkdown: string,
): SpecDocumentDiff | null {
  if (previousMarkdown === nextMarkdown) return null;
  const rows = lineOperations(previousMarkdown.split("\n"), nextMarkdown.split("\n"));
  return {
    added: rows.filter((row) => row.kind === "add" && row.text.trim()).length,
    removed: rows.filter((row) => row.kind === "remove" && row.text.trim()).length,
    rows,
  };
}
