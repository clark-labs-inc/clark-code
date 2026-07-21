import { lazy, Suspense, useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent } from "react";
import { useSessionStore } from "./store/sessionStore";
import { useFanOutStore } from "./store/fanOutStore";
import { useTheme } from "./lib/useTheme";
import { useHotkeys } from "./lib/hotkeys";
import type { TextSize } from "./lib/useTextSize";
import { TopBar } from "./surfaces/TopBar";
import { Sidebar } from "./surfaces/Sidebar";
import { StartCard } from "./surfaces/StartCard";
import { OpeningScreen } from "./surfaces/OpeningScreen";
import { Composer } from "./surfaces/Composer";
import { GoalStatusRail } from "./surfaces/GoalStatusRail";
import { CreditBanner } from "./surfaces/CreditBanner";
import { ActivityRewardToast } from "./surfaces/ActivityRewardToast";
import { OfflineBanner } from "./surfaces/OfflineBanner";
import { CommandPalette } from "./surfaces/CommandPalette";
import { MobileRemoteAgent } from "./surfaces/MobileRemoteAgent";
import { PanelErrorBoundary } from "./components/PanelErrorBoundary";
import type { Artifact } from "./core-bridge/types";
import {
  DEFAULT_ARTIFACT_PANEL_WIDTH,
  MIN_ARTIFACT_PANEL_WIDTH,
  MIN_CONVERSATION_PANEL_WIDTH,
  constrainArtifactPanelWidth,
  loadArtifactPanelWidth,
  saveArtifactPanelWidth,
} from "./lib/artifactPanelWidth";

const Conversation = lazy(() =>
  import("./surfaces/Conversation").then((module) => ({ default: module.Conversation })),
);
const ArtifactWorkspace = lazy(() =>
  import("./surfaces/work/ArtifactWorkspace").then((module) => ({ default: module.ArtifactWorkspace })),
);
const SubagentsInspector = lazy(() =>
  import("./surfaces/SubagentsInspector").then((module) => ({ default: module.SubagentsInspector })),
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

export default function AuthenticatedWorkspace({
  textSize,
  onTextSizeChange,
}: {
  textSize: TextSize;
  onTextSizeChange: (size: TextSize) => void;
}) {
  const session = useSessionStore((state) => state.session);
  const openingScreen = useSessionStore((state) => state.opening !== null);
  const terminalOpen = useSessionStore((state) => state.terminalOpen);
  const mcpOpen = useSessionStore((state) => state.mcpOpen);
  const sshOpen = useSessionStore((state) => state.sshOpen);
  const settingsOpen = useSessionStore((state) => state.settingsOpen);
  const subagentsOpen = useFanOutStore(
    (state) => state.inspectorOpen && state.fanOut !== null,
  );
  const closeSubagents = useFanOutStore((state) => state.closeInspector);
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
  const [artifactPanelWidth, setArtifactPanelWidth] = useState(loadArtifactPanelWidth);
  const [subagentsPanelWidth, setSubagentsPanelWidth] = useState(480);
  const [resizingArtifactPanel, setResizingArtifactPanel] = useState(false);
  const [splitPaneWidth, setSplitPaneWidth] = useState(0);
  const splitPaneRef = useRef<HTMLDivElement>(null);
  const artifactResizeCleanupRef = useRef<(() => void) | null>(null);
  const terminalTouched = useRef(false);
  if (terminalOpen) terminalTouched.current = true;
  const { dark, toggle, colorblind, toggleColorblind } = useTheme();
  const sidePanelOpen = subagentsOpen || activeArtifactId !== null;
  const sidePanelWidth = subagentsOpen ? subagentsPanelWidth : artifactPanelWidth;

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

  useEffect(() => {
    if (subagentsOpen && activeArtifactId) setActiveArtifactId(null);
  }, [activeArtifactId, subagentsOpen]);

  useEffect(() => {
    const splitPane = splitPaneRef.current;
    if (!sidePanelOpen || !splitPane || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) => {
      setSplitPaneWidth(entry.contentRect.width);
      if (subagentsOpen) {
        setSubagentsPanelWidth((current) =>
          constrainArtifactPanelWidth(current, entry.contentRect.width),
        );
      } else {
        setArtifactPanelWidth((current) =>
          constrainArtifactPanelWidth(current, entry.contentRect.width),
        );
      }
    });
    observer.observe(splitPane);
    return () => observer.disconnect();
  }, [sidePanelOpen, subagentsOpen]);

  useEffect(() => () => artifactResizeCleanupRef.current?.(), []);

  const resizeSidePanel = (clientX: number) => {
    const bounds = splitPaneRef.current?.getBoundingClientRect();
    if (!bounds) return;
    const next = constrainArtifactPanelWidth(bounds.right - clientX, bounds.width);
    if (subagentsOpen) setSubagentsPanelWidth(next);
    else setArtifactPanelWidth(next);
  };
  const finishSidePanelResize = () => {
    setResizingArtifactPanel(false);
    if (!subagentsOpen) {
      setArtifactPanelWidth((current) => {
        saveArtifactPanelWidth(current);
        return current;
      });
    }
  };
  const handleSidePanelResizeStart = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    artifactResizeCleanupRef.current?.();
    setResizingArtifactPanel(true);
    resizeSidePanel(event.clientX);
    const move = (moveEvent: MouseEvent) => {
      moveEvent.preventDefault();
      resizeSidePanel(moveEvent.clientX);
    };
    const stop = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", stop);
      artifactResizeCleanupRef.current = null;
      finishSidePanelResize();
    };
    artifactResizeCleanupRef.current = stop;
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", stop, { once: true });
  };
  const handleSidePanelResizeKey = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const bounds = splitPaneRef.current?.getBoundingClientRect();
    if (!bounds) return;
    let next = sidePanelWidth;
    if (event.key === "ArrowLeft") next += 24;
    else if (event.key === "ArrowRight") next -= 24;
    else if (event.key === "Home") next = MIN_ARTIFACT_PANEL_WIDTH;
    else if (event.key === "End") next = bounds.width;
    else return;
    event.preventDefault();
    const constrained = constrainArtifactPanelWidth(next, bounds.width);
    if (subagentsOpen) setSubagentsPanelWidth(constrained);
    else {
      setArtifactPanelWidth(constrained);
      saveArtifactPanelWidth(constrained);
    }
  };

  const openArtifact = (artifact: Artifact) => {
    closeSubagents();
    setActiveArtifactId(artifact.id);
  };
  const openArtifacts = () => {
    const artifacts = useSessionStore.getState().snapshot.artifacts;
    const latest = artifacts.at(-1);
    if (!latest) return;
    closeSubagents();
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
    { key: "Tab", shift: true, allowInInput: true, run: () => useSessionStore.getState().cycleApprovalPolicy() },
  ]);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-bg text-ink">
      <MobileRemoteAgent />
      <ActivityRewardToast />
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
          <div
            ref={splitPaneRef}
            className={`flex min-h-0 flex-1 ${
              resizingArtifactPanel ? "cursor-col-resize select-none" : ""
            }`}
          >
            <div
              className={
                sidePanelOpen
                  ? "hidden min-w-[20rem] flex-1 flex-col xl:flex"
                  : "flex min-w-0 flex-1 flex-col"
              }
            >
              <PanelErrorBoundary title="Conversation panel needs to restart" resetKey={session.id}>
                <Suspense fallback={<div className="min-h-0 flex-1" />}>
                  <Conversation activeArtifactId={activeArtifactId} onOpenArtifact={openArtifact} />
                </Suspense>
              </PanelErrorBoundary>
              <GoalStatusRail />
              <Composer />
            </div>
            {sidePanelOpen && (
              <>
                <div
                  role="separator"
                  aria-label="Resize details panel"
                  aria-orientation="vertical"
                  aria-valuemin={MIN_ARTIFACT_PANEL_WIDTH}
                  aria-valuemax={Math.max(
                    MIN_ARTIFACT_PANEL_WIDTH,
                    splitPaneWidth - MIN_CONVERSATION_PANEL_WIDTH,
                  )}
                  aria-valuenow={sidePanelWidth}
                  tabIndex={0}
                  title="Drag to resize details panel · Double-click to reset"
                  onDoubleClick={() => {
                    const width = constrainArtifactPanelWidth(
                      DEFAULT_ARTIFACT_PANEL_WIDTH,
                      splitPaneRef.current?.clientWidth ??
                        DEFAULT_ARTIFACT_PANEL_WIDTH + MIN_CONVERSATION_PANEL_WIDTH,
                    );
                    if (subagentsOpen) setSubagentsPanelWidth(480);
                    else {
                      setArtifactPanelWidth(width);
                      saveArtifactPanelWidth(width);
                    }
                  }}
                  onKeyDown={handleSidePanelResizeKey}
                  onMouseDown={handleSidePanelResizeStart}
                  className="group relative z-20 hidden w-2 shrink-0 touch-none cursor-col-resize outline-none xl:block"
                >
                  <span
                    className={`absolute inset-y-0 left-1/2 w-px -translate-x-1/2 transition-colors ${
                      resizingArtifactPanel
                        ? "bg-accent"
                        : "bg-border-subtle group-hover:bg-accent/70 group-focus-visible:bg-accent"
                    }`}
                  />
                </div>
                <div
                  className="flex min-w-0 flex-1 xl:flex-none"
                  style={{ width: sidePanelWidth }}
                >
                  <PanelErrorBoundary
                    title="Details panel needs to restart"
                    resetKey={subagentsOpen ? "subagents" : activeArtifactId}
                    onDismiss={() => {
                      closeSubagents();
                      setActiveArtifactId(null);
                    }}
                  >
                    <Suspense fallback={<div className="min-w-0 flex-1 bg-bg-elevated" />}>
                      {subagentsOpen ? (
                        <SubagentsInspector />
                      ) : activeArtifactId ? (
                        <ArtifactWorkspace
                          activeArtifactId={activeArtifactId}
                          conversationTitle={conversationTitle ?? "Current conversation"}
                          onSelect={setActiveArtifactId}
                          onClose={() => setActiveArtifactId(null)}
                          onJumpToSource={jumpToSource}
                        />
                      ) : null}
                    </Suspense>
                  </PanelErrorBoundary>
                </div>
              </>
            )}
          </div>
        ) : (
          <>
            <StartCard />
            <Composer />
          </>
        )}
        {/* The terminal drawer lives at the shell level (not inside a branch)
            so it survives switching between the start screen and a session —
            and so the sidebar can open it in a freshly picked project folder
            before any session exists. */}
        {terminalTouched.current && (
          <PanelErrorBoundary title="Terminal panel needs to restart" resetKey={terminalOpen ? 1 : 0}>
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
          </PanelErrorBoundary>
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
        <PanelErrorBoundary
          title="Settings needs to restart"
          resetKey={settingsOpen ? 1 : 0}
          onDismiss={() => useSessionStore.getState().setSettingsOpen(false)}
        >
          <Suspense fallback={null}>
            <Settings
              dark={dark}
              onToggleTheme={toggle}
              colorblind={colorblind}
              onToggleColorblind={toggleColorblind}
              textSize={textSize}
              onTextSizeChange={onTextSizeChange}
            />
          </Suspense>
        </PanelErrorBoundary>
      )}
      <CommandPalette dark={dark} onToggleTheme={toggle} />
    </div>
  );
}
