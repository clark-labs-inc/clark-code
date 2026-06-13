import { launch, VIEWPORT } from "./launch.mjs";
const browser = await launch();
const page = await browser.newPage({ viewport: VIEWPORT });
await page.goto("http://localhost:1420/?demo=1", { waitUntil: "domcontentloaded" });
await page.waitForTimeout(2600);
// expand the first expandable work line to verify capped detail
const exp = page.locator('button[aria-expanded="false"]:not([disabled])').first();
if (await exp.count()) { await exp.click().catch(()=>{}); await page.waitForTimeout(500); }
await page.screenshot({ path: "/tmp/clark-compact.png" });
await browser.close();
console.log("ok");
