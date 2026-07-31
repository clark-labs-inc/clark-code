/** Mirror of the composer textarea's current draft. The Composer writes this on
 *  every edit so store actions can read the unsent text without making the draft
 *  itself reactive — typing must not re-render the whole store. `endSession`
 *  stages it as a `composerPrefill` to carry a half-typed message across the
 *  composer remount that starting a new session forces: the active-session
 *  composer unmounts and a fresh start-screen composer mounts, which would
 *  otherwise drop the local `useState` text. Non-reactive by design — the
 *  textarea remains the source of truth. */
export const composerDraftRef: { current: string } = { current: "" };
