import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

class MemoryStorage {
  private values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }
}

const local = new MemoryStorage();
Object.defineProperty(globalThis, "localStorage", { value: local });
Object.defineProperty(globalThis, "window", {
  value: { dispatchEvent: vi.fn() },
});
Object.defineProperty(globalThis, "crypto", {
  value: { randomUUID: () => "host-1" },
});

import {
  organizationForRepository,
  setOrganizationForRepository,
} from "./organizationKnowledge";
import {
  setProjectKnowledgeEnabled,
  syncRepositoryHistory,
} from "./repositoryKnowledge";

const repository = {
  fingerprint: "repo-1",
  vcs: "git" as const,
  root: "/work/repo",
  head_oid: "a".repeat(40),
  current_branch: "main",
  default_branch: "main",
  canonical_remote: "github.com/acme/repo",
  remotes: [],
  commit_count: 1,
  shallow: false,
  dirty: false,
  refs_fingerprint: "refs-1",
};

describe("repository-scoped organization contribution", () => {
  beforeEach(() => {
    invoke.mockReset();
    setProjectKnowledgeEnabled(true);
    setOrganizationForRepository("repo-1", null);
  });

  it("is private until this exact repository is explicitly selected", () => {
    expect(organizationForRepository("repo-1")).toBeNull();
    setOrganizationForRepository("repo-1", "org-1");
    expect(organizationForRepository("repo-1")).toBe("org-1");
    expect(organizationForRepository("another-repo")).toBeNull();
  });

  it("routes an opted-in batch through the organization endpoint once", async () => {
    setProjectKnowledgeEnabled(true, "id:account-one");
    setOrganizationForRepository("repo-1", "org-1", "id:account-one");
    invoke.mockImplementation(async (command: string) => {
      if (command === "clark_repository_inspect") return repository;
      if (command === "clark_repository_history") {
        return {
          repository,
          offset: 0,
          next_offset: 1,
          complete: true,
          commits: [{
            oid: "a".repeat(40),
            parent_oids: [],
            author_name: "Ada",
            author_email: "ada@example.com",
            authored_at: "2026-07-14T00:00:00Z",
            committed_at: "2026-07-14T00:00:00Z",
            subject: "Ship organization memory",
            body: "",
          }],
        };
      }
      if (command === "desktop_organization_repository_sync") {
        return { next_offset: 1, complete: true, reset_required: false };
      }
      throw new Error(`unexpected command: ${command}`);
    });

    await syncRepositoryHistory(
      { accountScope: "id:account-one" },
      repository.root,
    );

    const commands = invoke.mock.calls.map(([command]) => command);
    expect(commands).toContain("desktop_organization_repository_sync");
    expect(commands).not.toContain("desktop_code_repository_sync");
    expect(invoke).toHaveBeenCalledWith(
      "desktop_organization_repository_sync",
      expect.objectContaining({ organizationId: "org-1", hostId: "host-1" }),
    );
  });
});
