import { launch, VIEWPORT } from "./launch.mjs";
const browser = await launch();
const page = await browser.newPage({ viewport: VIEWPORT });
await page.goto("http://localhost:1420/?demo=1", { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1200); // mid-run: plan + work + working indicator, before gate
await page.screenshot({ path: "/tmp/clark-busy.png" });
await browser.close(); console.log("ok");
