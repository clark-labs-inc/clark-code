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

# Compact Clark Cloud Agent Design QA

- Source visual truth: `/Users/stan/.codex/generated_images/019f7906-2c3d-7d41-a300-e8ca04fb846d/exec-5983220d-a430-4140-847d-53b547f0cb34.png`
- Live implementation: `.design-qa/cloud-agent-live.png`
- Completed expanded implementation: `.design-qa/cloud-agent-completed-expanded.png`
- Completed collapsed implementation: `.design-qa/cloud-agent-completed-collapsed.png`
- Full-view comparison: `.design-qa/cloud-agent-full-comparison.png`
- Focused comparison: `.design-qa/cloud-agent-focused-comparison.png`
- Viewport: 1440 × 1024 for source normalization and every implementation capture.
- State: dark Clark Desktop browser mock; live Clark Cloud research, completed cited findings, and collapsed completed receipt.

## Findings

No actionable P0, P1, or P2 differences remain. The implementation preserves the selected mock's Clark Cloud identity and research-process hierarchy while applying the requested simplification and using only data the current tool contract can prove.

- Fonts and typography: passed. The implementation uses Clark's existing DM Sans and JetBrains Mono roles, keeps the cloud identity at the conversation's normal 14 px hierarchy, and maintains readable 11–12 px process labels without clipping.
- Spacing and layout rhythm: passed. The live surface is about one-third the selected mock's height and fits the existing `max-w-2xl` conversation column. The completed receipt collapses to a compact identity-and-action surface while the expanded findings remain bounded and scrollable.
- Colors and visual tokens: passed. Warm graphite surfaces, quiet borders, off-white text, and existing violet accent tokens match the source. Violet remains limited to the live receipt, process motion, focus, and primary brief action.
- Image quality and asset fidelity: passed. The implementation reuses Clark's official open-C mark component and the project's installed interface icon system. No generated raster asset or placeholder is needed for this product-native surface.
- Copy and content: passed. `Clark Cloud Agent` and `Running securely on clarkchat.com` make provenance unmistakable. The live process names planning, web search, source reading, and cited synthesis. Completed source counts are derived from returned citations; the mock's invented thread and line counters are intentionally omitted because the current typed tool contract does not expose them.
- Accessibility and interaction: passed. The research surface is a named region, the disclosure reports `aria-expanded`, the live state auto-opens, the completed state collapses, and `View research brief` reopens the cited findings. Source actions retain titles and accessible labels.

## Full-view Comparison Evidence

The combined image places the selected compact mock beside the real live and completed Clark Desktop states at the same viewport. The implementation is intentionally narrower because it follows the production conversation column instead of enlarging the app content region. Composer, surrounding work rows, and conversation hierarchy remain stable in both states.

## Focused Region Comparison Evidence

The focused comparison shows the source component, live implementation, and completed implementation at a common height. Identity, provenance, phase hierarchy, violet restraint, corner treatment, and code typography remain legible. The implementation removes the source's two nested work rows and duplicate scale strip, which is the requested space-efficiency change rather than design drift.

## Comparison History

The first rendered comparison had no actionable P0/P1/P2 mismatch. No corrective visual iteration was required after QA. The intentional contract-driven differences are the omitted unverified telemetry and the separation of live process state from completed findings.

## Interaction and Runtime Evidence

- Submitted the xterm.js research scenario through the ordinary Clark Desktop composer.
- Verified the live surface automatically expanded with `Live`, the four research phases, and the exact clarkchat.com provenance line.
- Verified completion replaced live process detail with two real cited sources.
- Collapsed the completed result and reopened it through `View research brief`; `aria-expanded` returned to `true`.
- Browser application errors: none. The only console output was the expected reduced-motion development warning.
- Frontend tests passed: 40 files and 204 tests.
- Frontend TypeScript typecheck passed.

## Follow-up Polish

- P3: if the cloud research contract later exposes typed nested-agent activity or reviewed-source counts, the header has room for one quiet real-time scale receipt without restoring the mock's larger dashboard footprint.

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

---

# Composer checkout strip design QA

## Evidence

- Source visual truth:
  - `/Users/stan/Desktop/Screenshot 2026-07-18 at 10.05.35 PM.png` — compact independent metadata pills above the composer.
  - `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_4H5arg/Screenshot 2026-07-18 at 10.07.10 PM.png` — light-mode oversized shared-cap state to remove.
- Rendered implementation:
  - `/tmp/clark-context-dark-running-after.jpeg`
  - `/tmp/clark-context-light-running-after.jpeg`
- Combined normalized comparison: `/tmp/clark-context-final-comparison.jpeg`
- Viewport: 1179 × 768 native macOS app window; composer regions normalized to 1179 px wide in the combined comparison.
- State: authenticated desktop app, existing local conversation, active model run, local checkout on `main`; verified in dark and light themes.

## Full-view comparison evidence

The native captures show the strip centered to the same 672 px content column as the composer. It remains visible during an active run without overlapping the timeline, composer, sidebar, or Stop control. Dark and light theme surfaces retain clear separation without a shared border or cap.

## Focused region comparison evidence

The combined comparison places each source composer region directly above its rendered counterpart. This focused view was required because typography, 6 px spacing, icon alignment, semantic branch color, and the removal of the old shared cap are too small to judge reliably in the full app view.

## Findings

No actionable P0, P1, or P2 differences remain.

- Fonts and typography: 11 px DM Sans UI text, medium weight, and `leading-none` produce the requested quieter density without clipping. Long project and checkout names retain truncation.
- Spacing and layout rhythm: pills are 22 px high with 6 px between the strip and composer; each pill uses 6 px horizontal padding and a 6 px inter-pill gap. The former oversized cap, negative overlap, border collision, and excess vertical padding are gone.
- Colors and visual tokens: pills use the existing composer-context token in both themes. Branch and worktree keep distinct semantic foreground tokens; the live branch state is visibly different from location/project metadata in light and dark modes.
- Image quality and asset fidelity: no raster assets were introduced. Existing Lucide laptop, folder, branch, and worktree icons remain sharp and optically aligned at 12 px.
- Copy and content: labels remain data-derived (`Local`, project name, branch/worktree name), with no new static copy.
- Behavior and accessibility: in the active-run AX tree, Checkout context is exposed as plain text (`Local clark-desktop main`), not buttons, while the independent Stop button remains available. Before a session starts, environment and folder controls remain interactive.

## Open questions

None. The reference's add-folder affordance is intentionally absent from an ongoing conversation because the checkout is pinned and informational in that state.

## Comparison history

1. Initial source comparison found a P1 surface/layout mismatch: one oversized shared cap introduced excessive height, border collision, and a visual attachment to the composer. Fix: replaced the shared cap with independent pills, removed the negative overlap, aligned the strip to the composer column, and reordered location before project and checkout.
2. The next light-mode comparison found a P2 density mismatch: the strip still used 12 px type and 24 px controls with more vertical space than requested. Fix: reduced type to 11 px, controls to 22 px, and the composer gap to 6 px.
3. Post-fix dark and light native captures were combined with the source regions in `/tmp/clark-context-final-comparison.jpeg`. No actionable P0/P1/P2 mismatch remains.

## Implementation checklist

- [x] Independent context pills above the composer
- [x] 11 px type and 22 px pill height
- [x] 6 px vertical gap to composer
- [x] Distinct branch/worktree semantic colors
- [x] Read-only metadata during active runs
- [x] Dark-mode native verification
- [x] Light-mode native verification
- [x] Signed 0.1.61 bundle installed in `/Applications/Clark Code.app`

## Follow-up polish

No P3 follow-up is required for this scope.

final result: passed

---

# Artifact workspace horizontal resize design QA

## Evidence

- Source visual truth: `/Users/stan/Desktop/Screenshot 2026-07-18 at 10.29.30 PM.png`
- Rendered implementation:
  - `/tmp/clark-artifact-resize-before.jpeg`
  - `/tmp/clark-artifact-resize-wide.jpeg`
  - `/tmp/clark-artifact-resize-narrow.jpeg`
- Combined full-view comparison: `/tmp/clark-artifact-resize-comparison.jpeg`
- Combined focused divider comparison: `/tmp/clark-artifact-resize-focused-comparison.jpeg`
- Viewport: 1179 × 768 native macOS app window for implementation captures; source capture is 1312 × 1007 and was normalized to the same comparison height.
- State: authenticated Clark Code desktop app, dark theme, completed local conversation, Markdown artifact workspace open.

## Full-view comparison evidence

The source shows a fixed conversation/artifact split. The two implementation captures show the same artifact and conversation at one wider and one narrower artifact width. The conversation, composer, artifact toolbar, document preview, and context rail remain within their panes without overlap during either state.

## Focused region comparison evidence

The focused comparison isolates the split boundary from the source and both implementation states. It shows the divider moving from x=697 to x=559 when expanding the artifact and then to x=760 when narrowing it. The corrected drag does not select document or conversation text.

## Findings

No actionable P0, P1, or P2 differences remain.

- Fonts and typography: resizing changes available line length only; existing DM Sans and Newsreader typography, weights, line heights, and truncation remain unchanged.
- Spacing and layout rhythm: an 8 px hit area sits between panes with a single 1 px tokenized divider. The artifact is constrained to at least 420 px and the conversation to at least 320 px.
- Colors and visual tokens: the idle divider uses `border-subtle`; hover, focus, and active drag use the existing accent token.
- Image quality and asset fidelity: no images or icon assets were introduced or altered.
- Copy and content: no product copy changed. The divider tooltip explains drag and double-click reset behavior.
- Behavior and accessibility: native mouse dragging works in both directions and continues outside the divider through window-level tracking. Width is persisted locally. The divider is a focusable vertical separator with value metadata; Left/Right arrows resize by 24 px, Home selects the minimum, End selects the maximum, and double-click resets to the default.
- Responsive behavior: below the existing `xl` breakpoint the artifact remains full-width and the splitter stays hidden, preserving the prior narrow-window behavior.

## Open questions

None.

## Comparison history

1. Initial implementation used pointer capture. Native Computer Use exposed a P1 interaction failure: dragging selected page text and did not move the split.
2. Fix: moved the desktop interaction to `mousedown` plus window-level `mousemove`/`mouseup`, retained the keyboard separator contract, and disabled selection only while resizing.
3. Post-fix native drags moved the boundary 138 px left and 201 px right with no text-selection artifact. Full and focused combined comparisons show no remaining actionable P0/P1/P2 issue.

## Implementation checklist

- [x] Horizontal drag in both directions
- [x] Window-level drag tracking
- [x] Conversation and artifact minimum widths
- [x] Persisted width
- [x] Keyboard-accessible separator
- [x] Double-click reset
- [x] Responsive fallback below `xl`
- [x] Native dark-mode verification
- [x] Signed 0.1.61 bundle installed in `/Applications/Clark Code.app`

## Follow-up polish

No P3 follow-up is required for this scope.

final result: passed

---

# Accessible Subagents Inspector Design QA

## Evidence

- Source visual truth: `/Users/stan/.codex/attachments/09c170c5-c010-4a04-823d-1e1c95aea4f5/image-1.png`
- Rendered implementation: `.test-ui-screenshots/subagents-inspector-final-1440.png`
- Full-view comparison: `.test-ui-screenshots/subagents-design-qa-comparison.png`
- Focused comparison: `.test-ui-screenshots/subagents-design-qa-focus.png`
- Responsive captures: `.test-ui-screenshots/subagents-inspector-1024.png` and `.test-ui-screenshots/subagents-inspector-800.png`
- Primary viewport: 1440 x 1024; responsive checks at 1024 x 768 and 800 x 720.
- State: dark Clark Code browser mock, one running subagent selected, one complete, and one queued.

## Findings

No actionable P0, P1, or P2 differences remain. The implementation preserves the reference's conversation chips, quiet right inspector, selected-running state, objective, current activity, and public progress receipt while fitting Clark's existing centered conversation system.

- Fonts and typography: passed. Existing Clark type roles and compact hierarchy remain intact; agent labels wrap instead of truncating when three cards share the production conversation width.
- Spacing and layout rhythm: passed. The inspector uses the existing resizable detail shell and restrained separators. The narrower production conversation column is an intentional Clark-system adaptation rather than a second custom layout.
- Colors and visual tokens: passed. Graphite surfaces, semantic success/danger states, quiet borders, and violet selection/focus reuse existing tokens.
- Image quality and asset fidelity: passed. No custom raster asset is needed; status, disclosure, and navigation icons use the installed Lucide set.
- Copy and content: passed. The inspector exposes bounded objective, activity, attempt, elapsed time, result, and aggregate status. It does not reveal chain-of-thought, private reasoning, raw prompts, tokens, or provider credentials.
- Accessibility and interaction: passed. Agent chips and selector rows are named buttons with 44 px or larger targets, selected state is exposed through `aria-pressed`, Escape closes the inspector, Jump to transcript returns to the named transcript region, status is not color-only, and reduced motion is respected.
- Responsive behavior: passed. At 1024 px and 800 px the inspector becomes the primary content pane without horizontal overflow; the persistent Clark navigation remains available.

## Comparison History

1. Initial P2: the compact production conversation width truncated all three visible agent names.
2. Fix: increased chip height and allowed two-line labels, preserving three parallel cards and the reference hierarchy while making the names fully readable.
3. Post-fix browser measurements show each chip's scroll and client dimensions match, with no page-level horizontal overflow at desktop or responsive viewports.

## Interaction and Runtime Evidence

- Opened the running agent from its transcript chip and switched to the completed agent from the inspector task list.
- Closed the inspector with Escape.
- Reopened it and verified Jump to transcript closes the inspector and returns focus to the parallel-work region.
- Verified the 1024 px and 800 px layouts and zero body-width overflow.
- Browser application errors: none. The only warning was the expected reduced-motion development notice.
- Frontend typecheck and complete Vitest suite passed.
- Targeted Rust lifecycle/projection tests passed for `agent-core`, `provider-clark`, and `provider-local`.

## Follow-up Polish

- P3: when providers expose typed retry history, the progress timeline can add a compact rework receipt without showing internal reasoning.

final result: passed
