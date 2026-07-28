import { useEffect, useMemo, useState } from "react";
import {
  Handle,
  MarkerType,
  Position,
  ReactFlow,
} from "@xyflow/react";
import {
  IconActivity,
  IconArrowRight,
  IconBolt,
  IconBrandNetflix,
  IconBuildingBroadcastTower,
  IconCloud,
  IconCreditCard,
  IconDatabase,
  IconDeviceDesktop,
  IconFolder,
  IconGauge,
  IconLock,
  IconPlayerPauseFilled,
  IconPlayerPlayFilled,
  IconRefresh,
  IconRoute,
  IconShield,
  IconSparkles,
  IconUser,
  IconUsers,
  IconVideo,
} from "@tabler/icons-react";

const timeSteps = [
  { label: "Day 0", detail: "Baseline", clock: "00:00", coverage: 0 },
  { label: "Day 1", detail: "Identity mapped", clock: "11:20", coverage: 24 },
  { label: "Day 2", detail: "Content delivery expanded", clock: "17:05", coverage: 44 },
  { label: "Day 3", detail: "Playback simulation started", clock: "14:32", coverage: 63 },
  { label: "Week 1", detail: "Operations mapping", clock: "Friday", coverage: 72 },
  { label: "Week 2", detail: "Billing systems discovered", clock: "Wednesday", coverage: 81 },
  { label: "Week 3", detail: "Deeper inference added", clock: "Monday", coverage: 89 },
  { label: "Week 4", detail: "Full-system understanding", clock: "Friday", coverage: 94 },
];

const nodeCatalog = [
  { id: "video", label: "Video Playback", short: "Service", icon: IconVideo, x: 65, y: 70, at: 0, region: "playback", covered: true },
  { id: "web", label: "Player Web App", short: "Client", icon: IconDeviceDesktop, x: 40, y: 245, at: 0, region: "playback", covered: true },
  { id: "session", label: "Session Manager", short: "Runtime", icon: IconLock, x: 200, y: 80, at: 1, region: "playback", covered: true },
  { id: "playback", label: "Playback API", short: "Gateway", icon: IconShield, x: 235, y: 245, at: 1, region: "playback", covered: true },
  { id: "qoe", label: "QoE Monitor", short: "Telemetry", icon: IconGauge, x: 325, y: 25, at: 2, region: "playback", covered: true, inferred: true },
  { id: "drm", label: "DRM Service", short: "Policy", icon: IconLock, x: 395, y: 155, at: 2, region: "playback", covered: true },
  { id: "auth", label: "Auth Service", short: "Identity", icon: IconUser, x: 545, y: 70, at: 1, region: "identity", covered: true },
  { id: "account", label: "Account Service", short: "Identity", icon: IconUsers, x: 615, y: 235, at: 2, region: "identity", covered: true },
  { id: "profile", label: "Profile Service", short: "Identity", icon: IconFolder, x: 745, y: 105, at: 2, region: "identity", covered: true },
  { id: "device", label: "Device Graph", short: "Inference", icon: IconSparkles, x: 760, y: 275, at: 3, region: "identity", covered: true, inferred: true },
  { id: "cdn", label: "CDN Edge", short: "Delivery", icon: IconBuildingBroadcastTower, x: 165, y: 425, at: 2, region: "delivery", covered: true },
  { id: "traffic", label: "Traffic Controller", short: "Delivery", icon: IconBolt, x: 250, y: 545, at: 3, region: "delivery", covered: true },
  { id: "edge", label: "Edge Routing", short: "Delivery", icon: IconRoute, x: 345, y: 430, at: 2, region: "delivery", covered: true },
  { id: "origin", label: "Origin Service", short: "Delivery", icon: IconDatabase, x: 500, y: 430, at: 2, region: "delivery", covered: true },
  { id: "studio", label: "Studio Portal", short: "Application", icon: IconDeviceDesktop, x: 895, y: 90, at: 3, region: "studio", covered: false },
  { id: "ingest", label: "Ingest Service", short: "Pipeline", icon: IconCloud, x: 1010, y: 135, at: 3, region: "studio", covered: false },
  { id: "asset", label: "Asset Service", short: "Storage", icon: IconFolder, x: 1095, y: 235, at: 3, region: "studio", covered: false },
  { id: "transcode", label: "Transcode Service", short: "Compute", icon: IconRefresh, x: 920, y: 300, at: 3, region: "studio", covered: false },
  { id: "scene", label: "AI Scene Detection", short: "Hypothesis", icon: IconSparkles, x: 1060, y: 335, at: 3, region: "studio", covered: false, hypothesis: true },
  { id: "subscription", label: "Subscription Service", short: "Billing", icon: IconRefresh, x: 790, y: 540, at: 3, region: "billing", covered: false },
  { id: "billing", label: "Billing Service", short: "Ledger", icon: IconFolder, x: 910, y: 445, at: 3, region: "billing", covered: false },
  { id: "payment", label: "Payment Gateway", short: "Payments", icon: IconCreditCard, x: 1030, y: 465, at: 3, region: "billing", covered: false },
  { id: "revenue", label: "Revenue Insights", short: "Hypothesis", icon: IconActivity, x: 1110, y: 560, at: 3, region: "billing", covered: false, hypothesis: true },
];

const edgeCatalog = [
  ["video", "session", 1], ["session", "playback", 1], ["web", "playback", 1],
  ["video", "playback", 1], ["qoe", "playback", 2], ["playback", "drm", 2],
  ["drm", "auth", 2], ["auth", "account", 2], ["account", "profile", 2],
  ["profile", "device", 3], ["device", "account", 3], ["playback", "account", 2],
  ["cdn", "edge", 2], ["edge", "origin", 2], ["traffic", "edge", 3],
  ["origin", "playback", 2], ["traffic", "cdn", 3], ["origin", "subscription", 3],
  ["studio", "ingest", 3], ["ingest", "asset", 3], ["ingest", "transcode", 3],
  ["transcode", "scene", 3], ["subscription", "billing", 3], ["billing", "payment", 3],
  ["payment", "revenue", 3], ["account", "subscription", 3],
];

function AtlasNode({ data }) {
  const Icon = data.icon;
  return (
    <div className={`atlas-node ${data.covered ? "covered" : "uncovered"} ${data.inferred ? "inferred" : ""} ${data.hypothesis ? "hypothesis" : ""}`}>
      <Handle type="target" position={Position.Left} />
      <span className="atlas-node-icon"><Icon size={19} stroke={1.7} /></span>
      <span className="atlas-node-copy"><strong>{data.label}</strong><small>{data.short}</small></span>
      <Handle type="source" position={Position.Right} />
    </div>
  );
}

const nodeTypes = { atlas: AtlasNode };

export function Overview({ onNotice }) {
  const [timeIndex, setTimeIndex] = useState(3);
  const [playing, setPlaying] = useState(true);
  const [mode, setMode] = useState("simulation");
  const [simulationStatus, setSimulationStatus] = useState("Ready");

  useEffect(() => {
    if (!playing) return undefined;
    const timer = window.setInterval(() => {
      setTimeIndex((current) => {
        if (current >= timeSteps.length - 1) {
          setPlaying(false);
          return current;
        }
        return current + 1;
      });
    }, 3500);
    return () => window.clearInterval(timer);
  }, [playing]);

  const visibleCatalog = useMemo(
    () => nodeCatalog.filter((node) => node.at <= timeIndex && (mode !== "observed" || (!node.inferred && !node.hypothesis))),
    [mode, timeIndex],
  );
  const visibleIds = useMemo(() => new Set(visibleCatalog.map((node) => node.id)), [visibleCatalog]);
  const nodes = useMemo(() => visibleCatalog.map((node) => ({
    id: node.id,
    type: "atlas",
    position: { x: node.x, y: node.y },
    data: {
      ...node,
      covered: mode === "simulation" ? node.covered : false,
    },
  })), [mode, visibleCatalog]);
  const edges = useMemo(() => edgeCatalog
    .filter(([source, target, at]) => at <= timeIndex && visibleIds.has(source) && visibleIds.has(target))
    .map(([source, target, at], index) => {
      const sourceNode = nodeCatalog.find((node) => node.id === source);
      const targetNode = nodeCatalog.find((node) => node.id === target);
      const covered = sourceNode.covered && targetNode.covered;
      const inferred = sourceNode.inferred || targetNode.inferred || at > 4;
      return {
        id: `atlas-edge-${index}`,
        source,
        target,
        type: "smoothstep",
        markerEnd: { type: MarkerType.ArrowClosed, width: 10, height: 10 },
        className: `atlas-edge ${covered && mode === "simulation" ? "covered" : ""} ${inferred ? "inferred" : ""}`,
      };
    }), [mode, timeIndex, visibleIds]);

  const step = timeSteps[timeIndex];
  const discoveredCount = visibleCatalog.filter((node) => !node.hypothesis).length;
  const simulationCoverage = mode === "simulation" ? step.coverage : 0;

  const runSimulation = () => {
    setSimulationStatus("Running");
    onNotice("Playback API regional failure simulation is running against the Day 3 enterprise twin.");
    window.setTimeout(() => {
      setSimulationStatus("Complete");
      onNotice("Simulation complete: playback remained available in three regions; Studio Operations and Billing were outside coverage.");
    }, 1800);
  };

  const replay = () => {
    setTimeIndex(0);
    setPlaying(true);
    setSimulationStatus("Ready");
  };

  return (
    <div className={`temporal-atlas mode-${mode}`}>
      <header className="atlas-topbar">
        <button className="atlas-brand" type="button" onClick={() => window.location.assign("#/overview")}>clark<span /></button>
        <span className="atlas-divider" />
        <strong className="atlas-product">Temporal Atlas</strong>
        <span className="atlas-context">Clark System Cartography</span>
        <div className="atlas-date">
          <small>July 26, 2026</small>
          <strong>{step.label} · {step.clock}</strong>
        </div>
        <button className="atlas-profile" type="button">
          <span>MG</span>
          <span><strong>Maya Graham</strong><small>Netflix (Simulated)</small></span>
        </button>
      </header>

      <section className="atlas-command">
        <div className="atlas-enterprise">
          <span className="atlas-enterprise-mark"><IconBrandNetflix size={28} /></span>
          <span><strong>Netflix (Simulated)</strong><small>Enterprise twin</small></span>
        </div>
        <div className="atlas-modes" aria-label="Map display mode">
          <button type="button" className={mode === "observed" ? "active" : ""} onClick={() => setMode("observed")}><span className="mode-solid" />Observed</button>
          <button type="button" className={mode === "inferred" ? "active" : ""} onClick={() => setMode("inferred")}><span className="mode-dashed" />Inferred</button>
          <button type="button" className={mode === "simulation" ? "active" : ""} onClick={() => setMode("simulation")}><IconActivity size={16} />Simulation overlay</button>
        </div>
        <div className="atlas-legend">
          <span><i className="coverage-key" />Simulation coverage ({simulationCoverage}%)</span>
          <span><i className="mapped-key" />Mapped, not covered</span>
          <span><i className="hypothesis-key" />Hypothesis</span>
        </div>
      </section>

      <section className="atlas-live-strip">
        <IconSparkles size={18} />
        <strong>Live</strong>
        <span>Maya&apos;s Scout run added 18 services and made Playback failure simulation executable.</span>
        <small>2m ago</small>
        <i />
      </section>

      <section className="atlas-workspace">
        <main className="atlas-map">
          <div className="atlas-region atlas-region-playback"><strong>Playback</strong></div>
          <div className="atlas-region atlas-region-identity"><strong>Identity</strong></div>
          <div className="atlas-region atlas-region-delivery"><strong>Content Delivery</strong></div>
          <div className="atlas-region atlas-region-studio"><strong>Studio Operations</strong></div>
          <div className="atlas-region atlas-region-billing"><strong>Billing</strong></div>
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            fitView
            fitViewOptions={{ padding: 0.08 }}
            nodesDraggable={false}
            nodesConnectable={false}
            elementsSelectable
            panOnDrag
            zoomOnScroll={false}
            zoomOnPinch
            minZoom={0.62}
            maxZoom={1.25}
            proOptions={{ hideAttribution: true }}
          />
          {timeIndex < 2 && (
            <div className="atlas-empty-guidance">
              <IconSparkles size={20} />
              <strong>The twin is beginning to form</strong>
              <span>Play forward to watch verified evidence connect systems and unlock simulations.</span>
            </div>
          )}
        </main>

        <aside className="atlas-inspector">
          <small className="atlas-label">Current simulation</small>
          <h1>Playback API<br />regional failure</h1>
          <span className={`simulation-state state-${simulationStatus.toLowerCase()}`}>{simulationStatus}</span>
          <div className="atlas-inspector-section">
            <small>System coverage</small>
            <strong className="coverage-value">{simulationCoverage}%</strong>
            <span>of discovered system</span>
            <div className="coverage-bar"><i style={{ width: `${simulationCoverage}%` }} /></div>
          </div>
          <div className="atlas-inspector-section">
            <small>Confidence</small>
            <strong className="confidence-value">{timeIndex < 2 ? "Emerging" : "High"}</strong>
            <span>Based on {Math.max(246, discoveredCount * 970).toLocaleString()} evidence objects</span>
            <div className="confidence-ticks">{Array.from({ length: 7 }).map((_, index) => <i key={index} className={index < Math.ceil(timeIndex + 2) ? "active" : ""} />)}</div>
          </div>
          <div className="atlas-inspector-section">
            <small>Uncovered boundary</small>
            <p>{timeIndex < 4 ? "Studio Operations and Billing remain outside simulation coverage." : "Billing and licensing workflows remain partially modeled."}</p>
          </div>
          <div className="atlas-next-discovery">
            <span><IconRoute size={18} /></span>
            <div><small>Highest-value next discovery</small><p>Connect CDN control-plane evidence to extend coverage.</p></div>
          </div>
        </aside>
      </section>

      <section className="atlas-timeline">
        <div className="atlas-playback-controls">
          <button className="atlas-play-button" type="button" aria-label={playing ? "Pause discovery history" : "Play discovery history"} onClick={() => setPlaying((value) => !value)}>
            {playing ? <IconPlayerPauseFilled size={20} /> : <IconPlayerPlayFilled size={20} />}
          </button>
          <button className="atlas-replay-button" type="button" onClick={replay}><IconRefresh size={17} />Replay from Day 0</button>
        </div>
        <div className="atlas-evolution">
          <div className="atlas-evolution-title"><strong>System evolution</strong><small>Graph growth over time</small></div>
          <div className="atlas-time-track">
            <div className="atlas-time-progress" style={{ width: `${(timeIndex / (timeSteps.length - 1)) * 100}%` }} />
            {timeSteps.map((item, index) => (
              <button
                key={item.label}
                className={index === timeIndex ? "active" : index < timeIndex ? "complete" : ""}
                type="button"
                onClick={() => {
                  setTimeIndex(index);
                  setPlaying(false);
                }}
              >
                {index === timeIndex && <span className="atlas-now">Now<strong>{item.label} · {item.clock}</strong></span>}
                <i />
                <strong>{item.label}</strong>
                <small>{item.detail}</small>
              </button>
            ))}
          </div>
          <div className="atlas-evolution-legend">
            <span><i className="solid-line" />Observed (verified)</span>
            <span><i className="dashed-line" />Inferred (modeled)</span>
            <span><i className="dotted-line" />Hypothesized (not yet seen)</span>
          </div>
        </div>
        <div className="atlas-run">
          <button className="primary" type="button" disabled={simulationStatus === "Running"} onClick={runSimulation}>
            <IconPlayerPlayFilled size={16} />{simulationStatus === "Running" ? "Running simulation…" : "Run simulation"}
          </button>
          <small>Updates system state<br />and projections.</small>
        </div>
      </section>
    </div>
  );
}
