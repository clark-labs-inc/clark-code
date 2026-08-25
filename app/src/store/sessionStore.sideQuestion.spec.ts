import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import { useSessionStore } from "./sessionStore";

const sessionA = { id: "side-question-a", provider: "local" } as Session;
const sessionB = { id: "side-question-b", provider: "local" } as Session;

beforeEach(() => {
  useSessionStore.getState().endSession({ force: true });
  useSessionStore.setState({
    bridge: null,
    session: null,
    snapshot: emptySnapshot(),
    sideQuestion: null,
    auth: null,
  });
});

describe("side-question composer ownership", () => {
  it("does not let an older conversation overwrite a newer overlay", async () => {
    let answerA!: (answer: string) => void;
    let answerB!: (answer: string) => void;
    const bridge = {
      sideQuestion: vi.fn((sessionId: string) => new Promise<string>((resolve) => {
        if (sessionId === sessionA.id) answerA = resolve;
        else answerB = resolve;
      })),
    } as unknown as CoreBridge;
    useSessionStore.setState({ bridge, session: sessionA });

    const askingA = useSessionStore.getState().askSideQuestion("question for A");
    useSessionStore.setState({ session: sessionB });
    const askingB = useSessionStore.getState().askSideQuestion("question for B");

    answerA("answer from A");
    await askingA;
    expect(useSessionStore.getState().sideQuestion).toMatchObject({
      sessionId: sessionB.id,
      question: "question for B",
      answer: null,
      loading: true,
    });

    answerB("answer from B");
    await askingB;
    expect(useSessionStore.getState().sideQuestion).toMatchObject({
      sessionId: sessionB.id,
      question: "question for B",
      answer: "answer from B",
      loading: false,
    });
  });

  it("does not revive a dismissed overlay after a newer question in the same conversation", async () => {
    const resolvers: Array<(answer: string) => void> = [];
    const bridge = {
      sideQuestion: vi.fn(() => new Promise<string>((resolve) => resolvers.push(resolve))),
    } as unknown as CoreBridge;
    useSessionStore.setState({ bridge, session: sessionA });

    const first = useSessionStore.getState().askSideQuestion("first question");
    useSessionStore.getState().dismissSideQuestion();
    const second = useSessionStore.getState().askSideQuestion("second question");

    resolvers[0]("stale answer");
    await first;
    expect(useSessionStore.getState().sideQuestion).toMatchObject({
      question: "second question",
      answer: null,
      loading: true,
    });

    resolvers[1]("current answer");
    await second;
    expect(useSessionStore.getState().sideQuestion).toMatchObject({
      question: "second question",
      answer: "current answer",
      loading: false,
    });
  });
});
