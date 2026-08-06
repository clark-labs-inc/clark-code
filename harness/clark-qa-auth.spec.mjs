import assert from "node:assert/strict";
import test from "node:test";

import {
  assertClarkOwnedQaEmail,
  mintClarkQaSession,
} from "./clark-qa-auth.mjs";

test("accepts a Clark-owned QA identity", () => {
  assert.equal(
    assertClarkOwnedQaEmail("clark-code-vm-qa@clarkslabs.com"),
    "clark-code-vm-qa@clarkslabs.com",
  );
});

test("rejects identities outside the Clark-owned domain", () => {
  assert.throws(
    () => assertClarkOwnedQaEmail("clark-code-vm-qa@customer.example"),
    /must use the Clark-owned clarkslabs\.com domain/,
  );
});

test("rejects malformed identities before checking ownership", () => {
  assert.throws(
    () => assertClarkOwnedQaEmail("not-an-email"),
    /not a valid email address/,
  );
});

test("refuses a non-Clark identity before contacting auth", async () => {
  let fetchCalled = false;
  await assert.rejects(
    mintClarkQaSession({
      credentials: {
        name: "Autonomous VM QA",
        email: "clark-code-vm-qa@customer.example",
        password: "not-a-real-password",
      },
      fetchImpl: async () => {
        fetchCalled = true;
        throw new Error("must not be called");
      },
    }),
    /must use the Clark-owned clarkslabs\.com domain/,
  );
  assert.equal(fetchCalled, false);
});
