import { mkdir } from "node:fs/promises";
import net from "node:net";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

import { launch, VIEWPORT } from "./launch.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.dirname(harnessDir);
const port = Number(process.env.CLARK_SECURITY_UI_PORT || 1432);
const appUrl = `http://127.0.0.1:${port}`;
const artifactDir =
  process.env.CLARK_SECURITY_ARTIFACT_DIR
  || path.join("/tmp", "clark-security-simulation");
const storageKey = "clark-desktop:security-simulation";

function waitForPort(targetPort, child, output) {
  return new Promise((resolve, reject) => {
    const deadline = Date.now() + 30_000;
    const probe = () => {
      if (child.exitCode !== null) {
        reject(new Error(`Vite exited before it was ready:\n${output()}`));
        return;
      }
      const socket = net.createConnection({ host: "127.0.0.1", port: targetPort });
      socket.once("connect", () => {
        socket.destroy();
        resolve();
      });
      socket.once("error", () => {
        socket.destroy();
        if (Date.now() >= deadline) {
          reject(new Error(`Vite did not listen on ${targetPort}:\n${output()}`));
        } else {
          setTimeout(probe, 150);
        }
      });
    };
    probe();
  });
}

function seedBrowser() {
  localStorage.clear();
  localStorage.setItem("clark.auth.session", JSON.stringify({
    user: {
      id: "security-ui-simulation",
      name: "Security Simulation",
      email: "security-simulation@clark.local",
      method: "local",
    },
    clark: { endpoint: "ws://127.0.0.1:8400/ws", token: "fixture-session" },
  }));
  localStorage.setItem("clark-desktop:local-agent", JSON.stringify({
    cwd: "/tmp/security-vulnerable-repo",
    model: "z-ai/glm-5.2",
    reasoningEffort: "",
    apiKey: "fixture-not-a-real-key",
  }));
  localStorage.setItem("clark-desktop:security-simulation", "populated");
}

async function attachLocalSession(page) {
  await page.waitForFunction(() => window.__clarkStore?.getState().bridge);
  await page.evaluate(async () => {
    const store = window.__clarkStore;
    const state = store.getState();
    const session = await state.bridge.newSession("local", {
      cwd: "/tmp/security-vulnerable-repo",
      collaboration_mode: "default",
    });
    const now = Date.now();
    store.setState({
      session,
      activeProvider: "local",
      activeProjectRoot: "/tmp/security-vulnerable-repo",
      conversations: [{
        id: session.id,
        title: "Adversarial Security simulation",
        provider: "local",
        project: "/tmp/security-vulnerable-repo",
        createdAt: now,
        updatedAt: now,
      }],
      conversationsLoading: false,
    });
  });
}

let output = "";
const vite = spawn(
  "corepack",
  ["pnpm@10", "--dir", "app", "dev", "--host", "127.0.0.1", "--port", String(port)],
  { cwd: repoDir, env: process.env, stdio: ["ignore", "pipe", "pipe"] },
);
const collect = (chunk) => {
  output = `${output}${chunk}`.slice(-20_000);
};
vite.stdout.on("data", collect);
vite.stderr.on("data", collect);

let browser;
try {
  await mkdir(artifactDir, { recursive: true });
  await waitForPort(port, vite, () => output);
  browser = await launch();
  const context = await browser.newContext({ viewport: VIEWPORT, reducedMotion: "reduce" });
  await context.addInitScript(seedBrowser);
  const page = await context.newPage();
  const browserErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push(message.text());
  });
  page.on("pageerror", (error) => browserErrors.push(error.message));

  await page.goto(appUrl, { waitUntil: "domcontentloaded" });
  await attachLocalSession(page);

  const securityButton = page.getByRole("button", { name: "Show Security scans" });
  await securityButton.waitFor({ timeout: 10_000 });
  await securityButton.click();
  await page.getByText("adversarial-standard", { exact: true }).waitFor();
  await page.getByText("4 findings · 20 reviewed · 1 excluded", { exact: true }).waitFor();
  await page.getByText("4 deep passes", { exact: false }).waitFor();
  await page.getByText("adversarial-standard", { exact: true }).click();
  await page
    .getByText("Request body controls administrative access.", { exact: true })
    .first()
    .waitFor();
  const populatedScreenshot = path.join(artifactDir, "security-history-populated.png");
  await page.screenshot({ path: populatedScreenshot, fullPage: true });

  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Show Security scans" }).waitFor();
  await page.evaluate((key) => localStorage.setItem(key, "empty"), storageKey);
  await securityButton.click();
  await page.getByText("No scans yet", { exact: true }).waitFor();

  await page.keyboard.press("Escape");
  await page.evaluate((key) => localStorage.setItem(key, "error"), storageKey);
  await securityButton.click();
  await page.getByText("Simulated unreadable Security artifact", { exact: true }).waitFor();
  const errorScreenshot = path.join(artifactDir, "security-history-error.png");
  await page.screenshot({ path: errorScreenshot, fullPage: true });

  await page.keyboard.press("Escape");
  await page.evaluate(() => {
    window.__clarkStore.setState({
      activeRemote: { id: "remote-simulation" },
    });
  });
  if (await page.getByRole("button", { name: "Show Security scans" }).count()) {
    throw new Error("Security history must be hidden for remote sessions");
  }

  if (browserErrors.length) {
    throw new Error(`browser errors: ${browserErrors.slice(0, 5).join(" | ")}`);
  }
  console.log(JSON.stringify({
    simulation: "clark-security-ui-v1",
    journeys: ["populated", "expand-findings", "escape-close", "empty", "error", "remote-hidden"],
    screenshots: [populatedScreenshot, errorScreenshot],
    status: "passed",
  }, null, 2));
} finally {
  await browser?.close();
  vite.kill("SIGTERM");
}
