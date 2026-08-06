import assert from "node:assert/strict";
import test from "node:test";

import {
  parseDotEnv,
  parseUtmList,
  resolveValue,
} from "./release-environment-preflight.mjs";

test("preflight parses only allowlisted dotenv names without executing values", () => {
  const values = parseDotEnv(`
    export CLARK_CODE_API_KEY="clark-key"
    OPENROUTER_API_KEY='openrouter-key'
    NOT_A_SECRET=ignored
    EXECUTE_ME=$(touch /tmp/should-not-exist)
  `);
  assert.deepEqual(values, {
    CLARK_CODE_API_KEY: "clark-key",
    OPENROUTER_API_KEY: "openrouter-key",
  });
});

test("process values take precedence over ignored env files", () => {
  const resolved = resolveValue("CLARK_CODE_API_KEY", [
    { id: "process", values: { CLARK_CODE_API_KEY: "process-key" }, mode: null },
    { id: "desktop", path: "/tmp/.env", values: { CLARK_CODE_API_KEY: "file-key" }, mode: 0o600 },
  ]);
  assert.equal(resolved.value, "process-key");
  assert.equal(resolved.source, "process");
});

test("remote credential names are never treated as credentials themselves", () => {
  const resolved = resolveValue("CLARK_REMOTE_CPU_CREDENTIAL_ENV", [
    {
      id: "desktop",
      path: "/tmp/.env",
      values: {
        CLARK_REMOTE_CPU_CREDENTIAL_ENV: "CLARK_CODE_API_KEY",
        CLARK_CODE_API_KEY: "resolved-key",
      },
      mode: 0o600,
    },
  ]);
  assert.equal(resolved.value, "CLARK_CODE_API_KEY");
  assert.equal(resolveValue(resolved.value, [
    {
      id: "desktop",
      path: "/tmp/.env",
      values: { CLARK_CODE_API_KEY: "resolved-key" },
      mode: 0o600,
    },
  ]).value, "resolved-key");
});

test("UTM parsing retains exact VM names and statuses", () => {
  assert.deepEqual(
    parseUtmList(`UUID                                 Status   Name
95A632BC-CCB1-4EE4-95F0-8AD7609DECF6 started  Clark QA - Windows 11 ARM
F7B555EF-F2BB-463D-9702-9C8BA84C446A stopped  Clark QA - Ubuntu 24.04 Desktop
`),
    [
      {
        uuid: "95A632BC-CCB1-4EE4-95F0-8AD7609DECF6",
        status: "started",
        name: "Clark QA - Windows 11 ARM",
      },
      {
        uuid: "F7B555EF-F2BB-463D-9702-9C8BA84C446A",
        status: "stopped",
        name: "Clark QA - Ubuntu 24.04 Desktop",
      },
    ],
  );
});
