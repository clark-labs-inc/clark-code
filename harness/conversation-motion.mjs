// Repeatable motion review of the real UI with synthetic local snapshots.
// Run before edits with --baseline; normal runs assert the motion contracts.
// No hosted providers, account data, or persisted conversations are used.
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";
import { chromium, webkit } from "playwright";

const root = new URL("../", import.meta.url).pathname;
const baseline = process.argv.includes("--baseline");
const phase = baseline ? "before" : "after";
const out = `${root}docs/conversation-motion/${phase}`;
mkdirSync(out, { recursive: true });
const port = await new Promise((resolve, reject) => {
  const server = createServer();
  server.once("error", reject);
  server.listen(0, "127.0.0.1", () => {
    const { port } = server.address();
    server.close(() => resolve(port));
  });
});
const url = `http://127.0.0.1:${port}`;
const vite = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
  cwd: `${root}app`, env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" }, stdio: "ignore",
});
const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="720" height="360" viewBox="0 0 720 360"><defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="#e9e4d8"/><stop offset="1" stop-color="#b3c2bb"/></linearGradient></defs><rect width="720" height="360" rx="22" fill="url(#g)"/><circle cx="575" cy="50" r="170" fill="#879d91" opacity=".24"/><rect x="40" y="36" width="640" height="288" rx="18" fill="#fcfbf7"/><rect x="40" y="36" width="150" height="288" rx="18" fill="#efeee7"/><circle cx="65" cy="62" r="5" fill="#729584"/><text x="80" y="67" font-family="sans-serif" font-size="14" fill="#394b40">Workspace</text><rect x="60" y="92" width="108" height="22" rx="5" fill="#dce2d8"/><text x="220" y="85" font-family="sans-serif" font-size="23" fill="#273e32">Room to think.</text><text x="220" y="112" font-family="sans-serif" font-size="12" fill="#738078">A calmer place for your next idea.</text><rect x="220" y="145" width="205" height="140" rx="12" fill="#e4eadd"/><rect x="440" y="145" width="210" height="65" rx="12" fill="#eee7dc"/><rect x="440" y="224" width="210" height="61" rx="12" fill="#e6e9e7"/><path d="M244 253L286 205L321 225L366 177L402 198" fill="none" stroke="#82947a" stroke-width="4" stroke-linecap="round"/></svg>`;
const image = Buffer.from(svg).toString("base64");
const results = [];

async function scene(page) {
  await page.evaluate(({ image }) => {
    const store = window.__agentDesktopStore;
    store.setState({ session: { id: "motion-review", provider: "local", capabilities: {} }, opening: null,
      snapshot: { ...store.getState().snapshot, session: "motion-review", runs: {}, tool_calls: {}, timeline: [
        { item: "message", role: "user", blocks: [{ type: "text", text: "Give this workspace a calmer, more modern feel." }] },
        { item: "message", role: "agent", blocks: [{ type: "text", text: "## Room to think\n\nKeep the workspace quiet and let motion explain what changed. Here is the direction I would build on." }] },
        { item: "message", role: "user", blocks: [{ type: "image", mime_type: "image/svg+xml", data: image }, { type: "text", text: "I like this direction. Make the interactions feel just as considered." }] },
      ] },
    });
  }, { image });
  await page.getByRole("button", { name: "View Attachment full size" }).waitFor();
  await sleep(500);
}

async function review(page, label, capture) {
  const thumb = page.getByRole("button", { name: "View Attachment full size" });
  const log = page.getByRole("log");
  const before = await thumb.boundingBox();
  const scroll = await log.evaluate(el => el.scrollTop);
  await thumb.hover();
  await sleep(300);
  const hover = await thumb.evaluate(el => ({ transform: getComputedStyle(el).transform, width: el.getBoundingClientRect().width }));
  if (capture) await page.screenshot({ path: `${out}/01-image-hover.png` });
  await thumb.click();
  const dialog = page.locator('[role="dialog"][aria-label="Image viewer: Attachment"]');
  await dialog.waitFor();
  await sleep(400);
  const viewport = await dialog.boundingBox();
  await page.keyboard.press("Shift+Tab");
  const trappedFocus = await dialog.evaluate(el => el.contains(document.activeElement));
  if (capture) await page.screenshot({ path: `${out}/02-image-viewer.png` });
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  await sleep(250);
  await page.getByRole("button", { name: "Fit to screen" }).click();
  await sleep(250);
  await page.keyboard.press("Escape");
  // Observe a real exit frame as well as final removal.
  await sleep(35);
  const exitOpacity = await dialog.count() ? await dialog.evaluate(el => Number(getComputedStyle(el).opacity)) : null;
  await dialog.waitFor({ state: "detached" });
  const restoredFocus = await thumb.evaluate(el => el === document.activeElement);
  const scrollShift = await log.evaluate((el, top) => el.scrollTop - top, scroll);
  await page.mouse.move(350, 150);
  await sleep(350);

  await page.evaluate(() => {
    const store = window.__agentDesktopStore;
    store.setState({ snapshot: { ...store.getState().snapshot, runs: { live: { id: "live", status: "running" } } } });
  });
  await sleep(650);
  const words = "Small details make the difference: a gentle lift on images, a soft reveal for new words, and a clean fade when a temporary surface leaves. Everything should settle quickly, so you can keep reading.".split(" ");
  for (let count = 3; count < words.length + 3; count += 3) {
    await page.evaluate(text => {
      const store = window.__agentDesktopStore;
      const snapshot = store.getState().snapshot;
      store.setState({ snapshot: { ...snapshot, timeline: [
        ...snapshot.timeline.slice(0, 3),
        { item: "message", role: "agent", run: "live", phase: "final_answer", blocks: [{ type: "text", text }] },
      ] } });
    }, words.slice(0, count).join(" "));
    await sleep(90);
  }
  await sleep(300);
  if (capture) await page.screenshot({ path: `${out}/03-streaming.png` });
  await page.evaluate(() => {
    const store = window.__agentDesktopStore;
    store.setState({ snapshot: { ...store.getState().snapshot, runs: { live: { id: "live", status: "done" } } } });
  });
  await sleep(450);
  await page.evaluate(() => window.__agentDesktopStore.setState({ error: "Motion review: this temporary notice can be dismissed." }));
  await page.getByText("Motion review: this temporary notice can be dismissed.").waitFor();
  await sleep(400);
  await page.getByRole("button", { name: "Dismiss", exact: true }).click();
  await sleep(400);
  let attachmentExit = null;
  if (!baseline) {
    await page.evaluate(image => window.__agentDesktopStore.setState({ attachments: [
      { id: "review-image", filename: "concept.svg", content_type: "image/svg+xml", data_base64: image, size: image.length, previewUrl: `data:image/svg+xml;base64,${image}` },
      { id: "review-notes", filename: "notes.txt", content_type: "text/plain", data_base64: "Tm90ZXM=", size: 5 },
    ] }), image);
    const rail = page.getByRole("list", { name: "Attachments", exact: true });
    await rail.waitFor();
    await sleep(300);
    if (capture) await page.screenshot({ path: `${out}/04-attachment-rail.png` });
    await page.getByRole("button", { name: "Remove concept.svg" }).click();
    await sleep(300);
    await page.getByRole("button", { name: "Remove notes.txt" }).click();
    await sleep(30);
    attachmentExit = await rail.count() ? await rail.evaluate(el => Number(getComputedStyle(el).opacity)) : null;
    await rail.waitFor({ state: "detached" });

    // Markdown images use the same viewer without changing lazy-load props.
    await page.route("https://motion-fixture.invalid/preview.svg", route => route.fulfill({ contentType: "image/svg+xml", body: svg }));
    await page.evaluate(() => {
      const store = window.__agentDesktopStore;
      const snapshot = store.getState().snapshot;
      store.setState({ snapshot: { ...snapshot, timeline: [
        ...snapshot.timeline,
        { item: "message", role: "agent", blocks: [{ type: "text", text: "![Workspace concept](https://motion-fixture.invalid/preview.svg)" }] },
      ] } });
    });
    const markdownImage = page.getByRole("button", { name: "View Workspace concept full size" });
    await markdownImage.click();
    await page.getByRole("dialog", { name: "Image viewer: Workspace concept" }).waitFor();
    await page.keyboard.press("Escape");
    await page.locator("[data-image-viewer]").waitFor({ state: "detached" });
    await page.evaluate(() => {
      const store = window.__agentDesktopStore;
      const snapshot = store.getState().snapshot;
      store.setState({ snapshot: { ...snapshot,
        artifacts: [{ id: "review-artifact", kind: "image", title: "Workspace concept.svg", uri: "https://motion-fixture.invalid/preview.svg" }],
        timeline: [...snapshot.timeline, { item: "artifact", id: "review-artifact" }],
      } });
    });
    await page.getByRole("button", { name: "View Workspace concept.svg full size" }).click();
    await page.getByRole("dialog", { name: "Image viewer: Workspace concept.svg" }).waitFor();
    await page.keyboard.press("Escape");
    await page.locator("[data-image-viewer]").waitFor({ state: "detached" });
  }
  const result = { label, hover, initialWidth: before.width, viewport, exitOpacity, restoredFocus, trappedFocus, scrollShift, attachmentExit };
  if (!baseline) {
    const reduce = label.endsWith("reduce");
    assert.ok(reduce ? Math.abs(hover.width - before.width) < 0.5 : hover.width > before.width + 1, `${label}: image hover policy`);
    assert.ok(restoredFocus, `${label}: closing the viewer lost thumbnail focus`);
    assert.ok(trappedFocus, `${label}: keyboard focus escaped the viewer`);
    assert.ok(exitOpacity !== null && exitOpacity > 0 && exitOpacity < 1, `${label}: viewer exit hard-cut`);
    assert.ok(Math.abs(scrollShift) < 2, `${label}: viewer moved the transcript`);
    assert.equal(viewport.x, 0, `${label}: viewer was trapped by a row transform`);
    assert.equal(viewport.y, 0, `${label}: viewer was trapped by a row transform`);
    assert.equal(viewport.width, 1280);
    assert.equal(viewport.height, 900);
    assert.ok(attachmentExit !== null && attachmentExit > 0 && attachmentExit < 1, `${label}: last attachment hard-cut`);
  }
  return result;
}

try {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (vite.exitCode !== null) throw new Error("Vite exited before startup");
    try { if ((await fetch(url)).ok) break; } catch {}
    await sleep(100);
  }
  for (const [name, engine] of Object.entries({ chromium, webkit })) {
    if (process.env.ENGINE && process.env.ENGINE !== name) continue;
    if (baseline && name !== "chromium") continue;
    const browser = await engine.launch({ headless: true });
    try {
      for (const reducedMotion of baseline ? ["no-preference"] : ["no-preference", "reduce"]) {
        const capture = name === "chromium" && reducedMotion === "no-preference";
        const rawVideo = `${root}target/conversation-motion/${phase}`;
        const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion,
          ...(capture ? { recordVideo: { dir: rawVideo, size: { width: 1280, height: 900 } } } : {}),
        });
        await context.addInitScript(() => {
          localStorage.setItem("agent-desktop.dev-account", JSON.stringify({ user: { id: "motion-review", name: "Motion Review", method: "local" } }));
        });
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", error => errors.push(error.message));
        await page.goto(url);
        await page.getByLabel("Message Clark Code").waitFor();
        await page.waitForFunction(() => !!window.__agentDesktopStore);
        let result;
        try {
          await scene(page);
          result = await review(page, `${name} ${reducedMotion}`, capture);
        } catch (error) {
          mkdirSync(`${root}target/conversation-motion`, { recursive: true });
          await page.screenshot({ path: `${root}target/conversation-motion/failure.png` });
          console.error(JSON.stringify({ label: `${name} ${reducedMotion}`, errors,
            visible: (await page.locator("body").innerText()).slice(0, 1500) }));
          throw error;
        }
        result.errors = errors;
        assert.deepEqual(errors, []);
        results.push(result);
        const video = page.video();
        await context.close();
        if (capture) {
          const source = await video.path();
          execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", source, "-c:v", "libx264", "-crf", "23", "-pix_fmt", "yuv420p", "-movflags", "+faststart", `${out}/review.mp4`]);
          execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", source, "-filter_complex", "fps=15,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer", `${out}/review.gif`]);
        }
        console.log(JSON.stringify(result));
      }
    } finally { await browser.close(); }
  }
  writeFileSync(`${out}/receipts.json`, `${JSON.stringify(results, null, 2)}\n`);
} finally { vite.kill("SIGTERM"); }
