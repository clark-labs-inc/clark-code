import { execFileSync } from "node:child_process";
import { once } from "node:events";
import { setTimeout as sleep } from "node:timers/promises";

/** Stop a pnpm/Vite process and every child it owns, then reap the leader. */
export async function stopProcessTree(child) {
  if (!child || child.exitCode !== null) return;
  try {
    if (process.platform === "win32") {
      execFileSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], {
        stdio: "ignore",
      });
    } else if (child.pid) {
      process.kill(-child.pid, "SIGTERM");
    }
  } catch {
    // The child may have exited between the status check and the tree kill.
  }
  await Promise.race([
    once(child, "exit"),
    sleep(5_000),
  ]);
  if (child.exitCode === null) {
    try {
      if (process.platform === "win32") {
        execFileSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], {
          stdio: "ignore",
        });
      } else if (child.pid) {
        process.kill(-child.pid, "SIGKILL");
      }
    } catch {
      // The process was already reaped.
    }
  }
}
