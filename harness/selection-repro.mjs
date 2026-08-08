// Repro: "selected text often can't be unselected by clicking empty areas".
// Boots the app (vite dev + mock bridge), runs a fake conversation, drag-selects
// text in the agent reply, then clicks a battery of "empty area" points and
// reports, for each: what element received the click, its computed user-select,
// whether mousedown got preventDefaulted, and whether the selection survived.
//
// Usage: node harness/selection-repro.mjs [chromium|webkit]  (default chromium)
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { chromium, webkit } from "playwright";

const engine = process.argv[2] === "webkit" ? webkit : chromium;
const engineName = process.argv[2] === "webkit" ? "webkit" : "chromium";
const root = fileURLToPath(new URL("..", import.meta.url));
const port = Number(process.env.SELECTION_REPRO_PORT ?? 4177);
const url = `http://127.0.0.1:${port}/`;

const dev = spawn(
  "pnpm",
  ["--dir", "app", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  {
    cwd: root,
    env: { ...process.env, VITE_PRODUCT_DEV_AUTH: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let devOutput = "";
dev.stdout.on("data", (c) => (devOutput += c));
dev.stderr.on("data", (c) => (devOutput += c));

async function waitForServer() {
  for (let i = 0; i < 120; i++) {
    if (dev.exitCode != null) throw new Error(`vite exited early\n${devOutput}`);
    try {
      const r = await fetch(url);
      if (r.ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error(`vite did not start\n${devOutput}`);
}

function selectionState(page) {
  return page.evaluate(() => {
    const s = window.getSelection();
    return { len: s.toString().length, collapsed: s.isCollapsed, text: s.toString().slice(0, 60) };
  });
}

async function clickProbe(page, name, x, y) {
  const info = await page.evaluate(
    ([px, py]) => {
      const el = document.elementFromPoint(px, py);
      if (!el) return { found: false };
      const chain = [];
      let node = el;
      for (let i = 0; i < 5 && node; i++) {
        const cs = getComputedStyle(node);
        chain.push({
          tag: node.tagName.toLowerCase(),
          cls: (node.className?.baseVal ?? node.className ?? "").toString().slice(0, 80),
          userSelect: cs.userSelect,
          pointerEvents: cs.pointerEvents,
        });
        node = node.parentElement;
      }
      return { found: true, chain };
    },
    [x, y],
  );
  // Track whether anything preventDefaults this mousedown.
  await page.evaluate(() => {
    window.__mdPrevented = false;
    const mark = () => {
      window.__mdPrevented = true;
    };
    // A bubble-phase document listener sees defaultPrevented after target/root handlers.
    document.addEventListener(
      "mousedown",
      (e) => {
        if (e.defaultPrevented) mark();
      },
      { once: true },
    );
  });
  await page.mouse.click(x, y);
  await sleep(120);
  const prevented = await page.evaluate(() => window.__mdPrevented);
  const sel = await selectionState(page);
  const target = info.found ? info.chain[0] : null;
  console.log(`\n[${name}] click @(${Math.round(x)},${Math.round(y)})`);
  if (target) {
    console.log(
      `  hit: <${target.tag}> us=${target.userSelect} pe=${target.pointerEvents} cls="${target.cls}"`,
    );
    for (const a of info.chain.slice(1)) {
      if (a.userSelect === "none") console.log(`  ancestor user-select:none -> <${a.tag}> cls="${a.cls}"`);
    }
  } else console.log("  hit: <nothing>");
  console.log(`  mousedownPrevented=${prevented}  selection: len=${sel.len} collapsed=${sel.collapsed} "${sel.text}"`);
  return sel.len;
}

let browser;
try {
  await waitForServer();
  // PLAYWRIGHT_CHROMIUM_PATH mirrors launch.mjs: use a locally-cached browser
  // when the bundled revision isn't downloaded.
  browser = await engine.launch({
    headless: true,
    executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH || undefined,
  });
  const context = await browser.newContext({ viewport: { width: 1360, height: 880 } });
  await context.addInitScript(() => {
    localStorage.setItem(
      "agent-desktop.dev-account",
      JSON.stringify({
        user: { id: "selection-qa", name: "Selection QA", method: "local" },
      }),
    );
    localStorage.setItem(
      "agent-desktop:local-agent",
      JSON.stringify({ cwd: "/tmp", model: "local-model", reasoningEffort: "", apiKey: "" }),
    );
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => console.log("PAGEERROR:", e.message));
  await page.goto(url, { waitUntil: "domcontentloaded" });

  await page.getByLabel("New session").first().click();
  await sleep(600);
  const input = page.locator("textarea.composer-input");
  await input.click();
  await input.fill("hello selection repro");
  await input.press("Enter");

  // Phase A: select MID-STREAM (DOM churning every token) and click a margin
  // while tokens are still arriving.
  await page.waitForSelector("text=I read", { timeout: 15000 });
  {
    const live = await page.locator("text=I read").last().boundingBox();
    if (live) {
      await page.mouse.move(live.x + 4, live.y + live.height / 2);
      await page.mouse.down();
      await page.mouse.move(live.x + Math.min(live.width - 8, 220), live.y + live.height / 2, { steps: 6 });
      await page.mouse.up();
      const midSel = await selectionState(page);
      console.log(`mid-stream selection: len=${midSel.len} "${midSel.text}"`);
      // Click the far-left margin of the conversation area while streaming.
      const r = await page.evaluate(() => {
        const els = [...document.querySelectorAll("div")].filter((e) =>
          e.textContent?.includes("I read") && getComputedStyle(e).overflowY === "auto",
        );
        const el = els[els.length - 1];
        const b = el?.getBoundingClientRect();
        return b ? { left: b.left, top: b.top, height: b.height } : null;
      });
      if (r) await page.mouse.click(r.left + 30, r.top + r.height / 2);
      await sleep(120);
      const after = await selectionState(page);
      console.log(`mid-stream margin click => selection len=${after.len} ${after.len === 0 ? "CLEARED ✓" : "SURVIVED ✗"}`);
    }
  }

  // Wait for the streamed final answer to complete.
  await page.waitForSelector("text=want me to proceed?", { timeout: 15000 });
  await sleep(500);

  // Drag-select across part of the agent's final answer (a real mouse drag).
  const answer = page.locator("text=I read").last();
  const box = await answer.boundingBox();
  if (!box) throw new Error("answer element not found");
  await page.mouse.move(box.x + 4, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + Math.min(box.width - 8, 240), box.y + box.height / 2, { steps: 12 });
  await page.mouse.up();
  await sleep(120);
  const sel0 = await selectionState(page);
  console.log(`[engine=${engineName}] after drag-select: len=${sel0.len} "${sel0.text}"`);
  if (sel0.len === 0) throw new Error("drag-select produced no selection — repro blocked");

  // Where is the conversation scroll container? The sidebar has a similar
  // scroller — pick the one that CONTAINS the answer text.
  const conv = await page.evaluate(() => {
    const answer = [...document.querySelectorAll("*")].find(
      (el) => el.childNodes.length && el.textContent?.includes("want me to proceed?"),
    );
    const els = [...document.querySelectorAll(".overflow-y-auto")];
    const el = els.find((e) => answer && e.contains(answer)) ?? els[els.length - 1];
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { left: r.left, top: r.top, width: r.width, height: r.height };
  });
  console.log("conversation scroll rect:", JSON.stringify(conv));

  // Page-wide scan: which SIZABLE elements have computed user-select:none?
  const noneEls = await page.evaluate(() => {
    const out = [];
    for (const el of document.querySelectorAll("body *")) {
      const cs = getComputedStyle(el);
      if (cs.userSelect !== "none") continue;
      const r = el.getBoundingClientRect();
      if (r.width * r.height < 4000) continue; // ignore tiny chips/icons
      out.push({
        tag: el.tagName.toLowerCase(),
        cls: (el.className?.baseVal ?? el.className ?? "").toString().slice(0, 90),
        rect: { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) },
      });
    }
    return out.slice(0, 20);
  });
  console.log("sizable user-select:none elements:", JSON.stringify(noneEls, null, 1));

  const clicks = [];
  if (conv) {
    clicks.push(["conversation left margin", conv.left + 36, conv.top + conv.height * 0.5]);
    clicks.push(["conversation right margin", conv.left + conv.width - 36, conv.top + conv.height * 0.5]);
    clicks.push(["conversation center (below content)", conv.left + conv.width / 2, conv.top + conv.height - 40]);
  }
  clicks.push(["sidebar empty (bottom)", 120, 800]);
  clicks.push(["topbar empty", 700, 24]);

  for (const [name, x, y] of clicks) {
    // Re-select before each click if the previous click cleared the selection,
    // so every click is tested against a live selection.
    let cur = await selectionState(page);
    if (cur.len === 0) {
      await page.mouse.move(box.x + 4, box.y + box.height / 2);
      await page.mouse.down();
      await page.mouse.move(box.x + Math.min(box.width - 8, 240), box.y + box.height / 2, { steps: 8 });
      await page.mouse.up();
      await sleep(100);
      cur = await selectionState(page);
      console.log(`(re-selected: len=${cur.len})`);
    }
    const remaining = await clickProbe(page, name, x, y);
    console.log(`  => ${remaining === 0 ? "CLEARED ✓" : "SELECTION SURVIVED ✗"}`);
  }
} finally {
  await browser?.close();
  dev.kill("SIGTERM");
}
