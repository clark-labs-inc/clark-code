import { beforeEach, describe, expect, it } from "vitest";
import { useSessionStore } from "./sessionStore";
import { emptySnapshot } from "../core-bridge/types";

// The sidebar's project button (openProjectTerminal): make a folder the
// current project and open a terminal tab rooted in it. It must seed the NEXT
// session's cwd without touching an already-live session's project root, and
// it must no-op cleanly when the OS picker is unavailable (browser/dev) or
// cancelled.

const baseSettings = {
  cwd: "/tmp/project",
  model: "clark-code",
  reasoningEffort: "",
  apiKey: "",
};

beforeEach(() => {
  useSessionStore.setState({
    session: null,
    snapshot: emptySnapshot(),
    error: null,
    localSettings: { ...baseSettings },
    recentProjects: [],
    activeProjectRoot: null,
    terminalOpen: false,
    terminalLaunch: null,
  });
});

describe("openProjectTerminal", () => {
  it("sets the folder as current project and launches a terminal there", async () => {
    await useSessionStore.getState().openProjectTerminal("/tmp/demo-app");

    const s = useSessionStore.getState();
    expect(s.localSettings.cwd).toBe("/tmp/demo-app");
    expect(s.terminalOpen).toBe(true);
    expect(s.terminalLaunch).toEqual({ cwd: "/tmp/demo-app", nonce: 1 });
    expect(s.recentProjects[0]).toBe("/tmp/demo-app");
  });

  it("issues a fresh launch per click so the panel opens a new tab", async () => {
    await useSessionStore.getState().openProjectTerminal("/tmp/one");
    await useSessionStore.getState().openProjectTerminal("/tmp/two");

    const s = useSessionStore.getState();
    expect(s.terminalLaunch).toEqual({ cwd: "/tmp/two", nonce: 2 });
    expect(s.localSettings.cwd).toBe("/tmp/two");
  });

  it("seeds the next session without hijacking a live session's root", async () => {
    useSessionStore.setState({ activeProjectRoot: "/tmp/live-session" });

    await useSessionStore.getState().openProjectTerminal("/tmp/other");

    const s = useSessionStore.getState();
    expect(s.activeProjectRoot).toBe("/tmp/live-session");
    expect(s.localSettings.cwd).toBe("/tmp/other");
    expect(s.terminalLaunch?.cwd).toBe("/tmp/other");
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
