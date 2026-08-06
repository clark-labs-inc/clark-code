import { createCipheriv, randomBytes } from "node:crypto";

const MAGIC = Buffer.from("CLKCRD02");
const AAD = Buffer.from("clark-desktop-credentials-v2");

/** Build the exact app-owned encrypted credential files used by Clark Desktop. */
export function nativeCredentialEnvelope(retainedAuth) {
  if (
    retainedAuth?.version !== 2
    || !retainedAuth?.descriptor?.user?.id
    || !retainedAuth?.authOrigin
    || !retainedAuth?.clarkToken
  ) {
    throw new Error("native credential bootstrap requires a complete v2 retained auth record");
  }
  const key = randomBytes(32);
  const nonce = randomBytes(12);
  const plaintext = Buffer.from(JSON.stringify({
    version: 2,
    retained_auth: JSON.stringify(retainedAuth),
    code_keys: {},
    mcp_env: {},
  }));
  const cipher = createCipheriv("chacha20-poly1305", key, nonce, {
    authTagLength: 16,
  });
  cipher.setAAD(AAD, { plaintextLength: plaintext.length });
  const sealed = Buffer.concat([
    cipher.update(plaintext),
    cipher.final(),
    cipher.getAuthTag(),
  ]);
  return {
    key: key.toString("base64"),
    envelope: Buffer.concat([MAGIC, nonce, sealed]).toString("base64"),
  };
}
