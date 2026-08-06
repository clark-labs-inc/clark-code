import { beforeEach, describe, expect, it } from "vitest";
import { useSessionStore } from "./sessionStore";
import { emptySnapshot, type Session } from "../core-bridge/types";

// The sidebar's project button (openProjectTerminal): make a folder the
// current project. It records a terminal launch rooted at that folder (so a
// terminal that is already open adds a fresh tab there) but never forces the
// terminal drawer open — opening a project and opening a terminal are
// separate actions. The composer must follow the picked folder — a live
// session bound to a different root is detached (it keeps running in the
// sidebar pool) so the next message starts in the new project instead of the
// old one. Picking the session's own folder leaves the conversation in place.
// It must no-op cleanly when the OS picker is unavailable (browser/dev) or
// cancelled.

const baseSettings = {
  cwd: "/tmp/project",
  model: "clark-code",
  reasoningEffort: "",
};

beforeEach(() => {
  useSessionStore.setState({
    session: null,
    snapshot: emptySnapshot(),
    error: null,
    notice: null,
    runningIds: [],
    localSettings: { ...baseSettings },
    recentProjects: [],
    activeProjectRoot: null,
    terminalOpen: false,
    terminalLaunch: null,
  });
});

describe("openProjectTerminal", () => {
  it("sets the folder as the current project without forcing the terminal open", async () => {
    await useSessionStore.getState().openProjectTerminal("/tmp/demo-app");

    const s = useSessionStore.getState();
    expect(s.localSettings.cwd).toBe("/tmp/demo-app");
    expect(s.terminalOpen).toBe(false);
    expect(s.terminalLaunch).toEqual({ cwd: "/tmp/demo-app", nonce: 1 });
    expect(s.recentProjects[0]).toBe("/tmp/demo-app");
  });

  it("issues a fresh launch per click so an open terminal adds a new tab", async () => {
    await useSessionStore.getState().openProjectTerminal("/tmp/one");
    await useSessionStore.getState().openProjectTerminal("/tmp/two");

    const s = useSessionStore.getState();
    expect(s.terminalLaunch).toEqual({ cwd: "/tmp/two", nonce: 2 });
    expect(s.localSettings.cwd).toBe("/tmp/two");
  });

  it("moves the composer to the picked folder by detaching a live session on another root", async () => {
    useSessionStore.setState({
      session: { id: "old-chat", provider: "local" } as Session,
      activeProjectRoot: "/tmp/live-session",
    });

    await useSessionStore.getState().openProjectTerminal("/tmp/other");

    const s = useSessionStore.getState();
    // Detached, not destroyed: the composer now binds to localSettings.cwd,
    // and the old conversation stays in the sidebar pool untouched.
    expect(s.session).toBeNull();
    expect(s.activeProjectRoot).toBeNull();
    expect(s.localSettings.cwd).toBe("/tmp/other");
    expect(s.terminalOpen).toBe(false);
    expect(s.terminalLaunch?.cwd).toBe("/tmp/other");
    expect(s.notice).toBeNull();
  });

  it("keeps the live session when the picked folder is its own root", async () => {
    useSessionStore.setState({
      session: { id: "old-chat", provider: "local" } as Session,
      activeProjectRoot: "/tmp/live-session",
    });

    await useSessionStore.getState().openProjectTerminal("/tmp/live-session");

    const s = useSessionStore.getState();
    expect(s.session?.id).toBe("old-chat");
    expect(s.activeProjectRoot).toBe("/tmp/live-session");
    expect(s.localSettings.cwd).toBe("/tmp/live-session");
    expect(s.terminalLaunch?.cwd).toBe("/tmp/live-session");
  });

  it("warns when the detached session is still running in the sidebar", async () => {
    useSessionStore.setState({
      session: { id: "busy-chat", provider: "local" } as Session,
      activeProjectRoot: "/tmp/busy-root",
      runningIds: ["busy-chat"],
    });

    await useSessionStore.getState().openProjectTerminal("/tmp/other");

    const s = useSessionStore.getState();
    expect(s.session).toBeNull();
    expect(s.notice).toContain("busy-root");
    expect(s.notice).toContain("still running in the sidebar");
  });

  it("no-ops when no path is given and the picker is unavailable", async () => {
    // No __TAURI_INTERNALS__ in the test environment → pickFolder returns null,
    // exactly like cancelling the OS dialog.
    await useSessionStore.getState().openProjectTerminal();

    const s = useSessionStore.getState();
    expect(s.localSettings.cwd).toBe("/tmp/project");
    expect(s.terminalOpen).toBe(false);
    expect(s.terminalLaunch).toBeNull();
    expect(s.error).toBeNull();
  });

  it("treats a blank path like no path (picker, not an empty cwd)", async () => {
    await useSessionStore.getState().openProjectTerminal("   ");

    const s = useSessionStore.getState();
    expect(s.localSettings.cwd).toBe("/tmp/project");
    expect(s.terminalLaunch).toBeNull();
  });
});
