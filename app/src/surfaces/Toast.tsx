import type { CSSProperties } from "react";
import { useEffect } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  CircleX,
  Info,
  Loader2,
  X,
} from "lucide-react";
import { Toaster, toast } from "sonner";
import { useSessionStore } from "../store/sessionStore";
import type { TextSize } from "../lib/useTextSize";

const NOTICE_TOAST_ID = "clark-notice";
const WARNING_TOAST_ID = "clark-warning";
const TEXT_SIZE_TOAST_ID = "clark-text-size";

export const CLARK_TOAST_DURATION = {
  feedback: 1_200,
  notice: 4_000,
  warning: 8_000,
} as const;

const toasterStyle = {
  "--width": "min(22rem, calc(100vw - 2rem))",
} as CSSProperties;

/** One Sonner host for every transient notification. Sonner owns queueing,
 * stacking, focus, hotkeys, pause-on-hover, and swipe dismissal; Clark owns
 * its colors, type, spacing, icons, and motion through `index.css`. */
export function ClarkToaster({ dark }: { dark: boolean }) {
  return (
    <Toaster
      className="clark-toaster"
      theme={dark ? "dark" : "light"}
      position="bottom-center"
      visibleToasts={3}
      gap={8}
      offset={16}
      mobileOffset={12}
      closeButton
      expand={false}
      richColors={false}
      swipeDirections={["left", "right"]}
      containerAriaLabel="Notifications"
      toastOptions={{ closeButtonAriaLabel: "Dismiss notification" }}
      icons={{
        success: <CheckCircle2 className="size-4" aria-hidden="true" />,
        info: <Info className="size-4" aria-hidden="true" />,
        warning: <AlertTriangle className="size-4" aria-hidden="true" />,
        error: <CircleX className="size-4" aria-hidden="true" />,
        loading: <Loader2 className="size-4 animate-spin" aria-hidden="true" />,
        close: <X className="size-3" aria-hidden="true" />,
      }}
      style={toasterStyle}
    />
  );
}

function clearNoticeIfCurrent(message: string) {
  const state = useSessionStore.getState();
  if (state.notice === message) state.dismissNotice();
}

function clearWarningIfCurrent(message: string) {
  const state = useSessionStore.getState();
  if (state.warning === message) state.dismissWarning();
}

export function showNoticeToast(message: string) {
  toast.success(message, {
    id: NOTICE_TOAST_ID,
    position: "bottom-center",
    duration: CLARK_TOAST_DURATION.notice,
    onDismiss: () => clearNoticeIfCurrent(message),
    onAutoClose: () => clearNoticeIfCurrent(message),
  });
}

export function showWarningToast(message: string) {
  toast.warning(message, {
    id: WARNING_TOAST_ID,
    position: "bottom-center",
    duration: CLARK_TOAST_DURATION.warning,
    onDismiss: () => clearWarningIfCurrent(message),
    onAutoClose: () => clearWarningIfCurrent(message),
  });
}

export function showTextSizeToast(textSize: TextSize) {
  toast(`${textSize}%`, {
    id: TEXT_SIZE_TOAST_ID,
    position: "top-right",
    duration: CLARK_TOAST_DURATION.feedback,
    dismissible: false,
    closeButton: false,
    className: "clark-toast--text-size",
  });
}

/** Bridges the store's success/info channel into Sonner without changing the
 * existing store contract used by native notifications and action callers. */
export function NoticeToast() {
  const notice = useSessionStore((state) => state.notice);

  useEffect(() => {
    if (!notice) {
      toast.dismiss(NOTICE_TOAST_ID);
      return;
    }
    showNoticeToast(notice);
  }, [notice]);

  return null;
}

/** Non-fatal warning sibling of `NoticeToast`. Warnings stay visible longer
 * than confirmations but share the same accessible queue and visual shell. */
export function WarningToast() {
  const warning = useSessionStore((state) => state.warning);

  useEffect(() => {
    if (!warning) {
      toast.dismiss(WARNING_TOAST_ID);
      return;
    }
    showWarningToast(warning);
  }, [warning]);

  return null;
}

/** Compact browser-style feedback for global text-size shortcuts. A stable ID
 * updates the visible toast in place on repeated key presses instead of
 * flooding the notification stack. */
export function TextSizeToast({ textSize, signal }: { textSize: TextSize; signal: number }) {
  useEffect(() => {
    if (signal === 0) return;
    showTextSizeToast(textSize);
  }, [signal, textSize]);

  return null;
}
