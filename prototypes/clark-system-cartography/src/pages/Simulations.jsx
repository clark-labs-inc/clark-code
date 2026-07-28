import { useState } from "react";
import {
  IconAlertCircle,
  IconArrowRight,
  IconBolt,
  IconCheck,
  IconExternalLink,
  IconFlask,
  IconLock,
  IconPlayerPlay,
  IconShieldCheck,
  IconSparkles,
} from "@tabler/icons-react";
import { journeyStages, scenarios } from "../data";
import { AppHeader } from "../components/Layout";
import { Progress, SourceMark, Status } from "../components/Ui";

const guarantees = [
  ["Synthetic fixtures", "Customers, orders, inventory, and payments generated from contracts."],
  ["Mock boundaries", "Stripe, SendGrid, carriers, and other external systems never receive traffic."],
  ["Deterministic events", "Every event is recorded and byte-identical on replay."],
  ["Failure injection", "Controlled failure modes are applied per scenario."],
  ["Observability assertions", "Logs, metrics, traces, and events are checked automatically."],
  ["Disposable sandboxes", "Simulations run in isolated processes and WASM runtimes where appropriate."],
];

export function Simulations({ onNotice }) {
  const [stageId, setStageId] = useState("fulfillment");
  const [selectedScenarios, setSelectedScenarios] = useState(scenarios.map((scenario) => scenario.id));
  const [generating, setGenerating] = useState(false);
  const stage = journeyStages.find((item) => item.id === stageId) || journeyStages[0];

  const generate = () => {
    setGenerating(true);
    window.setTimeout(() => {
      setGenerating(false);
      onNotice("Simulation package generated with 6 scenarios and production writes disabled.");
    }, 1400);
  };

  return (
    <div className="page simulation-page">
      <AppHeader title="Order to Cash — Global checkout" subtitle="Enterprise journey · Acme Corp · Last observed Jul 26, 2026 10:42 AM PDT">
        <div className="readiness-compact"><span>Simulation readiness</span><strong>12 of 14 contracts verified</strong><Progress value={86} /></div>
        <button className="primary" type="button" onClick={generate} disabled={generating}>
          {generating ? <IconSparkles className="spin" size={18} /> : <IconBolt size={18} />}
          {generating ? "Generating…" : "Generate simulation"}
        </button>
      </AppHeader>

      <div className="journey-stage-flow">
        {journeyStages.map((item, index) => (
          <div className="stage-flow-item" key={item.id}>
            <button className={`journey-stage ${stageId === item.id ? "selected" : ""}`} type="button" onClick={() => setStageId(item.id)}>
              <div className="stage-number">{item.number}</div>
              <div className="stage-title"><strong>{item.title}</strong><span>{item.subtitle}</span></div>
              <Status value={item.state} label="" />
              <div className="stage-system"><SourceMark name={item.system} size="sm" /><span><strong>{item.system}</strong><small>{item.cloud}</small></span></div>
            </button>
            {index < journeyStages.length - 1 && <IconArrowRight size={18} className="stage-arrow" />}
          </div>
        ))}
      </div>

      <div className="simulation-workspace">
        <div className="simulation-main">
          <section className="journey-overview-strip">
            <div><span>End-to-end events</span><strong>18</strong></div>
            <div><span>Real systems</span><strong>9</strong></div>
            <div><span>Data contracts</span><strong>14</strong></div>
            <div><span>Observed regions</span><strong>4</strong></div>
            <button className="secondary-button" type="button">View system graph <IconExternalLink size={15} /></button>
          </section>

          <section className="panel scenario-panel">
            <header className="panel-header">
              <div><div className="eyebrow">Scenario matrix</div><h2>{selectedScenarios.length} selected</h2></div>
              <button className="text-button" type="button" onClick={() => setSelectedScenarios(selectedScenarios.length === scenarios.length ? [] : scenarios.map((item) => item.id))}>
                {selectedScenarios.length === scenarios.length ? "Clear all" : "Select all"}
              </button>
            </header>
            <div className="data-table scenario-table">
              <div className="table-head"><span>Run</span><span>Scenario</span><span>Purpose</span><span>Events</span><span>Failure injection</span><span>Assertions</span><span>Status</span></div>
              {scenarios.map((scenario) => (
                <label className="table-row" key={scenario.id}>
                  <span><input type="checkbox" checked={selectedScenarios.includes(scenario.id)} onChange={() => setSelectedScenarios((current) => current.includes(scenario.id) ? current.filter((id) => id !== scenario.id) : [...current, scenario.id])} /></span>
                  <span className="strong-cell">{scenario.id === "happy" ? <IconCheck size={16} /> : <IconFlask size={16} />}{scenario.name}</span>
                  <span>{scenario.purpose}</span>
                  <span>{scenario.events}</span>
                  <span className="mono">{scenario.injection}</span>
                  <span>{scenario.assertions}</span>
                  <span><Status value="ready" /></span>
                </label>
              ))}
            </div>
          </section>

          <section className="safety-guarantees">
            <div className="guarantee-grid">
              {guarantees.map(([title, body], index) => (
                <div key={title}>
                  {index === 5 ? <IconLock size={18} /> : index < 2 ? <IconShieldCheck size={18} /> : <IconFlask size={18} />}
                  <span><strong>{title}</strong><small>{body}</small></span>
                </div>
              ))}
            </div>
            <div className="production-lock"><IconLock size={19} /><strong>Production writes: disabled</strong><span>All writes are blocked. No data leaves the sandbox.</span><IconShieldCheck size={18} /></div>
          </section>
        </div>

        <aside className="simulation-side">
          <section className="panel stage-inspector">
            <header className="panel-header"><div><span className="stage-number">{stage.number}</span><h2>{stage.subtitle}</h2></div></header>
            <dl className="stage-details">
              <div><dt>Owner</dt><dd>Fulfillment Platform Team</dd></div>
              <div><dt>Inputs</dt><dd>OrderCreated, PaymentCaptured, InventoryReserved</dd></div>
              <div><dt>Outputs</dt><dd>OrderFulfilled, InventoryDecremented, ShipmentCreated</dd></div>
              <div><dt>Invariants</dt><dd>Exactly-once fulfillment; inventory not negative; shipment created within 4h</dd></div>
              <div><dt>Failure modes</dt><dd>Service unavailable, stale inventory, downstream timeout</dd></div>
              <div><dt>Evidence</dt><dd>GitHub Enterprise · Datadog · PagerDuty</dd></div>
            </dl>
            <div className="dependency-list">
              <strong>Real dependencies</strong>
              {["Orders service · GCP us-central1", "EventBridge · AWS us-east-1", "Warehouse service · GCP us-central1", "Aurora inventory · AWS us-east-1", "SendGrid · Global"].map((dependency) => <span key={dependency}>{dependency}</span>)}
            </div>
          </section>

          <section className="panel blockers">
            <header className="panel-header"><div><div className="eyebrow">Readiness blockers</div><h2>2 gaps</h2></div><button className="text-button" type="button">Resolve all</button></header>
            <button type="button"><IconAlertCircle size={18} /><span><strong>EU production account not observed</strong><small>No live observations from EU region in production.</small><em>Add EU observation</em></span></button>
            <button type="button"><IconAlertCircle size={18} /><span><strong>Carrier webhook ownership unverified</strong><small>Endpoint ownership and behavior remain unverified.</small><em>Verify ownership</em></span></button>
          </section>

          <section className="panel change-impact">
            <header className="panel-header"><div><div className="eyebrow">Change impact</div><h2>Since charter v11</h2></div></header>
            <div><IconCheck size={15} /><span>Added EventBridge between Checkout and Fulfillment</span></div>
            <div><IconCheck size={15} /><span>Added inventory decrement event</span></div>
            <div><IconAlertCircle size={15} /><span>Shipment timeout changed from 2h to 4h</span></div>
          </section>
        </aside>
      </div>
    </div>
  );
}
