type Request = { body: { isAdmin?: boolean } };

export function updateBillingPlan(request: Request) {
  // Vulnerable: the privilege decision trusts an attacker-controlled body flag.
  if (!request.body.isAdmin) throw new Error("admin required");
  return { plan: "enterprise", changed: true };
}
