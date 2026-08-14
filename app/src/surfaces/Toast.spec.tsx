import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { toast } from "sonner";
import { describe, expect, it, vi } from "vitest";

vi.mock("sonner", () => ({
  Toaster: ({
    className,
    theme,
    position,
    visibleToasts,
    closeButton,
    expand,
    containerAriaLabel,
    icons,
  }: {
    className?: string;
    theme?: string;
    position?: string;
    visibleToasts?: number;
    closeButton?: boolean;
    expand?: boolean;
    containerAriaLabel?: string;
    icons?: Record<string, ReactNode>;
  }) => (
    <div
      className={className}
      data-theme={theme}
      data-position={position}
      data-visible-toasts={visibleToasts}
      data-close-button={String(closeButton)}
      data-expand={String(expand)}
      aria-label={containerAriaLabel}
    >
      {icons?.success}
      {icons?.warning}
      {icons?.close}
    </div>
  ),
  toast: Object.assign(vi.fn(), {
    success: vi.fn(),
    warning: vi.fn(),
    dismiss: vi.fn(),
  }),
}));

import {
  CLARK_TOAST_DURATION,
  ClarkToaster,
  showNoticeToast,
  showTextSizeToast,
  showWarningToast,
} from "./Toast";

describe("ClarkToaster", () => {
  it("keeps Sonner behavior inside the Clark notification shell", () => {
    const markup = renderToStaticMarkup(<ClarkToaster dark={false} />);

    expect(markup).toContain('class="clark-toaster"');
    expect(markup).toContain('data-theme="light"');
    expect(markup).toContain('data-position="bottom-center"');
    expect(markup).toContain('data-visible-toasts="3"');
    expect(markup).toContain('data-close-button="true"');
    expect(markup).toContain('data-expand="false"');
    expect(markup).toContain('aria-label="Notifications"');
    expect(markup).toContain("lucide-circle-check");
    expect(markup).toContain("lucide-triangle-alert");
    expect(markup).toContain("lucide-x");
  });

  it("follows the application theme and keeps intentional dwell times", () => {
    expect(renderToStaticMarkup(<ClarkToaster dark />)).toContain('data-theme="dark"');
    expect(CLARK_TOAST_DURATION).toEqual({
      feedback: 1_200,
      notice: 4_000,
      warning: 8_000,
    });
  });

  it("maps Clark channels to stable Sonner queue entries", () => {
    showNoticeToast("Saved");
    showWarningToast("Sync delayed");
    showTextSizeToast(125);

    expect(toast.success).toHaveBeenCalledWith("Saved", expect.objectContaining({
      id: "clark-notice",
      position: "bottom-center",
      duration: CLARK_TOAST_DURATION.notice,
    }));
    expect(toast.warning).toHaveBeenCalledWith("Sync delayed", expect.objectContaining({
      id: "clark-warning",
      position: "bottom-center",
      duration: CLARK_TOAST_DURATION.warning,
    }));
    expect(toast).toHaveBeenCalledWith("125%", expect.objectContaining({
      id: "clark-text-size",
      position: "top-right",
      dismissible: false,
    }));
  });
});
