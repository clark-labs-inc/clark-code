# Attachment thumbnail design QA

- Source visual truth: `/tmp/codex-remote-attachments/019f72ef-b16e-7b53-96c4-1f3e3b9a309b/D96318F6-CC4C-459B-A64C-8D6DBF9CFFC7/1-Photo-1.jpg`
- Implementation screenshot: `/tmp/clark-desktop-attachment-smoke.png`
- Full-view comparison: `/tmp/clark-attachment-full-comparison.png`
- Focused comparison: `/tmp/clark-attachment-focused-comparison.png`
- Viewport: default Codex in-app Browser viewport, captured at `1280 x 720`
- State: authenticated local mock session, light theme, composer with one compacted large-text paste and one image attachment

## Full-view comparison evidence

The mobile reference and desktop implementation were placed in one combined
image. The desktop intentionally keeps Clark's existing paper palette, type,
controls, and wider workspace composition. Within that product shell, the
reference behavior is preserved: attachments occupy a compact rail above the
message field, show a thumbnail or file icon, truncate long names, expose a
small remove button, and do not make the composer grow as more items are added.

## Focused region comparison evidence

The composer regions were cropped and placed in one combined image at a common
width. Both use a single row of 40px attachment chips with compact media,
filename, and close affordances. The implementation uses Clark's existing DM
Sans typography and light surface tokens instead of copying the reference's
iOS dark theme. This is an intentional product-system adaptation, not design
drift.

## Required fidelity surfaces

- Fonts and typography: existing Clark DM Sans hierarchy is unchanged; chip
  labels use the established compact 12px medium treatment and truncate rather
  than wrap.
- Spacing and layout rhythm: the rail stays 42px high with 8px gaps. With five
  items its `clientWidth` was 646px and `scrollWidth` was 909px, proving
  horizontal overflow without a second row.
- Colors and visual tokens: the implementation uses the existing
  `bg-bg-tertiary`, `bg-bg-sunken`, and ink tokens, preserving light/dark theme
  compatibility.
- Image quality and asset fidelity: image attachments use their real object URL
  thumbnails with `object-cover`; non-image and pasted-text attachments use the
  existing Lucide file icon. No placeholder raster or handcrafted icon was
  introduced.
- Copy and content: original filenames remain visible; large clipboard text is
  labeled `[Pasted Content N chars]`, with collision-free numbering for repeated
  pastes. Its full text remains typed pending-paste state and expands into
  ordinary message text on submit rather than becoming a `.txt` upload.

## Findings

- No actionable P0, P1, or P2 differences remain. Theme and overall viewport
  differences are intentional because the screenshot is a mobile interaction
  reference being applied inside the existing desktop design system.

## Comparison history

- Pass 1: no P0/P1/P2 finding. The focused combined comparison confirmed the
  attachment density, ordering, thumbnail scale, truncation, and close-control
  placement without requiring a visual correction.

## Interaction and runtime evidence

- A 1,041-character clipboard payload became `Pasted Content 1041 chars`; the
  textarea remained empty.
- The reference image loaded as a real thumbnail chip alongside the paste chip.
- Removing the paste chip removes exactly that typed pending paste.
- WebKit submission proved the full paste reached the user turn as ordinary
  text, with no placeholder leakage and no page errors.
- Frontend typecheck, all 145 tests, the production Vite build, and the focused
  attachment smoke passed.

## Follow-up polish

- None required for this scope.

final result: passed
