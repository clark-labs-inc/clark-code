import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import { DEFAULT_LOCAL_SETTINGS } from "../lib/localAgent";
import { emptySnapshot } from "../core-bridge/types";
import { useSessionStore } from "../store/sessionStore";
import { EnvironmentPicker } from "./EnvironmentPicker";
import { sshDialogKeyboardIntent } from "./SshSettings";

beforeEach(() => {
  useSessionStore.setState({
    auth: null,
    providers: [],
    activeProvider: "local",
    projectMode: "local",
    selectedHostId: null,
    localSettings: { ...DEFAULT_LOCAL_SETTINGS, cwd: "" },
    recentProjects: [],
    sshOpen: false,
    session: null,
    snapshot: emptySnapshot(),
  });
});

describe("environment picker accessibility", () => {
  it("announces the target popover as a controlled collapsed dialog", () => {
    const markup = renderToStaticMarkup(
      <EnvironmentPicker compact allowCloud={false} showLocalFolder={false} />,
    );

    expect(markup).toContain('aria-haspopup="dialog"');
    expect(markup).toContain('aria-expanded="false"');
  });
});

describe("remote hosts dialog accessibility", () => {
  it("classifies Escape and Tab before the modal keyboard handler acts", () => {
    expect(sshDialogKeyboardIntent("Escape")).toBe("close");
    expect(sshDialogKeyboardIntent("Tab")).toBe("cycle_focus");
    expect(sshDialogKeyboardIntent("Enter")).toBe("none");
  });
});
