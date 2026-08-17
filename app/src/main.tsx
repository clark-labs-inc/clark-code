import React from "react";
import ReactDOM from "react-dom/client";
import { LazyMotion, MotionConfig, domMax } from "motion/react";
import { initializeAuthSession } from "./lib/auth";
import { installLocalCapture } from "./lib/localCapture";
import "@product-entry";
import "./index.css";

if (import.meta.env.DEV) {
  installLocalCapture();
}

async function start(): Promise<void> {
  if (
    import.meta.env.DEV
    && new URLSearchParams(window.location.search).has("spec-run-preview")
  ) {
    const { SpecRunPreview } = await import("./surfaces/specialists/SpecRunPreview");
    ReactDOM.createRoot(document.getElementById("root")!).render(
      <React.StrictMode>
        <LazyMotion features={domMax} strict>
          <MotionConfig reducedMotion="user">
            <SpecRunPreview />
          </MotionConfig>
        </LazyMotion>
      </React.StrictMode>,
    );
    return;
  }

  if (
    import.meta.env.DEV
    && new URLSearchParams(window.location.search).has("rsi-loop-preview")
  ) {
    const { RsiLoopPreview } = await import("./surfaces/specialists/RsiLoopPreview");
    ReactDOM.createRoot(document.getElementById("root")!).render(
      <React.StrictMode>
        <LazyMotion features={domMax} strict>
          <MotionConfig reducedMotion="user">
            <RsiLoopPreview />
          </MotionConfig>
        </LazyMotion>
      </React.StrictMode>,
    );
    return;
  }

  // Restore native encrypted auth before importing stores; their initial state
  // is synchronously partitioned by the active account.
  await initializeAuthSession();
  const [{ default: App }, { getBridge }, { useSessionStore }] = await Promise.all([
    import("./App"),
    import("./core-bridge/bridge"),
    import("./store/sessionStore"),
  ]);

  // Headless profiling hook (harness/profile-chat-switch.mjs): expose the store
  // and bridge only in dev/preview bundles — never inside the shipped Tauri app.
  if (import.meta.env.DEV) {
    (window as unknown as Record<string, unknown>).__agentDesktopProfiling = {
      store: useSessionStore,
      getBridge,
    };
  }

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <LazyMotion features={domMax} strict>
        <MotionConfig reducedMotion="user">
          <App />
        </MotionConfig>
      </LazyMotion>
    </React.StrictMode>,
  );
}

void start().catch(() => {
  const root = document.getElementById("root");
  if (root) {
    root.textContent = "The desktop app could not open its local session. Restart the app and try again.";
  }
});
