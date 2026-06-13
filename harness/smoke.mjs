import { launch, VIEWPORT } from "./launch.mjs";
const url = "http://localhost:1420/?dev&provider=clark&q=" +
  encodeURIComponent("In one short sentence, what is the Rust programming language?");
const browser = await launch();
const page = await browser.newPage({ viewport: VIEWPORT });
const errors = [];
page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
// Clark gateway creds come from the environment — CLARK_WS / CLARK_TOKEN.
if (process.env.CLARK_WS || process.env.CLARK_TOKEN) {
  await page.addInitScript(
    ([ep, tok]) => {
      if (ep) localStorage.setItem("clark.endpoint", ep);
      if (tok) localStorage.setItem("clark.token", tok);
    },
    [process.env.CLARK_WS, process.env.CLARK_TOKEN],
  );
}
await page.goto(url, { waitUntil: "domcontentloaded" });
let ok = false;
for (let i = 0; i < 75; i++) {
  const txt = await page.evaluate(() => document.body.innerText);
  if (txt.toLowerCase().split("rust").length > 2) { ok = true; break; }
  await page.waitForTimeout(1000);
}
await page.screenshot({ path: "/tmp/clark-real-clark.png" });
const bodyText = await page.evaluate(() => document.body.innerText);
console.log("REPLY_FOUND:", ok);
console.log("BODY:", bodyText.replace(/\s+/g, " ").slice(0, 500));
if (errors.length) console.log("CONSOLE_ERRORS:", errors.slice(0, 5));
await browser.close();
