import { launch, VIEWPORT } from "./launch.mjs";
const browser = await launch();
const page = await browser.newPage({ viewport: VIEWPORT });
await page.goto("http://localhost:1420/?demo=1", { waitUntil: "domcontentloaded" });
await page.waitForTimeout(2600);
// click the work line that mentions Edit (the diff one)
const edit = page.locator('button:has-text("Edit")').first();
if (await edit.count()) { await edit.click().catch(()=>{}); await page.waitForTimeout(500); }
await page.screenshot({ path: "/tmp/clark-workline-expand.png" });
await browser.close();
console.log("ok");
