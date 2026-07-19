# Artifact Workspace Design QA

- Source visual truth: `/Users/stan/.codex/generated_images/019f5ee9-6d53-7263-9224-a042c3f63857/exec-88beda04-3ef7-49ac-accc-f478c1d424e4.png`
- Implementation screenshot: `.design-qa/artifact-workspace-implementation-verified.png`
- Full-view comparison: `.design-qa/artifact-workspace-full-comparison-verified.png`
- Focused comparison: `.design-qa/artifact-workspace-focused-comparison-verified.png`
- Narrow responsive capture: `.design-qa/artifact-workspace-narrow.png`
- Viewport: 1440 × 1024 for the primary comparison; 900 × 800 for responsive source navigation.
- State: light Clark theme, artifact-producing task complete, Markdown artifact selected, Source context open.

## Findings

No actionable P0, P1, or P2 differences remain. The implementation preserves the selected mockup's conversation-plus-reader composition, artifact tabs, context rail, source popover, and restrained Clark visual language.

- Fonts and typography: passed. Existing DM Sans, Newsreader, and JetBrains Mono roles match the source hierarchy; the document reader uses the editorial face for headings without changing global UI type.
- Spacing and layout rhythm: passed. The desktop split now holds chat to 25rem, leaving the artifact reader as the dominant workspace. At widths below 1280px, the reader becomes the primary pane instead of crushing both surfaces.
- Colors and visual tokens: passed. The implementation reuses Clark's warm paper, graphite, violet selection, border, and semantic status tokens. Repeated inline actions were reduced to neutral controls so the conversation stays quiet.
- Image quality and asset fidelity: passed. The target contains no custom raster assets that need recreation; existing Lucide interface icons remain crisp and consistent.
- Copy and content: passed. Tabs, descriptive external actions, source labels, availability, exact Markdown byte size, and the generating tool title are visible. The mock's invented creation timestamps were intentionally not fabricated because the current artifact contract does not provide them.
- Accessibility and interaction: passed. Tabs, picker, presentation toggle, details/source panels, and close controls have accessible names. “Jump to source message” reaches the generating tool call and reveals chat automatically at narrow widths. Reduced motion remains respected.

## Interaction Verification

- Opened the workspace from an inline Markdown artifact.
- Switched to an image artifact and verified its richer availability fallback.
- Opened Details, Source, and the artifact picker.
- Toggled Present back to Read.
- Verified narrow-window source navigation closes the reader and reveals the generating tool call.
- Browser console had no application errors; only the expected reduced-motion development warning was present.

## Comparison History

1. Initial P2: the chat pane was about 90px wider than the selected mockup, squeezing the artifact toolbar and wrapping metadata.
2. Fix: changed the desktop split to a fixed 25rem chat pane and made the artifact pane dominant; below 1280px the reader uses the whole content area.
3. Initial P2: repeated violet View buttons made inline artifacts louder than the target.
4. Fix: moved repeated artifact actions to neutral Clark surfaces while keeping violet for selection and the primary source-jump action.
5. Post-fix evidence: `.design-qa/artifact-workspace-full-comparison-verified.png` and `.design-qa/artifact-workspace-focused-comparison-verified.png`. No actionable P0/P1/P2 mismatch remains.

## Follow-up Polish

- P3: extend the cross-provider artifact contract with created/updated timestamps and revision identifiers so the Details and Versions panels can show real history instead of only current availability.

final result: passed

---

# Simplified Parallel Work Design QA

- Source visual truth: `.design-qa/multi-agent-collapsed.png` and `.design-qa/multi-agent-expanded.png`
- Implementation screenshots: `.design-qa/multi-agent-simplified-collapsed.png` and `.design-qa/multi-agent-simplified-expanded.png`
- Full-view comparisons: `.design-qa/multi-agent-simplified-collapsed-comparison.png` and `.design-qa/multi-agent-simplified-expanded-comparison.png`
- Viewport: 1280 x 720 for every source and implementation capture
- State: dark theme, local mock session complete, three parallel parts ready; collapsed and expanded

## Findings

No actionable P0, P1, or P2 differences remain. The implementation intentionally reduces explanatory chrome while preserving the accepted card, status receipt, and disclosure interaction.

- Fonts and typography: passed. The shorter `Parallel work` heading and existing 12px supporting status preserve Clark's compact hierarchy and remove redundant copy without changing the type system.
- Spacing and layout rhythm: passed. The collapsed card keeps the same footprint and alignment. The expanded state removes the introductory and footer rows, leaving a cleaner three-row task list with consistent separators, padding, and status alignment.
- Colors and visual tokens: passed. The standalone violet orchestration icon, one-pixel completion receipt, graphite surface, quiet border, and green ready states remain within the existing Clark token system.
- Image quality and asset fidelity: passed. No raster or custom illustrative asset is required; the orchestration and disclosure marks remain crisp Lucide icons.
- Copy and content: passed. `Parallel work` and `All parts are ready` communicate the outcome directly. The task list appears only on expansion, without model names, worktree terminology, or orchestration explanation.
- Accessibility and interaction: passed. The whole summary remains one button with `aria-expanded`; its accessible name includes the heading and status, and the chevron communicates disclosure without a redundant visible Details/Hide label.

## Full-view Comparison Evidence

The collapsed combined image shows the accepted source on the left and the simplified implementation on the right at identical scale. The revised card preserves its placement, size, border, status, and progress receipt while reducing the header to the essential label and chevron. The expanded combined image shows the same shell and state; only the redundant explanation and footer have been removed.

## Focused Region Comparison Evidence

A separate crop was not needed because each card remains fully legible at original resolution in the 2560 x 720 combined comparisons. Both the header controls and all three expanded task/status rows can be judged directly without rescaling.

## Comparison History

1. The accepted source used `Clark is working in parallel`, a visible Details/Hide label, an icon tile, an explanatory expanded intro, three work rows, and a footer explanation.
2. The requested simplification shortened the heading, removed the visible disclosure label and icon tile, and limited the expanded body to the three work rows and their statuses.
3. Post-change evidence: `.design-qa/multi-agent-simplified-collapsed-comparison.png` and `.design-qa/multi-agent-simplified-expanded-comparison.png`. No corrective P0/P1/P2 iteration was required.

## Interaction and Runtime Evidence

- Verified the collapsed button exposes `Parallel work All parts are ready` and expands to the three typed work rows.
- Verified the expanded card collapses from the same control.
- Browser console errors: none. The only warning was the expected reduced-motion development notice.
- Frontend typecheck passed.
- The complete Vitest suite passed: 36 files and 183 tests.
- Production Vite build passed.

## Follow-up Polish

- No P3 visual change is recommended; further reduction would start hiding useful progress detail.

final result: passed

---

# Parallel Work Extra-Line Removal Design QA

- Source visual truth: `/tmp/codex-remote-attachments/019f733c-78c5-7d21-8533-bbf2cd9cfa31/179D6D0D-47A4-497C-8918-C1B31689F35D/1-Photo-1.jpg` and `.design-qa/multi-agent-simplified-expanded.png`
- Implementation screenshot: `.design-qa/multi-agent-no-extra-line-expanded.png`
- Full-view comparison: `.design-qa/multi-agent-no-extra-line-comparison.png`
- Viewport: 1280 x 720 for the normalized before/after comparison; the supplied phone image is a portrait crop of the same expanded state
- State: dark theme, completed parallel work, details expanded, summary button focused

## Findings

No actionable P0, P1, or P2 differences remain. The unwanted violet horizontal line is absent while the card's structure, content, dimensions, and expand behavior remain unchanged.

- Fonts and typography: passed. Heading, supporting status, task labels, and ready states are unchanged.
- Spacing and layout rhythm: passed. Removing the line did not alter the summary height, task-row spacing, card radius, or surrounding conversation layout.
- Colors and visual tokens: passed. The clipped violet line is gone. Keyboard focus now uses the existing quiet hover background token across the summary instead of drawing a partial outline.
- Image quality and asset fidelity: passed. No imagery or icon asset changed; the existing Lucide orchestration, disclosure, and status icons remain crisp.
- Copy and content: passed. `Parallel work`, `All parts are ready`, and the three task/status rows are unchanged.
- Accessibility and interaction: passed. The summary still exposes `aria-expanded=true` and retains a visible keyboard-focus cue through its background state without the stray line.

## Full-view Comparison Evidence

The combined comparison places the prior expanded state on the left and the revised state on the right at the same viewport. The only structural removal is the horizontal progress divider. In the focused state, the global outline no longer clips into a line at the bottom edge of the summary.

## Focused Region Comparison Evidence

The full-view comparison renders the card large enough to inspect the exact summary/task boundary, so a separate crop is unnecessary. The removed line and preserved task spacing are directly legible at original resolution.

## Comparison History

1. Initial P2: the supplied image and previous implementation showed a bright violet horizontal line between the summary and task rows.
2. First fix: removed the dedicated progress-divider element. The first post-fix capture showed that a second line persisted when the summary button was focused.
3. Root fix: the global focus outline was being clipped by the card's `overflow-hidden` boundary. The toggle now suppresses that partial outline and uses a quiet full-summary background as its focus cue.
4. Post-fix evidence: `.design-qa/multi-agent-no-extra-line-expanded.png` and `.design-qa/multi-agent-no-extra-line-comparison.png`. No actionable P0/P1/P2 issue remains.

## Interaction and Runtime Evidence

- Ran the local mock through the ordinary composer and expanded the completed Parallel Work card.
- Verified the button remains expanded and its computed outline style is `none` in the focused state.
- Browser console errors: none.
- Frontend typecheck passed.
- The complete Vitest suite passed: 36 files and 183 tests.
- Production Vite build passed.

## Follow-up Polish

- None recommended for this focused change.

final result: passed

---

# Simple Multi-Agent Progress Design QA

- Source visual truth: `.design-qa/settings/implementation-pass-2.jpg` (Clark Desktop's selected dark-shell, card-density, typography, and token reference)
- Collapsed implementation: `.design-qa/multi-agent-collapsed.png`
- Expanded implementation: `.design-qa/multi-agent-expanded.png`
- Full-view comparison: `.design-qa/multi-agent-design-comparison.png`
- Focused comparison: `.design-qa/multi-agent-focused-comparison.png`
- Viewport: 1280 x 720 for both source and implementation captures
- State: dark theme, local mock session complete, three parallel parts ready; details collapsed and expanded

## Full-view comparison evidence

The source is a different Clark workflow, so the comparison is intentionally
limited to the existing product shell and design language rather than claiming
component-for-component fidelity. In the combined image, the implementation
preserves the same sidebar proportion, centered content width, warm graphite
surfaces, quiet borders, compact hierarchy, and restrained violet selection.
The new card reads as part of Clark rather than a separate orchestration
dashboard.

## Focused comparison evidence

The focused comparison places Clark's existing settings cards beside the
expanded multi-agent card at a common scale. Row height, separator weight,
corner radius, muted supporting text, semantic status treatment, and control
density remain consistent. The orchestration surface adds no bespoke imagery;
it uses the installed Lucide icon set and existing tokens.

## Findings

No actionable P0, P1, or P2 differences remain for this vertical slice.

- Fonts and typography: passed. The card uses the existing DM Sans hierarchy: 14px primary copy and compact 12px status/supporting copy, with no new display treatment.
- Spacing and layout rhythm: passed. The collapsed card stays one compact 58px summary surface; expanded rows use the same quiet density, separators, radius, and centered conversation width as adjacent Clark components.
- Colors and visual tokens: passed. Background, hover, border, ink, accent, success, and danger states all use Clark tokens; violet is limited to the icon tint and progress receipt.
- Image quality and asset fidelity: passed. No raster or custom decorative asset is required; the Network, status, and disclosure icons come from the installed Lucide library.
- Copy and content: passed. Default copy explains the outcome in nontechnical language; agent names, model details, tokens, worktrees, and logs are hidden until details are requested.
- Accessibility and interaction: passed. The whole summary is a single button with `aria-expanded`; Details/Hide tracks the state, and reduced motion is respected.

## Comparison history

1. Initial source implementation was a prominent swarm card with a large accent block, a visible progress bar, numbered agent tiles, and technical copy such as “fanned out” and “merging.”
2. Fix: replaced it with a single quiet summary row, plain-language status, a one-pixel progress receipt, and details collapsed by default.
3. Post-fix evidence: `.design-qa/multi-agent-collapsed.png`, `.design-qa/multi-agent-expanded.png`, and `.design-qa/multi-agent-focused-comparison.png`. No actionable P0/P1/P2 issue remains.

## Interaction and runtime evidence

- Ran the local browser mock through the ordinary composer using “Build this feature in parallel, then review and verify it.”
- Verified the completed receipt remains visible, opens to three typed child rows, and closes from the same control.
- Browser console errors: none. The only warning was the expected reduced-motion development notice.
- Frontend typecheck and the complete 182-test Vitest suite passed before capture.

## Follow-up polish

- P3: once retry and failure recovery are exposed in a live model run, capture the “Needs attention” and safe rework states in the same shell.

final result: passed

---

# Streamlined Settings Design QA

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_p3n2ml/Screenshot 2026-07-18 at 1.09.06 PM.png`
- Implementation screenshot: `.design-qa/settings/implementation-pass-2.jpg`
- Full-view comparison: `.design-qa/settings/full-comparison-pass-2.png`
- Focused comparison: `.design-qa/settings/focused-comparison-pass-2.png`
- Viewport: 1280 x 720 implementation; the 3427 x 1310 Codex reference was normalized to the implementation width for comparison.
- State: dark theme, General settings selected, default text size, Approve for me permission mode.

## Findings

No actionable P0, P1, or P2 differences remain for the requested Settings redesign scope.

- Information architecture: passed. Settings now use a full-window shell with a persistent grouped rail, global search, compact section labels, and one centered content column. Clark-specific areas are grouped into Personal, Workspace, Extensions, and System rather than copying irrelevant Codex sections.
- Typography and spacing: passed. Section headings, descriptions, labels, supporting copy, card rows, and navigation use a restrained hierarchy with consistent row density and readable line lengths.
- Surfaces and state: passed. Cards use quiet solid surfaces and separators; active navigation and selected rows are neutral, with violet reserved for the selected control itself.
- Interaction: passed. Search filters sections by labels, descriptions, and keywords; filtered navigation opens the target section; Back to app and Escape close Settings; large text reflows without horizontal page overflow.
- Accessibility: passed for inspected behavior. The shell is exposed as a named modal dialog, search has an accessible label, text-size choices use a radio group, toggles are switches with enlarged hit targets, and navigation remains keyboard-addressable.

## Comparison History

1. Initial P1: Clark used a centered modal with a narrow tab strip, weak section grouping, and a form layout that felt denser and less navigable than the Codex full-window settings surface.
2. First implementation: introduced the full-window shell, persistent grouped navigation, settings search, centered content column, simplified section labels, solid cards, and larger control hit targets.
3. P1 refinement: the first combined comparison showed selected permission and output rows still reading as violet panels. Their fills were changed to neutral foreground tints so the accent is reserved for the selected radio control.
4. Post-fix evidence: `.design-qa/settings/full-comparison-pass-2.png` and `.design-qa/settings/focused-comparison-pass-2.png`. No actionable P0/P1/P2 differences remain.

## Verification

- TypeScript typecheck passed.
- 32 Vitest files and 170 tests passed.
- Production Vite build passed.
- Search, section navigation, Back to app, Escape close, and large-text reflow passed at 1280 x 720.
- Large text produced no horizontal document overflow.
- Browser console errors: none.

## Follow-up Polish

- P3: verify the same information density in the packaged Windows and Linux WebViews at 100% and 125% display scaling before release.

final result: passed

---

# Codex-like Typography and Spacing Design QA

- Source visual truth: `/tmp/codex-remote-attachments/019f72e1-2841-73c1-8058-7b131ccb3187/AAB29689-95AB-4E2B-969C-E7B24A1EB557/1-Photo-1.jpg`
- Original Clark reference: `/tmp/codex-remote-attachments/019f72e1-2841-73c1-8058-7b131ccb3187/AAB29689-95AB-4E2B-969C-E7B24A1EB557/2-Photo-2.jpg`
- Implementation screenshot: `/tmp/clark-type-qa/clark-dark-transcript-final.jpg`
- Full-view comparison: `/tmp/clark-type-qa/codex-vs-clark-final-full.jpg`
- Focused text comparison: `/tmp/clark-type-qa/codex-vs-clark-final-text.jpg`
- Viewport: 1280 × 755.
- State: Codex light-theme completed transcript compared with Clark Code dark-theme completed demo transcript. Content and theme intentionally differ; typography scale, hierarchy, line measure, and spacing rhythm are the fidelity targets.

## Findings

No actionable P0, P1, or P2 differences remain for the requested typography and spacing scope.

- Fonts and typography: passed. Clark keeps its existing DM Sans product face and Newsreader display face, but the desktop scale now uses an 11px / 12px / 13px UI hierarchy, regular 400 body weight, 13px assistant copy, 20.8px answer line height, and normal letter spacing. This matches the compact, crisp hierarchy in the Codex reference without copying its light theme or font family.
- Spacing and layout rhythm: passed. The conversation, start screen, environment controls, queued messages, and composer now share a 42rem maximum reading width. Transcript blocks have a consistent 16px gap; Markdown paragraphs and lists use 10px separation; headings receive a clearer 16px lead-in. The cold-loaded composer is 78px tall instead of the original oversized card.
- Colors and visual tokens: passed for scope. Clark intentionally retains its black surfaces, violet interaction state, and current contrast tokens; the white Codex palette was not copied.
- Image quality and asset fidelity: passed. Neither reference requires new raster assets, logos, or illustrations. Existing library icons remain crisp at the smaller scale.
- Copy and content: passed for scope. Clark-specific plan, tool, project, permission, and model copy remains unchanged. The demo transcript differs from the supplied Codex transcript, so textual line breaks were assessed by scale and line measure rather than exact sentence matching.
- Accessibility and interaction: passed for the inspected states. New session submission, the mock run, dark theme, and the settled composer were exercised at the target viewport. The browser console reported no errors. Screenshot evidence alone does not prove every OS text-scaling or keyboard-focus combination.

## Comparison History

1. Initial P1: the supplied Clark screen and source used a 16px base scale with a 450 default weight, making navigation and transcript text materially larger and heavier than Codex. A later density pass had tightened gaps around that enlarged type, producing a heavy-but-cramped rhythm.
2. First fix: reduced the base to 14px/400, narrowed the reading column, restored Markdown section spacing, and reduced the start-screen headline. The first rendered comparison found a remaining P2: Clark's body copy and 101px composer were still visibly larger than the Codex reference.
3. Final fix: reduced the desktop base to 13px, aligned the supporting UI steps to 11px and 12px, tightened answer line height to 1.6, and reduced the cold-loaded composer to 78px with a consistent 32px action row.
4. Post-fix evidence: `/tmp/clark-type-qa/codex-vs-clark-final-full.jpg` and `/tmp/clark-type-qa/codex-vs-clark-final-text.jpg`. The final dark Clark screen now has comparable compactness, optical weight, and vertical rhythm without losing its own theme.

## Verification

- TypeScript typecheck passed.
- 27 Vitest files and 143 tests passed.
- Production Vite build passed.
- Demo conversation submission and completion passed at 1280 × 755.
- Browser console errors: none.

## Follow-up Polish

- P3: evaluate the final scale on Windows WebView at 100% and 125% display scaling before the next desktop release.

final result: passed

---

# Sidebar Design QA

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_R3OM3B/Screenshot 2026-07-13 at 8.43.09 PM.png`
- Original Clark capture: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_dRTQ0e/Screenshot 2026-07-13 at 8.42.41 PM.png`
- Implementation screenshot: `.design-qa/sidebar-implementation.png`
- Full-view comparison: `.design-qa/sidebar-full-comparison.png`
- Focused comparison: `.design-qa/sidebar-focused-comparison.png`
- Viewport: 1307 × 849 implementation; the Codex reference is a sidebar-only 516 × 1275 capture, so the component was normalized by visible sidebar width for comparison.
- State: light Clark theme, populated project groups, one opening conversation, one running conversation, archived section collapsed.

## Findings

No actionable P0, P1, or P2 differences remain. Clark keeps its own light theme and product identity while matching the reference's compact hierarchy and interaction density.

- Fonts and typography: passed. The sidebar now uses a restrained 13px local UI scale, 15px product label, normal-case project names, and medium weights instead of the global enlarged scale and uppercase tracking.
- Spacing and layout rhythm: passed. Rows use a consistent 28px height, 8px radii, narrow gutters, compact project grouping, and a 32px account footer.
- Colors and visual tokens: passed. The neutral selected and hover states use the existing Clark background tokens; accent color is reserved for running and selected status instead of filling the primary action.
- Image quality and asset fidelity: passed. There are no raster assets in the target sidebar; existing Lucide UI icons remain crisp and consistently sized.
- Copy and content: passed. Clark-specific labels and conversation data are preserved, while the hierarchy now follows the reference's product label, new-session action, Projects label, project groups, conversations, archived section, and compact account footer.

## Interaction Verification

- Search icon opens the existing conversation filter.
- Filtering to `compaction` leaves only the matching project and row.
- Escape clears the query and closes search.
- Collapse reduces the sidebar to the icon rail; Expand restores the full sidebar.
- Browser console had no application errors. The only warning was the expected reduced-motion environment notice.

## Comparison History

1. Initial P1: the original Clark sidebar used an oversized gradient CTA, uppercase tracked project headers, timestamps squeezed into every row, icon-heavy conversation rows, large rounded surfaces, and a two-line account footer. Together these made the sidebar materially denser and less scannable than the Codex reference.
2. Fixes: replaced the gradient CTA with a compact navigation row, moved search to a header control with a focused inline field, added the Projects hierarchy, normalized project labels, removed idle row icons and timestamps, changed selected/hover states to neutral surfaces, tightened the rail and footer, and preserved live/opening status indicators.
3. Post-fix evidence: `.design-qa/sidebar-focused-comparison.png` and `.design-qa/sidebar-implementation.png`. No actionable P0/P1/P2 mismatches remain.

## Follow-up Polish

- P3: evaluate the same density in dark mode after the light-theme change has been used in the installed app; no dark-mode blocker was observed from token usage.

final result: passed

---

# Composer Add Menu Design QA

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_l7sDKH/Screenshot 2026-07-18 at 9.44.28 PM.png`
- Light implementation screenshot: `.design-qa/composer-add-menu-light.png`
- Dark implementation screenshot: `.design-qa/composer-add-menu-dark.png`
- Focused source/implementation comparison: `.design-qa/composer-add-menu-comparison.png`
- Viewport: 1280 × 720 implementation; the 797 × 358 Codex reference was normalized into a focused component comparison.
- State: local Clark Code conversation, Add menu open above the composer; verified before the first turn and after a session starts.

## Findings

No actionable P0, P1, or P2 differences remain for the attachment workflow. Clark intentionally limits the menu to working attachment sources instead of copying unavailable Codex goals and plugin entries.

- Fonts and typography: passed. The menu uses Clark's existing DM Sans scale, with a compact `Add` label, 13px actions, and 12px inline hints matching the reference hierarchy.
- Spacing and layout rhythm: passed. The final 640px menu follows the composer's width, opens upward from the plus button, uses compact 36px rows, and matches the reference's rounded full-width sheet treatment.
- Colors and visual tokens: passed. Light and dark captures reuse Clark's elevated, hover, border, ink, and shadow tokens; the menu does not introduce a bespoke surface palette.
- Image quality and asset fidelity: passed. No raster assets are required. File and folder actions use the repository's established Lucide interface icon system and remain crisp at 16px.
- Copy and content: passed. `Files` and `Folder` expose the two working attachment sources with concise descriptions. Codex-only goal, plugin, and product-attachment entries were not fabricated.
- Accessibility and interaction: passed. The plus button exposes `aria-haspopup`, `aria-expanded`, and a stable accessible name. The menu has named menu items, closes on outside click and Escape, and returns focus to its trigger.

## Interaction Verification

- Confirmed Add is enabled before any conversation exists.
- Opened and dismissed the menu with both pointer and Escape behavior.
- Selected a text file through the real file-chooser flow and saw its attachment chip.
- Sent an attachment-only first turn; session creation succeeded and the staged chip cleared after dispatch.
- Selected a folder through the directory chooser and saw its contained file staged.
- Confirmed Add remains enabled after a session starts.
- Verified light and dark menu renders and checked the browser console: no errors.

## Comparison History

1. Initial P1: the existing plus button was disabled before a session existed, so the first-turn attachment workflow was impossible and no Codex-style Add menu appeared.
2. First implementation restored first-turn attachment staging and introduced working Files and Folder menu actions. The initial 320px stacked-label capture was a P2 density mismatch against the wide, inline Codex reference.
3. Final fix widened the menu to 640px and moved hints inline, preserving Clark's composer width while matching the reference's sheet proportions and row rhythm.
4. Post-fix evidence: `.design-qa/composer-add-menu-comparison.png`, `.design-qa/composer-add-menu-dark.png`, and `.design-qa/composer-add-menu-light.png`. No actionable P0/P1/P2 issue remains.

## Follow-up Polish

- P3: add arrow-key roving focus if the Add menu grows beyond the current two actions.

final result: passed
