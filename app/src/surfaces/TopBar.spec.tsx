import { beforeEach, describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { useSessionStore } from "../store/sessionStore";
import { UpdatePill } from "./TopBar";

beforeEach(() => {
  useSessionStore.setState({
    update: null,
    updateProgress: null,
    updateChecking: false,
    updateWaiting: false,
    updateApplying: false,
  });
});

describe("UpdatePill", () => {
  it("renders an actionable ready state as soon as an update is staged", () => {
    useSessionStore.setState({ update: { version: "0.1.65" } });

    const html = renderToStaticMarkup(<UpdatePill />);

    expect(html).toContain("<button");
    expect(html).toContain("Ready to update");
    expect(html).toContain("Ready to update Clark Code to 0.1.65; restart now");
  });

  it("shows download progress before the ready action", () => {
    useSessionStore.setState({
      updateProgress: { downloaded: 25, total: 100 },
    });

    const html = renderToStaticMarkup(<UpdatePill />);

    expect(html).toContain("Downloading update 25%");
    expect(html).not.toContain("Ready to update");
  });
});
