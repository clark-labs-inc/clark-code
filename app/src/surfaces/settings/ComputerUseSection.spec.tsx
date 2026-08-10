import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type {
  ComputerUseActionReceipt,
  ComputerUseAppApproval,
} from "../../core-bridge/bridge";
import {
  computerUseRepairMessage,
  computerUseSupportMessage,
  ComputerUseApprovalRows,
  ComputerUseReceiptRows,
} from "./ComputerUseSection";

describe("ComputerUseSection", () => {
  it("distinguishes unsupported, rejected-service, permission, and ready boundaries", () => {
    expect(computerUseSupportMessage(null)).toContain("native service");
    expect(computerUseSupportMessage({
      supported: false,
      platform: "windows",
      service_ready: false,
      readiness: "unsupported",
    })).toBe("Native computer use is unavailable on windows.");
    expect(computerUseSupportMessage({
      supported: true,
      platform: "macos",
      service_ready: false,
      readiness: "service_unavailable",
      detail: "service signature rejected",
    })).toBe("service signature rejected");
    expect(computerUseSupportMessage({
      supported: true,
      platform: "macos",
      service_ready: true,
      readiness: "needs_permission",
      permission_owner: {
        display_name: "the agent Computer Use",
        bundle_id: "org.agentdesktop.computer-use",
      },
    })).toBe("the agent Computer Use needs macOS privacy access.");
    expect(computerUseSupportMessage({
      supported: true,
      platform: "linux",
      service_ready: true,
      readiness: "needs_permission",
      permission_owner: {
        display_name: "the agent Computer Use Service",
        bundle_id: "org.agentdesktop.ComputerUse",
      },
    })).toBe("the agent Computer Use Service needs desktop capture and input access.");
    expect(computerUseSupportMessage({
      supported: true,
      platform: "macos",
      service_ready: true,
      readiness: "ready",
    })).toBe("The signed computer-use service is ready.");
    expect(computerUseSupportMessage({
      supported: true,
      platform: "windows",
      service_ready: true,
      readiness: "ready",
    })).toBe("The isolated computer-use service is ready.");
  });

  it("gives platform-specific repair guidance for the permission-owning service", () => {
    expect(computerUseRepairMessage({
      supported: true,
      platform: "macos",
      service_ready: true,
      readiness: "needs_permission",
      permission_owner: {
        display_name: "the agent Computer Use Dev",
        bundle_id: "org.agentdesktop.computer-use.dev",
      },
    })).toBe(
      "Grant access to the agent Computer Use Dev. Existing Clark Code privacy grants do not transfer to the separately identified service.",
    );
    expect(computerUseRepairMessage({
      supported: true,
      platform: "windows",
      service_ready: true,
      readiness: "needs_permission",
    })).toContain("signed-in desktop session");
    expect(computerUseRepairMessage({
      supported: true,
      platform: "linux",
      service_ready: true,
      readiness: "needs_permission",
    })).toContain("X11 or XWayland");
  });

  it("shows signer-bound approvals with an exact revocation target", () => {
    const approvals: ComputerUseAppApproval[] = [{
      identity_key: "signer-bound-identity",
      bundle_id: "com.apple.TextEdit",
      app_name: "TextEdit",
      team_identifier: "APPLE",
      granted_at_ms: 1_700_000_000_000,
      last_used_at_ms: 1_700_000_100_000,
    }];
    const markup = renderToStaticMarkup(
      <ComputerUseApprovalRows
        approvals={approvals}
        working={null}
        onRevoke={vi.fn()}
      />,
    );
    expect(markup).toContain("TextEdit");
    expect(markup).toContain("com.apple.TextEdit");
    expect(markup).toContain("Team APPLE");
    expect(markup).toContain('aria-label="Revoke TextEdit"');
    expect(markup).not.toContain("signer-bound-identity");
  });

  it("renders only bounded redacted receipt summaries", () => {
    const receipts: ComputerUseActionReceipt[] = Array.from(
      { length: 6 },
      (_, index) => ({
        receipt_id: `receipt-${index}`,
        prepared_action_id: `prepared-${index}`,
        application_identity_key: "identity",
        bundle_id: "com.apple.TextEdit",
        pid: 42,
        window_id: 7,
        action_kind: "type_text",
        disposition: "allow",
        outcome: "succeeded",
        payload_summary: `text redacted (${index} characters)`,
        completed_at_ms: 1_700_000_000_000 + index,
        persisted: true,
      }),
    );
    const markup = renderToStaticMarkup(
      <ComputerUseReceiptRows receipts={receipts} />,
    );
    expect(markup).toContain("text redacted (5 characters)");
    expect(markup).toContain("text redacted (1 characters)");
    expect(markup).not.toContain("text redacted (0 characters)");
    expect(markup).not.toContain("prepared-5");
    expect(markup).not.toContain("identity");
  });
});
