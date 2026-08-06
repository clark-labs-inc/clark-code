// Deliberately fake sentinel; it has never been a valid credential.
export const PAYMENT_API_KEY = "sk_test_CLARK_SECURITY_FIXTURE_NOT_A_SECRET";

export function paymentHeaders() {
  // Vulnerable pattern: a credential is embedded in tracked source.
  return { Authorization: `Bearer ${PAYMENT_API_KEY}` };
}
