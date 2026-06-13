import { chromium } from "playwright";
const EXE = process.env.PLAYWRIGHT_CHROMIUM_PATH || undefined;
const browser = await chromium.launch({ headless: true, executablePath: EXE });
const page = await browser.newPage({ viewport: { width: 1360, height: 880 } });
const logs = [];
page.on("console", (m) => logs.push(`${m.type()}: ${m.text()}`));
await page.addInitScript(([ep, tok]) => {
  localStorage.setItem("clark.endpoint", ep);
  localStorage.setItem("clark.token", tok);
}, [process.env.CLARK_WS, process.env.CLARK_TOKEN]);
await page.goto("http://localhost:1420/?dev&provider=clark&q=" + encodeURIComponent("Say pong"), { waitUntil: "domcontentloaded" });
// read back localStorage + wait for either a reply or error
const ls = await page.evaluate(() => ({ ep: localStorage.getItem("clark.endpoint"), tok: localStorage.getItem("clark.token") }));
console.log("localStorage:", JSON.stringify(ls));
let state = "?";
for (let i = 0; i < 40; i++) {
  const txt = await page.evaluate(() => document.body.innerText);
  if (/pong/i.test(txt)) { state = "REPLY"; break; }
  if (/error/i.test(txt) && /transport|refused|connect/i.test(txt)) { state = "CONN_ERROR: " + txt.match(/Error[^\n]*/)?.[0]; break; }
  await page.waitForTimeout(1000);
}
console.log("state:", state);
console.log("console:", logs.slice(0, 8).join(" | "));
await browser.close();
