# Project actions menu design QA

- Source visual truth: `/Users/stan/Desktop/Screenshot 2026-07-18 at 1.52.17 PM.png`
- Clark sidebar context: `/Users/stan/Desktop/Screenshot 2026-07-18 at 1.51.31 PM.png`
- Implementation screenshot: `/tmp/clark-desktop-project-menu-open.png`
- Focused implementation crop: `/tmp/clark-desktop-project-menu-focused.png`
- Combined comparison: `/tmp/clark-desktop-project-menu-comparison.png`
- New-session implementation screenshot: `/tmp/clark-desktop-project-new-session.png`
- New-session comparison: `/tmp/clark-desktop-project-new-session-comparison.png`
- Viewport: `1280 x 720`
- State: authenticated local mock workspace, light theme, remembered `clark-desktop` project, project actions menu open

## Full-view comparison evidence

The browser-rendered implementation was captured in Clark Desktop's complete
workspace. The project overflow trigger sits at the right edge of the project
row, and its fixed popover opens to the right of the sidebar without clipping
the sidebar, conversation workspace, or persistent composer controls.

## Focused comparison evidence

The dark reference and the Clark implementation crop were placed in one
combined image. Both use a compact six-action vertical menu aligned beside the
project row, with the same order and copy: Pin project, Reveal in Finder,
Create permanent worktree, Rename project, Archive chats, and Remove. Standard
icons communicate each action; Remove uses the reference's X rather than a
destructive-delete icon. Clark intentionally retains its light paper palette,
DM Sans typography, existing radii, shadows, and surface tokens.

The project row also reproduces the reference's second trailing control with
the installed Lucide `SquarePen` icon. Its accessible label and tooltip read
"New session in clark-desktop". The fresh comparison places the source
and Clark's focused sidebar state side by side; the ellipsis then note-and-pen
ordering, scale, spacing, and right-edge alignment match the reference while
retaining Clark's existing light-theme tokens.

## Required fidelity surfaces

- Fonts and typography: Clark's existing DM Sans UI type remains intact. Menu
  labels render at the established 14px size with a compact 32px row height,
  no wrapping, truncation, or optical-weight mismatch.
- Spacing and layout rhythm: the 240px popover, 6px trigger gap, 6px inner
  padding, 32px action rows, 12px radius, and lifted shadow preserve the source
  density while fitting Clark's existing sidebar scale.
- Colors and visual tokens: the component uses `bg-bg-elevated`, `bg-bg-hover`,
  ink, border, accent, and danger tokens, so it adapts correctly to Clark's
  light/dark themes instead of hard-coding the source screenshot's dark colors.
- Image quality and asset fidelity: the reference contains no raster imagery.
  All controls use the installed Lucide icon library; no handcrafted SVG, CSS
  drawing, text glyph, or placeholder asset was introduced.
- Copy and content: all six reference labels are reproduced exactly. The open
  worktree, rename, and remove subflows add concise product-specific guidance
  without changing the main menu copy.

## Findings

- No actionable P0, P1, or P2 differences remain. Theme, sidebar width, and
  background conversation content differ intentionally because the reference
  interaction is adapted to Clark Desktop's existing product system.

## Comparison history

- Pass 1: the menu matched layout, copy, ordering, and placement. A P3 icon
  polish issue used a trash can for Remove; it was replaced with the reference's
  X icon.
- Pass 2: the revised focused comparison confirmed the X icon and found no
  actionable P0/P1/P2 differences.

## Interaction and runtime evidence

- The overflow trigger exposes `aria-haspopup="menu"` and toggles its expanded
  state. Outside click and Escape dismiss the menu.
- Pin changes to Unpin and persists; rename updates the project label; both were
  restored after the interaction test.
- Worktree creation enables only after a name is entered. Browser preview shows
  the expected desktop-only fallback; the native command is covered separately
  by a real temporary Git-repository test.
- Remove opens an explicit confirmation explaining that files are untouched;
  the test cancelled before mutation.
- The note-and-pen control was clicked in the rendered browser preview. It
  returned to the clean session composer with `clark-desktop` selected as the
  project, confirming both the new-session reset and project scoping.
- No browser console errors were emitted. The only warnings were the existing
  Motion reduced-motion notices from the test device.

## Follow-up polish

- None required for this scope.

final result: passed
