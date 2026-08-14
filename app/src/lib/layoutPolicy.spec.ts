import { describe, expect, it } from "vitest";

const sourceModules = import.meta.glob("../surfaces/**/*.{ts,tsx}", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const librarySourceModules = import.meta.glob("../lib/**/*.{ts,tsx}", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

describe("responsive GUI layout policy", () => {
  it("keeps specialist chat primary while the optional canvas stays bounded", () => {
    const source = sourceModules["../surfaces/specialists/SpecialistWorkspace.tsx"];

    expect(source).toContain("<UpdatePill />");
    expect(source).toContain('useState<"chat" | "canvas">("chat")');
    expect(source).toContain("useState(false)");
    expect(source).toContain("xl:grid-cols-[minmax(32rem,1fr)_clamp(22rem,34vw,30rem)]");
    expect(source).toContain("Show insights");
    expect(source).toContain('cn("min-h-0 min-w-0"');
    expect(source).toContain("flex h-full min-h-0 min-w-0 flex-col");
    expect(source).toContain("flex min-h-0 min-w-0 flex-col bg-bg");
  });

  it("keeps the send controls visible while checkout menus remain reachable", () => {
    const composer = sourceModules["../surfaces/Composer.tsx"];
    const contextBar = sourceModules["../surfaces/ComposerContextBar.tsx"];
    const parallelContext = sourceModules["../surfaces/ParallelWorkContext.tsx"];

    expect(composer).toContain("flex min-w-0 flex-1 items-center gap-1 overflow-x-auto");
    expect(composer).toContain("flex shrink-0 items-center gap-2.5");
    expect(contextBar).toContain("gap-1.5 overflow-visible");
    expect(parallelContext).toContain("left-1/2");
    expect(parallelContext).toContain("calc(100vw-6rem)");
  });

  it("settles an accepted start-screen cloud draft before session remount", () => {
    const composer = sourceModules["../surfaces/Composer.tsx"];
    const branch = composer.indexOf("if (startedNewSession)");
    const settle = composer.indexOf(
      "startDraftSettleResult = await settleAcceptedDraft()",
      branch,
    );
    const start = composer.indexOf("await start({ submittedDraft: t", settle);

    expect(settle).toBeGreaterThan(branch);
    expect(start).toBeGreaterThan(settle);
  });

  it("remounts the composer when draft ownership changes", () => {
    const composer = sourceModules["../surfaces/Composer.tsx"];

    expect(composer).toContain("function ScopedComposer()");
    expect(composer).toContain(
      'return <ScopedComposer key={`${owner}\\u0000${conversationId ?? "new"}`} />',
    );
  });

  it("does not re-save the previous render after a cloud draft discard", () => {
    const source = librarySourceModules["./useCloudComposerDraft.ts"];
    const effect = source.indexOf("const activeGeneration = configRef.current?.generation");
    const guard = source.indexOf("discardedGenerationRef.current === activeGeneration", effect);
    const desiredWrite = source.indexOf("desiredTextRef.current = text", guard);

    expect(effect).toBeGreaterThan(-1);
    expect(guard).toBeGreaterThan(effect);
    expect(desiredWrite).toBeGreaterThan(guard);
  });

  it("surfaces a paused cloud draft instead of hiding repeated conflicts", () => {
    const composer = sourceModules["../surfaces/Composer.tsx"];
    const cloudDraft = librarySourceModules["./useCloudComposerDraft.ts"];

    expect(composer).toContain('draft.cloud.status === "conflict"');
    expect(composer).toContain("Draft cloud sync is paused.");
    expect(cloudDraft).toContain("conflictPausedRef.current = true");
    expect(cloudDraft).toContain("!conflictPausedRef.current");
    expect(cloudDraft).not.toContain("conflictRetries");
  });

  it("uses the compact specialist composition when window height is scarce", () => {
    const composer = sourceModules["../surfaces/Composer.tsx"];
    const welcome = sourceModules["../surfaces/specialists/SpecialistWelcome.tsx"];

    expect(composer).toContain('specialistSession && "specialist-composer"');
    expect(composer).toContain('specialistSession && "specialist-composer-surface"');
    expect(welcome).toContain("specialist-welcome-copy");
    expect(welcome).toContain("specialist-welcome-starters");
    expect(welcome).toContain("specialist-welcome-starter-detail");
    expect(welcome).toContain("specialist-welcome-footer");
  });

  it("keeps specialist surfaces flat instead of nesting bordered cards and tab bars", () => {
    const workspace = sourceModules["../surfaces/specialists/SpecialistWorkspace.tsx"];
    const welcome = sourceModules["../surfaces/specialists/SpecialistWelcome.tsx"];
    const primitives = sourceModules["../surfaces/specialists/SpecialistPrimitives.tsx"];
    const composer = sourceModules["../surfaces/Composer.tsx"];

    expect(workspace).not.toContain("border-r border-border-subtle");
    expect(workspace).not.toContain("border-b border-border-subtle");
    expect(workspace).not.toContain('aria-label="Specialists"');
    expect(welcome).not.toContain("border-y border-border-subtle");
    expect(welcome).not.toContain("border-b border-border-subtle");
    expect(primitives).not.toContain("border border-border-subtle");
    expect(primitives).not.toContain("shadow-sm");
    expect(composer).toContain('"border-border bg-composer-surface shadow-none"');
    expect(composer).toContain("rounded-lg border px-2.5");
    expect(composer).not.toContain("rounded-[20px] border");
  });

  it("uses a flat surface hierarchy across the shared application chrome", () => {
    const topBar = sourceModules["../surfaces/TopBar.tsx"];
    const sidebar = sourceModules["../surfaces/Sidebar.tsx"];
    const settingsPrimitives = sourceModules["../surfaces/settings/Primitives.tsx"];
    const settingsNavigation = sourceModules["../surfaces/settings/SettingsNavigation.tsx"];
    const commandPalette = sourceModules["../surfaces/CommandPalette.tsx"];
    const startCard = sourceModules["../surfaces/StartCard.tsx"];

    expect(topBar).not.toContain("border-b border-border-subtle");
    expect(sidebar).not.toContain("flex w-[17rem] shrink-0 flex-col border-r");
    expect(settingsNavigation).not.toContain("flex w-64 shrink-0 flex-col border-r");
    expect(settingsPrimitives).not.toContain("[&>*+*]:border-t");
    expect(settingsPrimitives).not.toContain("rounded-xl bg-bg-secondary/55");
    expect(commandPalette).not.toContain("border-b border-border-subtle bg-transparent");
    expect(startCard).not.toContain("divide-y divide-border-subtle");
    expect(startCard).not.toContain("rounded-2xl bg-bg-secondary/55");
  });
});
