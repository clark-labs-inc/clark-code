import { useCallback, useMemo, useState } from "react";
import {
  Background,
  Controls,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  useEdgesState,
  useNodesState,
} from "@xyflow/react";
import {
  IconAdjustments,
  IconClock,
  IconFilter,
  IconFocusCentered,
  IconSearch,
  IconShieldCheck,
} from "@tabler/icons-react";
import { graphEdges, graphNodes } from "../data";
import { AppHeader } from "../components/Layout";
import { KeyValue, SourceMark, Status } from "../components/Ui";

function SystemNode({ data, selected }) {
  return (
    <div className={`flow-node flow-node-${data.state} ${selected ? "selected" : ""}`}>
      <Handle type="target" position={Position.Left} />
      <SourceMark name={data.label} size="sm" />
      <div><strong>{data.label}</strong><span>{data.type}</span></div>
      <Status value={data.state} label="" />
      <Handle type="source" position={Position.Right} />
    </div>
  );
}

const nodeTypes = { system: SystemNode };

export function SystemGraph() {
  const initialNodes = useMemo(() => graphNodes.map((node) => ({
    id: node.id,
    type: "system",
    position: { x: node.x, y: node.y },
    data: { label: node.label, type: node.type, state: node.state },
  })), []);
  const initialEdges = useMemo(() => graphEdges.map(([source, target, label], index) => ({
    id: `e-${index}`,
    source,
    target,
    label,
    animated: label === "calls" || label === "publishes",
    className: `flow-edge flow-edge-${label}`,
  })), []);
  const [nodes, setNodes, onNodesChange] = useNodesState(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);
  const [selectedId, setSelectedId] = useState("checkout");
  const [timeMode, setTimeMode] = useState("current");
  const [filterOpen, setFilterOpen] = useState(true);
  const selected = nodes.find((node) => node.id === selectedId) || nodes[0];

  const onNodeClick = useCallback((_, node) => setSelectedId(node.id), []);

  return (
    <div className="page graph-page">
      <AppHeader title="System graph" subtitle="Explore qualified entities and relationships across Acme Corp.">
        <button className="secondary-button" type="button" onClick={() => setFilterOpen(!filterOpen)}><IconFilter size={17} /> Filters</button>
        <button className="primary" type="button"><IconFocusCentered size={18} /> Focus journey</button>
      </AppHeader>

      <div className="graph-toolbar">
        <label className="search-control"><IconSearch size={17} /><input placeholder="Find a service, team, account, or journey" /></label>
        <div className="time-toggle">
          <button type="button" className={timeMode === "current" ? "active" : ""} onClick={() => setTimeMode("current")}>Current qualified</button>
          <button type="button" className={timeMode === "history" ? "active" : ""} onClick={() => setTimeMode("history")}><IconClock size={15} /> Time travel</button>
        </div>
        <span className="graph-freshness"><IconShieldCheck size={16} /> Fixed point verified · 18 min ago</span>
      </div>

      <div className={`graph-workspace ${filterOpen ? "with-filter" : ""}`}>
        {filterOpen && (
          <aside className="graph-filter-panel">
            <div className="panel-title"><IconAdjustments size={18} /><strong>Graph scope</strong></div>
            <label>Business journey<select defaultValue="order"><option value="order">Order to Cash</option><option>All journeys</option></select></label>
            <label>Environment<select defaultValue="global"><option value="global">Global production</option><option>EU production</option><option>Staging</option></select></label>
            <div className="filter-group">
              <strong>Entity kinds</strong>
              {["Services", "Applications", "Data stores", "Teams & owners", "Cloud resources", "External vendors"].map((item) => <label className="check-row compact" key={item}><input type="checkbox" defaultChecked /><span>{item}</span></label>)}
            </div>
            <div className="filter-group">
              <strong>Trust status</strong>
              {["Verified", "Partial", "Gap"].map((item) => <label className="check-row compact" key={item}><input type="checkbox" defaultChecked /><span>{item}</span></label>)}
            </div>
            <button className="secondary-button full" type="button">Reset filters</button>
          </aside>
        )}

        <main className="flow-canvas">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onNodeClick={onNodeClick}
            nodeTypes={nodeTypes}
            fitView
            fitViewOptions={{ padding: 0.16 }}
            minZoom={0.45}
            maxZoom={1.7}
          >
            <Background color="#dedbd4" gap={24} size={1} />
            <Controls showInteractive={false} />
            <MiniMap pannable zoomable nodeColor={(node) => node.data.state === "gap" ? "#e8a626" : node.data.state === "partial" ? "#6b5be6" : "#5d9163"} />
          </ReactFlow>
          <div className="graph-legend">
            <span><i className="legend-verified" /> Verified</span>
            <span><i className="legend-partial" /> Partial</span>
            <span><i className="legend-gap" /> Gap</span>
            <span>10 entities · 9 relationships · 7 evidence sources</span>
          </div>
        </main>

        <aside className="inspector graph-inspector">
          <div className="inspector-title">
            <SourceMark name={selected.data.label} />
            <div><strong>{selected.data.label}</strong><span>{selected.data.type}</span></div>
            <Status value={selected.data.state} />
          </div>
          <div className="inspector-scroll">
            <section className="inspector-section">
              <h3>Identity</h3>
              <KeyValue label="Canonical ID">entity:7f32…91ac</KeyValue>
              <KeyValue label="Provider ID">service/acme/{selected.id}</KeyValue>
              <KeyValue label="Environment">Global production</KeyValue>
              <KeyValue label="Classification">Internal</KeyValue>
            </section>
            <section className="inspector-section">
              <h3>Trust</h3>
              <KeyValue label="Status"><Status value={selected.data.state} /></KeyValue>
              <KeyValue label="Last observed">18 min ago</KeyValue>
              <KeyValue label="Independent sources">4</KeyValue>
              <KeyValue label="Qualified passes">2 identical</KeyValue>
            </section>
            <section className="inspector-section">
              <h3>Key relationships</h3>
              <div className="relationship-list">
                <span>Web app <small>upstream</small></span>
                <span>Orders service <small>downstream</small></span>
                <span>Aurora orders <small>writes</small></span>
                <span>Datadog APM <small>observed by</small></span>
                <span>Payments team <small>owned by</small></span>
              </div>
            </section>
            <section className="inspector-section">
              <h3>Temporal history</h3>
              <div className="compact-timeline">
                <span><i />Current version opened Jul 18</span>
                <span><i />Ownership corrected Jul 21</span>
                <span><i />AWS task revision 38 observed Jul 26</span>
              </div>
            </section>
          </div>
        </aside>
      </div>
    </div>
  );
}
