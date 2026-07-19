import React from "react";
import ReactDOM from "react-dom/client";
import { MotionConfig } from "motion/react";
import { PermissionGate } from "./surfaces/PermissionGate";
import "./index.css";
import type { PermissionRequest } from "./core-bridge/types";

const PLAN = `
1. **Conversation provider** — \`src-tauri/src/lib.rs\` Add \`TauriConversationProvider\` with an \`Arc<Mutex<HashMap<String, Arc<dyn ConversationHandler>>>>\` to mirror Claude's provider. Provider handles \`(session_id, prompt)\` → lock → \`get(&session_id)\` → \`handle(prompt)\`.

2. **Tauri shim** — \`src-tauri/src/lib.rs\` Expose \`#[tauri::command]\` async \`side_question(session_id, question, state)\` → \`Result<String, String>\`, resolving the session entry and calling \`provider.side_question(…)\`. Register it in the existing \`generate_handler!\` list.

   ⚠️ \`commands.rs\` already has someone's in-progress edits — re-read right before editing, append-only.

3. **Tauri command** — \`src-tauri/src/commands.rs\`, \`src-tauri/src/lib.rs\` Add \`#[tauri::command] pub async fn side_question(session_id, question, state) -> Result<String, String>\`, resolving the session entry and calling \`provider.side_question(…)\`. Register it in the existing \`generate_handler!\` list.

4. **CoreBridge seam** — \`app/src/core-bridge/{bridge,tauriBridge,mockBridge,devBridge}.ts\` Add \`sideQuestion?(sessionId, question): Promise<string>\`. Tauri → \`invoke("side_question", …)\`. Mock → scripted one-line answer after a short delay. devBridge → no-op/forward.

5. **Session store + composer routing** — \`app/src/store/sessionStore.ts\`, \`app/src/surfaces/Composer.tsx\`, \`app/src/lib/slashCommands.ts\`

   - Store state \`sideQuestion: {question, answer|null, error|null, loading}\` + actions \`askSideQuestion(text)\`, \`dismissSideQuestion()\`.
   - In \`Composer.submit\`, detect a \`/btw\` prefix **before** send → route to \`askSideQuestion\`, clear composer.
   - Add \`/btw\` to slash autocomplete as a discoverable hint.

6. **Overlay UI** — new \`app/src/surfaces/SideQuestionCard.tsx\`, mounted in \`Conversation.tsx\`. Floating card: \`/btw\` accent header + dimmed question + scrollable markdown answer + spinner while loading + error state. Dismiss on Esc/click. It stays mounted while the main run streams behind it and matches the warm paper/violet design system.

---

### One decision: concurrency at the Tauri lock

\`side_question\` runs while holding the \`HostSession\` mutex, so the main run's **snapshot emission pauses** for the call's duration (~2–6s). The main run's engine task is independent and keeps executing unaffected; buffered events flush when the side question returns. This matches Claude Code's “overlay over a paused transcript” UX.
`.trim();

const request: PermissionRequest = {
  id: "plan-preview",
  session: "preview-session",
  title: "Review the proposed plan",
  risk: "plan",
  detail: PLAN,
  options: [
    { id: "approve_auto", label: "Approve — run it for me", kind: "allow_once" },
    { id: "approve_review", label: "Approve — check each step with me", kind: "allow_once" },
    { id: "revise", label: "Suggest changes", kind: "reject_once" },
  ],
};

document.documentElement.classList.remove("dark");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <MotionConfig reducedMotion="always">
      <main className="mx-auto min-h-screen max-w-2xl bg-bg px-5 py-6">
        <PermissionGate req={request} />
      </main>
    </MotionConfig>
  </React.StrictMode>,
);
