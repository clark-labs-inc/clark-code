#!/usr/bin/env node

import { createHash, randomUUID } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";

const ADAPTER_SERVICE = "scout-adapter-v1";
const EXEC_PROTOCOL_VERSION = 9;
const ADAPTER_PROTOCOL_VERSION = 3;
const RUNTIME_PROTOCOL_VERSION = 3;

function valueArg(args, name) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  if (index < 0) return undefined;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function requireArg(args, name) {
  const value = valueArg(args, name);
  if (!value) throw new Error(`${name} is required`);
  return value;
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function digestJson(value) {
  return sha256(JSON.stringify(value));
}

function uuid(prefix) {
  return `${prefix}:${randomUUID()}`;
}

function responseFailure(response) {
  return response?.response?.failure?.code || "invalid_response";
}

class ExecClient {
  constructor(url, token) {
    this.url = url;
    this.token = token;
    this.nextId = 1;
    this.pending = new Map();
  }

  async connect() {
    this.socket = new WebSocket(this.url);
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(String(event.data));
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(message.error.message));
      else pending.resolve(message.result);
    });
    this.socket.addEventListener("close", () => {
      for (const pending of this.pending.values()) {
        pending.reject(new Error("exec-server connection closed"));
      }
      this.pending.clear();
    });
    await new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", reject, { once: true });
    });
    await this.call("auth", {
      token: this.token,
      protocol_version: EXEC_PROTOCOL_VERSION,
    });
  }

  call(method, params) {
    const id = this.nextId++;
    const request = { jsonrpc: "2.0", id, method, params };
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify(request));
    });
  }

  async adapter(root, request) {
    const result = await this.call("targetService/call", {
      service: ADAPTER_SERVICE,
      root,
      request: Buffer.from(JSON.stringify(request)).toString("base64"),
    });
    return JSON.parse(Buffer.from(result.response, "base64").toString("utf8"));
  }

  close() {
    this.socket.close();
  }
}

function fetchSpec(provider) {
  if (provider === "github") {
    return {
      operation: "list_repositories",
      provider_resource_type: "github.repository",
      resource_kind: "repository",
      region_or_project: "global",
      projection: [
        "archived",
        "default_branch",
        "disabled",
        "fork",
        "full_name",
        "html_url",
        "name",
        "owner_login",
        "private",
        "visibility",
      ],
    };
  }
  if (provider === "aws") {
    return {
      operation: "list_organization_accounts",
      provider_resource_type: "aws.organizations.account",
      resource_kind: "account",
      region_or_project: "global",
      projection: ["arn", "email", "id", "joined_method", "name", "state"],
    };
  }
  return null;
}

function firstPageRequest({ target, targetFingerprint, auth, spec, sequence }) {
  return {
    protocol_version: ADAPTER_PROTOCOL_VERSION,
    request_id: uuid("request"),
    target_id: target.target_id,
    target_identity_sha256: targetFingerprint,
    adapter_id: auth.adapter_id,
    auth_context_handle: auth.handle,
    auth_context_id: auth.context_id,
    coverage: {
      enterprise_id: "live-qualification",
      charter_id: "live-read-only-control-planes",
      discovery_epoch: 1,
      sequence,
      adapter_id: auth.adapter_id,
      auth_context_id: auth.context_id,
      tenant: auth.authority_scope,
      region_or_project: spec.region_or_project,
      resource_kind: spec.resource_kind,
    },
    query: {
      operation: spec.operation,
      authority_scope: auth.authority_scope,
      provider_resource_type: spec.provider_resource_type,
      filters: {},
      projection: spec.projection,
      page_size: 100,
    },
    page_ordinal: 0,
    cursor_handle: null,
    limits: {
      max_records: 100,
      max_response_bytes: 16 * 1024 * 1024,
      max_duration_ms: 60_000,
    },
    requested_at_ms: Date.now(),
  };
}

function nextPageRequest(receipt) {
  return {
    ...receipt.request,
    request_id: uuid("request"),
    page_ordinal: receipt.request.page_ordinal + 1,
    cursor_handle: receipt.next_cursor_handle,
    requested_at_ms: Date.now(),
  };
}

async function fetchAll(client, root, firstRequest, maxPages) {
  let request = firstRequest;
  const recordIds = new Set();
  const pageDigests = [];
  let pages = 0;
  let sourceRecordsSeen = 0;
  let terminalOutcome = null;
  while (pages < maxPages) {
    const response = await client.adapter(root, {
      action: "fetch_page",
      request,
    });
    if (response.result !== "fetch_page" || response.response?.status !== "succeeded") {
      return {
        status: "failed",
        failure_code: responseFailure(response),
        pages,
        records: recordIds.size,
      };
    }
    const receipt = response.response.receipt;
    pages += 1;
    pageDigests.push(receipt.safe_page_sha256);
    sourceRecordsSeen += receipt.redaction_summary.source_records_seen;
    for (const record of receipt.records) recordIds.add(record.record_id);
    terminalOutcome = receipt.outcome;
    if (!receipt.next_cursor_handle) break;
    request = nextPageRequest(receipt);
  }
  const cursorRemaining = request.page_ordinal + 1 >= maxPages
    && terminalOutcome?.status === "succeeded"
    && terminalOutcome?.final_page === false;
  return {
    status: cursorRemaining ? "bounded" : "complete",
    pages,
    records: recordIds.size,
    source_records_seen: sourceRecordsSeen,
    record_ids_sha256: sha256([...recordIds].sort().join("\n")),
    page_digests_sha256: sha256(pageDigests.join("\n")),
    terminal_outcome: terminalOutcome?.status || "unknown",
    cursor_remaining: cursorRemaining,
  };
}

async function qualifyCandidate({
  client,
  root,
  census,
  candidate,
  authority,
  sequence,
  maxPages,
}) {
  const base = {
    provider: candidate.provider,
    adapter_id: candidate.adapter_id,
    source: candidate.source,
    candidate_handle_sha256: sha256(candidate.handle),
  };
  if (!authority && candidate.provider === "github") {
    return { ...base, status: "gap", failure_code: "authority_not_supplied" };
  }
  const targetFingerprint = digestJson(census.target);
  const verify = await client.adapter(root, {
    action: "verify_auth",
    request: {
      runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
      target_id: census.target.target_id,
      target_identity_sha256: targetFingerprint,
      candidate_handle: candidate.handle,
      adapter_id: candidate.adapter_id,
      requested_authority_scope: authority || null,
    },
  });
  if (verify.result !== "verify_auth" || verify.response?.status !== "succeeded") {
    return {
      ...base,
      status: "unverified",
      failure_code: responseFailure(verify),
    };
  }
  const auth = verify.response.auth_context;
  const spec = fetchSpec(candidate.provider);
  const verified = {
    ...base,
    status: "verified",
    authority_sha256: sha256(auth.authority_scope),
    principal_sha256: sha256(auth.principal_native_id),
    grant_boundary_sha256: auth.grant_boundary_sha256,
  };
  if (!spec) return { ...verified, collection: { status: "unsupported" } };
  const collection = await fetchAll(
    client,
    root,
    firstPageRequest({
      target: census.target,
      targetFingerprint,
      auth,
      spec,
      sequence,
    }),
    maxPages,
  );
  return { ...verified, collection };
}

async function main() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Usage:
  CLARK_EXEC_TOKEN=... SCOUT_GITHUB_AUTHORITY=... node \
    harness/scout-adapter-live-qualification.mjs \
    --url ws://127.0.0.1:PORT --root TARGET_PATH --out NEW_DIRECTORY

The token and optional GitHub authority are accepted only through environment
variables. The receipt contains status classes, counts, and hashes; it never
contains credentials, principal ids, authority names, records, or cursors.`);
    return;
  }
  const url = requireArg(args, "--url");
  const root = requireArg(args, "--root");
  const output = path.resolve(requireArg(args, "--out"));
  const token = process.env.CLARK_EXEC_TOKEN;
  if (!token) throw new Error("CLARK_EXEC_TOKEN is required");
  const githubAuthority = process.env.SCOUT_GITHUB_AUTHORITY || null;
  const maxPages = Number(valueArg(args, "--max-pages") || "10000");
  if (!Number.isSafeInteger(maxPages) || maxPages < 1 || maxPages > 10000) {
    throw new Error("--max-pages must be between 1 and 10000");
  }
  mkdirSync(output, { recursive: false, mode: 0o700 });
  const client = new ExecClient(url, token);
  await client.connect();
  try {
    const response = await client.adapter(root, {
      action: "census",
      request: { runtime_protocol_version: RUNTIME_PROTOCOL_VERSION },
    });
    if (response.result !== "census" || response.response?.status !== "succeeded") {
      throw new Error(`adapter census failed safely: ${responseFailure(response)}`);
    }
    const census = response.response;
    const candidates = [];
    for (let index = 0; index < census.candidates.length; index += 1) {
      const candidate = census.candidates[index];
      candidates.push(await qualifyCandidate({
        client,
        root,
        census,
        candidate,
        authority: candidate.provider === "github" ? githubAuthority : null,
        sequence: index + 1,
        maxPages,
      }));
    }
    const allCandidatesTerminal = candidates.every((candidate) =>
      candidate.status === "verified"
        ? candidate.collection?.status === "complete"
        : ["unverified", "gap"].includes(candidate.status));
    const verifiedCandidateCount = candidates.filter(
      (candidate) => candidate.status === "verified",
    ).length;
    const completeCollectionCount = candidates.filter(
      (candidate) =>
        candidate.status === "verified"
        && candidate.collection?.status === "complete",
    ).length;
    const gapCount = candidates.length - verifiedCandidateCount;
    const receipt = {
      schema_version: "scout-live-adapter-qualification-v1",
      status:
        allCandidatesTerminal && completeCollectionCount > 0
          ? "passed"
          : "failed",
      generated_at: new Date().toISOString(),
      read_only: true,
      mutating_provider_requests: 0,
      credential_values_emitted: false,
      provider_cursors_emitted: false,
      target: {
        target_id: census.target.target_id,
        target_identity_sha256: digestJson(census.target),
        platform: census.target.platform,
        architecture: census.target.architecture,
      },
      census: {
        candidate_count: census.candidates.length,
        verified_candidate_count: verifiedCandidateCount,
        complete_collection_count: completeCollectionCount,
        gap_count: gapCount,
        available_tools: census.tools
          .filter((tool) => tool.available)
          .map((tool) => tool.tool)
          .sort(),
        unavailable_tools: census.tools
          .filter((tool) => !tool.available)
          .map((tool) => tool.tool)
          .sort(),
      },
      candidates,
    };
    const serialized = `${JSON.stringify(receipt, null, 2)}\n`;
    writeFileSync(path.join(output, "receipt.json"), serialized, { mode: 0o600 });
    console.log(JSON.stringify({
      status: receipt.status,
      candidates: candidates.map((candidate) => ({
        provider: candidate.provider,
        source: candidate.source,
        status: candidate.status,
        failure_code: candidate.failure_code,
        collection: candidate.collection,
      })),
      receipt: path.join(output, "receipt.json"),
    }));
    if (receipt.status !== "passed") process.exitCode = 1;
  } finally {
    client.close();
  }
}

await main();
