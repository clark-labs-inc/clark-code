import { chromium } from "playwright";

const MD =
  "text-ink [&_p]:my-3 [&_table]:my-2 [&_table]:w-full [&_table]:border-collapse [&_table]:table-fixed [&_table]:text-xs " +
  "[&_th]:border [&_th]:border-border-subtle [&_th]:px-2 [&_th]:py-1.5 [&_th]:text-left [&_th]:align-top [&_th]:font-medium [&_th]:text-ink-secondary [&_th]:break-words " +
  "[&_td]:border [&_td]:border-border-subtle [&_td]:px-2 [&_td]:py-1.5 [&_td]:align-top [&_td]:break-words [&_td]:overflow-wrap-anywhere";

// Two tables: a normal 2-col (the common case) and a wide 4-col that exceeds width.
const table = (cols: string[], rows: string[][]) => {
  const head = `<thead><tr>${cols.map((c) => `<th>${c}</th>`).join("")}</tr></thead>`;
  const body = `<tbody>${rows
    .map((r) => `<tr>${r.map((c) => `<td>${c}</td>`).join("")}</tr>`)
    .join("")}</tbody>`;
  return `<div class="overflow-x-auto"><table><${head}${body}</table></div>`;
};

const html =
  `<div class="${MD}" style="padding:28px;max-width:520px;margin:40px auto;background:var(--color-bg-primary);min-height:100vh;font-family:var(--font-sans)">` +
  `<h3 style="font-size:14px;margin:0 0 8px">Current table rendering (table-fixed + w-full)</h3>` +
  table(
    ["Command", "Description"],
    [
      ["<code>cargo build</code>", "Compile the workspace and all its crates"],
      ["<code>cargo test -p agent-core</code>", "Run the agent-core unit tests"],
    ],
  ) +
  `<h3 style="font-size:14px;margin:18px 0 8px">Wide table (4 cols in a 520px column)</h3>` +
  table(
    ["Layer", "File", "Lines", "Role"],
    [
      ["Domain", "agent-core/src/domain.rs", "412", "ToolCallPatch, AgentEvent, snapshot types"],
      ["Projection", "agent-core/src/projection.rs", "386", "Pure reducer over AgentEvent"],
      ["ACP adapter", "provider-acp/src/translate.rs", "189", "JSON-RPC → domain events"],
    ],
  ) +
  `</div>`;

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 760, height: 900 }, deviceScaleFactor: 2 });
await page.goto("http://localhost:1420/", { waitUntil: "domcontentloaded" });
await page.waitForTimeout(2500);
await page.evaluate((h) => {
  document.documentElement.classList.remove("dark");
  document.body.innerHTML = h;
}, html);
await page.waitForTimeout(700);
await page.screenshot({ path: "/tmp/table-current.png" });
await browser.close();
console.log("saved /tmp/table-current.png");
