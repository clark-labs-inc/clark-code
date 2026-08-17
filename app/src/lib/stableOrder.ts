// Stable ordering for the conversation lists.
//
// Both the sidebar and the start screen receive a live `conversations` array
// whose `updatedAt` is bumped while work streams. Activity time is therefore
// not a stable display key: sorting on it makes rows leap-frog each other.
//
// `createdAt` expresses the actual UI contract instead: the latest-created
// conversation is on top, and that conversation never moves as messages arrive.
// Unlike a first-seen session rank, creation order also survives relaunches and
// does not depend on whatever ordering the cloud list happened to return.

interface CreatedItem {
  id: string;
  createdAt: number;
}

// Project groups need their own rank table. Reusing conversation ranks would
// make a coincidentally identical project key and conversation id share a
// slot, and—more importantly—removing a project's first conversation must not
// change the position of the whole project.
const projectRanks = new Map<string, number>();
let projectsInitialized = false;
let newProjectsSeen = 0;

function ordered<T extends CreatedItem>(items: T[]): T[] {
  // IDs provide an immutable tie-breaker for conversations created in the same
  // millisecond. Relying on input order would let an updated row move within
  // that tie when the store prepends it.
  return [...items].sort((a, b) => b.createdAt - a.createdAt || a.id.localeCompare(b.id));
}

/** Return conversations newest-created first. `updatedAt` is deliberately not
 * consulted, so streaming, reopening, and background completion cannot move a
 * row. */
export function stableOrderIds<T extends CreatedItem>(items: T[]): T[] {
  return ordered(items);
}

/** Like `stableOrderIds`, but returns an id→ascending-sort-key lookup so
 *  callers can order many buckets (project groups, rows within a group)
 *  consistently in one pass with a plain `keyA - keyB`. */
export function stableRankMap<T extends CreatedItem>(items: T[]): Map<string, number> {
  return new Map(ordered(items).map((item, index) => [item.id, index]));
}

/** Keep project groups in their first-seen session order. New projects land at
 * the top, while removing or archiving conversations inside an existing
 * project cannot move the project itself. */
export function stableProjectOrder<T extends { key: string }>(
  items: T[],
  priority: (item: T) => number = () => 0,
): T[] {
  if (!projectsInitialized) {
    items.forEach((item, index) => {
      if (!projectRanks.has(item.key)) projectRanks.set(item.key, index);
    });
    projectsInitialized = true;
  } else {
    for (const item of items) {
      if (!projectRanks.has(item.key)) {
        projectRanks.set(item.key, -(++newProjectsSeen));
      }
    }
  }
  return [...items].sort((a, b) => {
    const priorityDelta = priority(a) - priority(b);
    return priorityDelta || projectRanks.get(a.key)! - projectRanks.get(b.key)!;
  });
}

/** Reset process-local ordering at an authenticated-account boundary. */
export function resetStableOrder(): void {
  projectsInitialized = false;
  newProjectsSeen = 0;
  projectRanks.clear();
}

/** Test-only alias retained for the focused ordering suite. */
export function __resetStableOrderForTests(): void {
  resetStableOrder();
}
