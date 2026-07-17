// Stable ordering for the conversation lists.
//
// Both the sidebar and the start screen re-derive their order from the live
// `conversations` array, whose `updatedAt` is bumped on every streamed token
// of EVERY running conversation. With several chats running in parallel a pure
// `sort by updatedAt desc` re-ranks on each flush, so rows and whole project
// groups leap-frog each other — the "keeps reordering" bug.
//
// The fix ranks each conversation by when it was FIRST SEEN this session, not
// by its current timestamp:
//   • ids present on the very first call are treated as pre-existing history —
//     they keep their incoming (store = newest-first) order, and never move.
//   • an id that appears LATER is genuinely new activity (a freshly started
//     chat, prepended by the store) — it lands on top of the history block.
// After that every id keeps its slot forever, so a streaming conversation's
// ticking `updatedAt` can never reshuffle the list mid-session.
//
// The rank table is module-level (persists across re-renders, shared by every
// surface in the session) and keyed by conversation id. Each id maps to ONE
// ascending sort key, so callers sort with a plain `keyA - keyB`.

// Each id's ascending sort key. History (first-sight) ids occupy
// `0 .. historyCount-1` in store order. New-activity ids get NEGATIVE keys
// counting down (-1, -2, …), so the newest new conversation has the smallest
// key and sorts on top, above all history — and never collides with history.
const ranks = new Map<string, number>();
let initialized = false;
let newSeen = 0;

function assignRanks(items: { id: string }[]): void {
  if (!initialized) {
    // First sight = session history, already newest-first from the store.
    // Rank by index so each keeps its slot; none ever moves again.
    items.forEach((item, i) => {
      if (!ranks.has(item.id)) ranks.set(item.id, i);
    });
    initialized = true;
    return;
  }
  for (const item of items) {
    if (!ranks.has(item.id)) {
      // New after session start → above all history, newest-of-new first.
      ranks.set(item.id, -(++newSeen));
    }
  }
}

function ordered<T extends { id: string }>(items: T[]): T[] {
  assignRanks(items);
  // A stable sort keeps arrival order for any (clamped) equal keys.
  return [...items].sort((a, b) => ranks.get(a.id)! - ranks.get(b.id)!);
}

/** Return `items` in stable session order. The live array the store holds is
 *  used only to (a) seed history order on first call and (b) detect brand-new
 *  conversations; already-seen ids keep their original slot regardless of how
 *  their `updatedAt` has since moved. */
export function stableOrderIds<T extends { id: string }>(items: T[]): T[] {
  return ordered(items);
}

/** Like `stableOrderIds`, but returns an id→ascending-sort-key lookup so
 *  callers can order many buckets (project groups, rows within a group)
 *  consistently in one pass with a plain `keyA - keyB`. */
export function stableRankMap<T extends { id: string }>(items: T[]): Map<string, number> {
  assignRanks(items);
  const m = new Map<string, number>();
  for (const item of items) m.set(item.id, ranks.get(item.id)!);
  return m;
}

/** Test-only: reset the session rank table. */
export function __resetStableOrderForTests(): void {
  initialized = false;
  newSeen = 0;
  ranks.clear();
}
