# Desktop UI automation contract

Clark Desktop exposes two deterministic GUI boundaries:

```bash
pnpm --dir harness test:specialists-ui
```

For the broader conversation and composer journey, run:

```bash
pnpm --dir harness test:full-gui
```

This mock-provider journey covers two-turn continuation, permission handling,
goal create/steer/complete/clear, all 18 built-in slash commands, terminal,
MCP, memory, compact, `/btw`, `/sentry` expansion, artifacts, and a 375px
responsive overflow check. It makes no paid provider calls and writes a
receipt plus screenshot under `target/full-gui-smoke/`.

This starts a local Vite development surface, seeds a disposable local QA
account, and tests the rendered UI without sending a prompt or calling a
provider. It covers the paid preview and free coverage gate for Scout,
Security, Scientist, and RSI, each specialist's starter prefill and canvas,
plus a 375px no-horizontal-overflow check. The command writes a new receipt
and screenshots under `target/specialist-ui-smoke/`.

The rendered app also exposes stable `data-qa` hooks and matching accessible
names for external drivers. The important selectors are:

| Surface | Selector |
| --- | --- |
| Specialist navigation | `[data-qa="specialist-navigation"]` |
| Specialist route | `[data-qa="specialist-nav-<kind>"]` |
| Welcome | `[data-qa="specialist-welcome-<kind>"]` |
| Starter | `[data-qa="specialist-starter-<kind>-<index>"]` |
| Coverage gate | `[data-qa="specialist-gate-<kind>"]` |
| Insights toggle | `[data-qa="specialist-show-insights-<kind>"]` |
| Canvas | `[data-qa="specialist-canvas-<kind>"]` |
| Composer | accessible label `Message Clark` |

`<kind>` is one of `scout`, `security`, `scientist`, or `rsi`. Clark Tester
can use the same visible labels through its browser CDP lane, or the
accessibility/Appium macOS lane when a dedicated macOS target is configured.
The packaged native app remains a separate proof boundary: this browser
contract does not claim native authentication, live provider execution, or
paid model quality.
