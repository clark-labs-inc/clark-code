type Claims = { sub: string; tenantId: string; admin?: boolean };

export function authenticate(token: string): Claims {
  // Vulnerable: this decodes attacker data but never verifies its signature.
  const payload = token.split(".")[1];
  return JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
}
