import type { ConversationMeta } from "./history";
import type { SpecialistKind } from "./specialists";

export function specialistConversationsForNavigation(
  conversations: readonly ConversationMeta[],
  kind: SpecialistKind,
): ConversationMeta[] {
  return conversations
    .filter((conversation) => !conversation.archived && conversation.specialist?.kind === kind)
    .slice(0, 4);
}
