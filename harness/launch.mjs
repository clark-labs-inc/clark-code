// Shared Playwright launch. Set PLAYWRIGHT_CHROMIUM_PATH to use a locally-cached
// Chromium (avoids a download when the bundled revision differs); otherwise
// Playwright's bundled browser is used.
import { chromium } from "playwright";

export const EXECUTABLE = process.env.PLAYWRIGHT_CHROMIUM_PATH || undefined;

export const VIEWPORT = { width: 1360, height: 880 };

export async function launch() {
  return chromium.launch({ headless: true, executablePath: EXECUTABLE });
}
