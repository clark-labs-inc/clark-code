import type { CodeRemoteCommand } from "./mobileRemote";

/**
 * Follow-ups must stay in the server-backed command queue until their target
 * conversation can start them. A desktop-local queue is intentionally
 * ephemeral and cannot be treated as a durable mobile command receipt.
 */
export function mobileRemoteCommandWaitsForIdle(
  command: Pick<CodeRemoteCommand, "command_type" | "desktop_id">,
  targetConversationBusy: boolean,
): boolean {
  return (
    (
      command.command_type === "send_message"
      || command.command_type === "compact_conversation"
      || command.command_type === "edit_and_resend"
    )
    && Boolean(command.desktop_id)
    && targetConversationBusy
  );
}
