// A real transcript alternates narration and tool receipts before answering.
// Status-only transitions must not add/remove predicted answer geometry.
export async function conversationProgressGeometry(page) {
  await page.evaluate(() => {
    const store = window.__agentDesktopStore;
    const history = Array.from({ length: 24 }, (_, index) => ({
      item: "message", role: "agent", blocks: [{ type: "text", text: `Saved paragraph ${index}. A stable line of earlier conversation.` }],
    }));
    store.setState({ session: { id: "progress-geometry", provider: "local", capabilities: {} }, opening: null,
      snapshot: { ...store.getState().snapshot, session: "progress-geometry", tool_calls: {},
        runs: { progress: { id: "progress", status: "running" } },
        timeline: [...history,
          { item: "message", role: "user", run: "progress", blocks: [{ type: "text", text: "Check the background task." }] },
          { item: "message", role: "agent", phase: "commentary", run: "progress", blocks: [{ type: "text", text: "I will check the background task." }] },
        ],
      },
    });
  });
  await page.getByText("I will check the background task.", { exact: true }).waitFor();
  const settle = () => page.evaluate(async () => {
    const end = performance.now() + 350;
    while (performance.now() < end) await new Promise(requestAnimationFrame);
    const log = document.querySelector('[role="log"]');
    const anchor = [...log.querySelectorAll("p")].find(p => p.textContent === "I will check the background task.");
    return { height: log.firstElementChild.getBoundingClientRect().height, anchorTop: anchor.getBoundingClientRect().top };
  });
  const commentary = await settle();
  await page.evaluate(() => {
    const store = window.__agentDesktopStore;
    const snapshot = store.getState().snapshot;
    store.setState({ snapshot: { ...snapshot,
      tool_calls: { check: { id: "check", title: "Check task status", kind: "execute", status: "in_progress", content: [], locations: [] } },
      timeline: [...snapshot.timeline, { item: "tool_call", id: "check", run: "progress" }],
    } });
  });
  const runningTool = await settle();
  await page.evaluate(() => {
    const store = window.__agentDesktopStore;
    const snapshot = store.getState().snapshot;
    store.setState({ snapshot: { ...snapshot, tool_calls: { check: { ...snapshot.tool_calls.check, status: "completed" } } } });
  });
  const completedTool = await settle();
  await page.evaluate(() => {
    const store = window.__agentDesktopStore;
    const snapshot = store.getState().snapshot;
    store.setState({ snapshot: { ...snapshot, runs: { progress: { id: "progress", status: "done" } } } });
  });
  const done = await settle();
  return { commentary, runningTool, completedTool, done,
    appendHeight: runningTool.height - commentary.height,
    statusHeightShift: completedTool.height - runningTool.height,
    statusAnchorShift: completedTool.anchorTop - runningTool.anchorTop,
    finishHeightShift: done.height - completedTool.height,
  };
}

// Measure both the DOM identity and painted boxes while disclosures animate
// and distinct commands are appended rapidly. No provider is invoked.
export async function commandRowGeometry(page) {
  await page.evaluate(() => {
    const store = window.__agentDesktopStore;
    const snapshot = store.getState().snapshot;
    store.setState({ snapshot: { ...snapshot,
      runs: { progress: { id: "progress", status: "running" } },
      timeline: snapshot.timeline.slice(-3),
      tool_calls: { ...snapshot.tool_calls, check: { ...snapshot.tool_calls.check, content: [{ type: "text", text: "Background task output\n".repeat(10) }] } },
    } });
    const seen = new Map();
    const probe = { frames: 0, overlap: 0, replaced: 0, duplicates: 0, active: true };
    window.__commandGeometry = probe;
    const sample = () => {
      const rows = [...document.querySelectorAll("[data-tool-call-id]")];
      const ids = new Set();
      let previous;
      for (const row of rows) {
        const id = row.dataset.toolCallId;
        if (ids.has(id)) probe.duplicates++;
        ids.add(id);
        if (seen.has(id) && seen.get(id) !== row) probe.replaced++;
        seen.set(id, row);
        const rect = row.getBoundingClientRect();
        if (previous) probe.overlap = Math.max(probe.overlap, previous.bottom - rect.top);
        previous = rect;
      }
      probe.frames++;
      if (probe.active) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
  const disclosure = page.locator('[data-tool-call-id="check"] > button');
  await disclosure.click();
  await page.evaluate(async () => {
    const store = window.__agentDesktopStore;
    for (let index = 0; index < 12; index++) {
      const snapshot = store.getState().snapshot;
      const tool_calls = Object.fromEntries(Object.entries(snapshot.tool_calls).map(([id, call]) => [id, { ...call, status: "completed" }]));
      const id = `command-${index}`;
      tool_calls[id] = { id, title: `Inspect background task ${index}`, kind: "execute", status: "in_progress", content: [], locations: [] };
      store.setState({ snapshot: { ...snapshot, tool_calls, timeline: [...snapshot.timeline, { item: "tool_call", id, run: "progress" }] } });
      for (let frame = 0; frame < 3; frame++) await new Promise(requestAnimationFrame);
    }
  });
  await disclosure.click();
  return page.evaluate(async () => {
    const end = performance.now() + 250;
    while (performance.now() < end) await new Promise(requestAnimationFrame);
    const probe = window.__commandGeometry;
    probe.active = false;
    const commands = [...document.querySelectorAll('[data-tool-call-id^="command-"]')];
    const viewport = document.querySelector('[role="log"]').getBoundingClientRect();
    return { frames: probe.frames, overlap: probe.overlap, replaced: probe.replaced, duplicates: probe.duplicates,
      retainedTitles: commands.every((row, index) => row.textContent.includes(`Inspect background task ${index}`)), count: commands.length,
      visible: commands.filter(row => { const rect = row.getBoundingClientRect(); return rect.top >= viewport.top && rect.bottom <= viewport.bottom; }).length,
    };
  });
}
