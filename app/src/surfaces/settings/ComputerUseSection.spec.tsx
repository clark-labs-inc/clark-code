import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type {
  ComputerUseActionReceipt,
  ComputerUseAppApproval,
} from "../../core-bridge/bridge";
import {
  computerUseSupportMessage,
  ComputerUseApprovalRows,
  ComputerUseReceiptRows,
} from "./ComputerUseSection";

describe("ComputerUseSection", () => {
  it("distinguishes unsupported, rejected-helper, and ready boundaries", () => {
    expect(computerUseSupportMessage(null)).toContain("Checking native helper");
    expect(computerUseSupportMessage({
      supported: false,
      platform: "windows",
      helper_ready: false,
    })).toBe("Native computer use is unavailable on windows.");
    expect(computerUseSupportMessage({
      supported: true,
      platform: "macos",
      helper_ready: false,
      detail: "helper signature rejected",
    })).toBe("helper signature rejected");
    expect(computerUseSupportMessage({
      supported: true,
      platform: "macos",
      helper_ready: true,
    })).toBe("The signed helper is ready.");
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
