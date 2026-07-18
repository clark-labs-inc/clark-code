import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { useSessionStore } from "./store/sessionStore";
import { useTheme } from "./lib/useTheme";
import { useHotkeys } from "./lib/hotkeys";
import { TopBar } from "./surfaces/TopBar";
import { Sidebar } from "./surfaces/Sidebar";
import { StartCard } from "./surfaces/StartCard";
import { OpeningScreen } from "./surfaces/OpeningScreen";
import { Composer } from "./surfaces/Composer";
import { CreditBanner } from "./surfaces/CreditBanner";
import { OfflineBanner } from "./surfaces/OfflineBanner";
import { CommandPalette } from "./surfaces/CommandPalette";
import { MobileRemoteAgent } from "./surfaces/MobileRemoteAgent";
import type { Artifact } from "./core-bridge/types";

const Conversation = lazy(() =>
  import("./surfaces/Conversation").then((module) => ({ default: module.Conversation })),
);
const ArtifactWorkspace = lazy(() =>
  import("./surfaces/work/ArtifactWorkspace").then((module) => ({ default: module.ArtifactWorkspace })),
);
const TerminalPanel = lazy(() =>
  import("./surfaces/TerminalPanel").then((module) => ({ default: module.TerminalPanel })),
);
const McpSettings = lazy(() =>
  import("./surfaces/McpSettings").then((module) => ({ default: module.McpSettings })),
);
const SshSettings = lazy(() =>
  import("./surfaces/SshSettings").then((module) => ({ default: module.SshSettings })),
);
const Settings = lazy(() =>
  import("./surfaces/Settings").then((module) => ({ default: module.Settings })),
);

export default function AuthenticatedWorkspace() {
  const session = useSessionStore((state) => state.session);
  const openingScreen = useSessionStore((state) => state.opening !== null);
  const terminalOpen = useSessionStore((state) => state.terminalOpen);
  const mcpOpen = useSessionStore((state) => state.mcpOpen);
  const sshOpen = useSessionStore((state) => state.sshOpen);
  const settingsOpen = useSessionStore((state) => state.settingsOpen);
  // A PRIMITIVE, not the snapshot object: the host re-clones the whole snapshot
  // per streamed token, so subscribing to `state.snapshot` here re-rendered the
  // entire workspace (TopBar, Composer, Sidebar…) ~60fps. The count only moves
  // when an artifact is added, so the shell stays still during streaming. The
  // artifact arrays themselves are read where they're actually needed:
  // callbacks via getState(), and ArtifactWorkspace subscribes itself.
  const artifactCount = useSessionStore((state) => state.snapshot.artifacts.length);
  const conversationTitle = useSessionStore((state) =>
    state.session ? state.conversations.find((conversation) => conversation.id === state.session?.id)?.title : null,
  );
  const [activeArtifactId, setActiveArtifactId] = useState<string | null>(null);
  const terminalTouched = useRef(false);
  if (terminalOpen) terminalTouched.current = true;
  const { dark, toggle, colorblind, toggleColorblind } = useTheme();

  useEffect(() => {
    setActiveArtifactId(null);
  }, [session?.id]);
  useEffect(() => {
    // Re-check membership when the artifact set changes (count is the cheap
    // signal; the array is read fresh from the store).
    const artifacts = useSessionStore.getState().snapshot.artifacts;
    if (activeArtifactId && !artifacts.some((artifact) => artifact.id === activeArtifactId)) {
      setActiveArtifactId(null);
    }
  }, [activeArtifactId, artifactCount]);

  const openArtifact = (artifact: Artifact) => setActiveArtifactId(artifact.id);
  const openArtifacts = () => {
    const artifacts = useSessionStore.getState().snapshot.artifacts;
    const latest = artifacts.at(-1);
    if (!latest) return;
    setActiveArtifactId((current) =>
      current && artifacts.some((artifact) => artifact.id === current) ? current : latest.id,
    );
  };
  const jumpToSource = (artifact: Artifact) => {
    const targetId = artifact.tool_call ? `tool-call-${artifact.tool_call}` : `artifact-${artifact.id}`;
    setActiveArtifactId(null);
    requestAnimationFrame(() => {
      const target = document.getElementById(targetId);
      target?.scrollIntoView({ behavior: "smooth", block: "center" });
      target?.focus({ preventScroll: true });
    });
  };

  useHotkeys([
    { key: "k", mod: true, allowInInput: true, run: () => useSessionStore.getState().togglePalette() },
    { key: "n", mod: true, run: () => useSessionStore.getState().endSession() },
    { key: "\\", mod: true, allowInInput: true, run: () => useSessionStore.getState().toggleSidebar() },
    {
      key: "j",
      mod: true,
      run: () => {
        if (useSessionStore.getState().session) useSessionStore.getState().toggleTerminal();
      },
    },
    { key: ".", mod: true, allowInInput: true, run: () => void useSessionStore.getState().cancelActive() },
    { key: ",", mod: true, allowInInput: true, run: () => useSessionStore.getState().setSettingsOpen(true) },
    { key: "Tab", shift: true, allowInInput: true, run: () => useSessionStore.getState().cyclePermissionMode() },
  ]);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-bg text-ink">
      <MobileRemoteAgent />
      <Sidebar artifactCount={artifactCount} onOpenArtifacts={openArtifacts} />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar dark={dark} onToggleTheme={toggle} />
        <OfflineBanner />
        <CreditBanner />
        {/* The opening screen takes priority over a still-mounted previous
            session: opening a not-yet-live conversation (especially remote,
            which brings up an SSH tunnel) keeps the old `session` set until
            the new one is live, and leaving the old chat interactive during
            that window looked frozen. Switching between already-live sessions
            never sets `opening` — it reattaches instantly. */}
        {openingScreen ? (
          <OpeningScreen />
        ) : session ? (
          <div className="flex min-h-0 flex-1">
            <div
              className={
                activeArtifactId
                  ? "hidden min-w-0 flex-col xl:flex xl:w-[25rem] xl:shrink-0 xl:border-r xl:border-border-subtle"
                  : "flex min-w-0 flex-1 flex-col"
              }
            >
              <Suspense fallback={<div className="min-h-0 flex-1" />}>
                <Conversation activeArtifactId={activeArtifactId} onOpenArtifact={openArtifact} />
              </Suspense>
              <Composer />
              {terminalTouched.current && (
                <Suspense
                  fallback={
                    terminalOpen ? (
                      <div className="flex h-10 shrink-0 items-center gap-2 border-t border-border px-4 text-xs text-ink-muted">
                        <span className="size-3 animate-[spin_1s_linear_infinite] rounded-full border border-ink-faint border-t-transparent" />
                        Loading terminal…
                      </div>
                    ) : null
                  }
                >
                  <TerminalPanel />
                </Suspense>
              )}
            </div>
            {activeArtifactId && (
              <Suspense fallback={<div className="min-w-0 flex-1 bg-bg-elevated" />}>
                <ArtifactWorkspace
                  activeArtifactId={activeArtifactId}
                  conversationTitle={conversationTitle ?? "Current conversation"}
                  onSelect={setActiveArtifactId}
                  onClose={() => setActiveArtifactId(null)}
                  onJumpToSource={jumpToSource}
                />
              </Suspense>
            )}
          </div>
        ) : (
          <>
            <StartCard />
            <Composer />
          </>
        )}
      </div>
      {mcpOpen && (
        <Suspense fallback={null}>
          <McpSettings />
        </Suspense>
      )}
      {sshOpen && (
        <Suspense fallback={null}>
          <SshSettings />
        </Suspense>
      )}
      {settingsOpen && (
        <Suspense fallback={null}>
          <Settings dark={dark} onToggleTheme={toggle} colorblind={colorblind} onToggleColorblind={toggleColorblind} />
        </Suspense>
      )}
      <CommandPalette dark={dark} onToggleTheme={toggle} />
    </div>
  );
}
