import { cn } from "../../lib/cn";
import {
  CHAT_CONTRASTS,
  type ChatContrast,
} from "../../lib/localAgent";
import { useSessionStore } from "../../store/sessionStore";
import { Row } from "./Primitives";

const LABELS: Record<ChatContrast, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
};

export function ChatContrastControl() {
  const contrast = useSessionStore((state) => state.localSettings.chatContrast ?? "low");
  const setLocalSettings = useSessionStore((state) => state.setLocalSettings);

  return (
    <Row name="Chat contrast" sub="Brightness of assistant response text">
      <div
        role="group"
        aria-label="Chat contrast"
        className="flex max-w-full flex-wrap justify-end rounded-lg bg-bg-sunken p-0.5 text-xs"
      >
        {CHAT_CONTRASTS.map((value) => (
          <button
            key={value}
            type="button"
            aria-pressed={contrast === value}
            onClick={() => setLocalSettings({ chatContrast: value })}
            className={cn(
              "rounded-md px-2.5 py-1 transition",
              contrast === value
                ? "bg-bg-elevated text-ink shadow-sm"
                : "text-ink-muted hover:text-ink-secondary",
            )}
          >
            {LABELS[value]}
          </button>
        ))}
      </div>
    </Row>
  );
}
