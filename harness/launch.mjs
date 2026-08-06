// Shared Playwright launch. Set PLAYWRIGHT_CHROMIUM_PATH to use a locally-cached
// Chromium (avoids a download when the bundled revision differs); otherwise
// Playwright's bundled browser is used.
import { chromium } from "playwright";
import { existsSync } from "node:fs";

const systemChromium = [
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Chromium.app/Contents/MacOS/Chromium",
].find((candidate) => existsSync(candidate));

export const EXECUTABLE = process.env.PLAYWRIGHT_CHROMIUM_PATH || systemChromium || undefined;

export const VIEWPORT = { width: 1360, height: 880 };

export async function launch() {
  return chromium.launch({ headless: true, executablePath: EXECUTABLE });
}
