import React from "react";
import ReactDOM from "react-dom/client";
import { LazyMotion, MotionConfig, domMax } from "motion/react";
import { initializeAuthSession } from "./lib/auth";
import { installLocalCapture } from "./lib/localCapture";
import "./index.css";

if (import.meta.env.DEV) {
  installLocalCapture();
}

async function start(): Promise<void> {
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
    (window as unknown as Record<string, unknown>).__clarkProfiling = {
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
    root.textContent = "Clark Code could not open its encrypted local session. Restart the app or sign in again.";
  }
});
