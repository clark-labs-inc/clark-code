import { useSessionStore } from "../../store/sessionStore";
import { projectDisplayName } from "../../lib/projectSidebar";
import { useSpecialistStore } from "../../store/specialistStore";
import type { SpecialistKind } from "../../lib/specialists";

/** Detach from ongoing work and put keyboard input in the new task composer. */
export function newConversation(nextProjectLabel?: string) {
  const state = useSessionStore.getState();
  const runningCheckout = state.session && state.runningIds.includes(state.session.id)
    ? state.activeProjectRoot?.trim() || state.localSettings.cwd.trim()
    : null;
  const alreadyAtStart = !state.session && !state.opening;
  state.endSession();

  if (runningCheckout && nextProjectLabel) {
    state.flashNotice(
      `Started a new session in ${nextProjectLabel}. ${projectDisplayName(runningCheckout)} is still running in the sidebar.`,
    );
  } else if (alreadyAtStart) {
    state.flashNotice(nextProjectLabel
      ? `New session in ${nextProjectLabel}. Type a message to begin.`
      : "New session ready. Type a message to begin.");
  }

  // React must first replace the previous workspace with the start composer.
  requestAnimationFrame(() => {
    document.querySelector<HTMLTextAreaElement>("textarea.composer-input")?.focus();
  });
}

/** Start a fresh conversation in the chosen lens, without carrying its old workflow or draft. */
export function newSpecialistConversation(kind: SpecialistKind) {
  newConversation();
  useSpecialistStore.getState().open(kind);
}
