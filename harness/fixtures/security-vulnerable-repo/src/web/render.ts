export function renderProfile(displayName: string) {
  // Vulnerable: stored profile data becomes raw executable markup.
  return `<section class="profile">${displayName}</section>`;
}
