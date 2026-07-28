import { useMemo, useState } from "react";
import {
  IconChevronRight,
  IconClock,
  IconDots,
  IconExternalLink,
  IconPlayerPause,
  IconPlayerPlay,
  IconRefresh,
  IconSearch,
  IconShieldCheck,
} from "@tabler/icons-react";
import { journeys, summary, workstreams } from "../data";
import { AppHeader } from "../components/Layout";
import { CopyValue, KeyValue, Metric, Panel, Progress, Segmented, SourceMark, Status } from "../components/Ui";

export function Discoveries({ running, onToggleRun }) {
  const [view, setView] = useState("journeys");
  const [selectedId, setSelectedId] = useState("aws");
  const [query, setQuery] = useState("");
  const [inspectorTab, setInspectorTab] = useState("inspector");
  const selected = workstreams.find((item) => item.id === selectedId) || workstreams[0];
  const filtered = useMemo(() => workstreams.filter((item) => `${item.source} ${item.task} ${item.machine}`.toLowerCase().includes(query.toLowerCase())), [query]);

  return (
    <div className="page discovery-page">
      <AppHeader title="Live discovery" subtitle="Bounded agents are exhausting Acme Corp’s declared control planes.">
        <button className="secondary-button" type="button"><IconRefresh size={17} /> Refresh</button>
      </AppHeader>

      <section className="run-banner">
        <div className="run-identity">
          <strong>Discovery run #184</strong>
          <Status value={running ? "running" : "resumable"} label={running ? "Running" : "Paused safely"} />
        </div>
        <div className="run-meta">
          <span><small>Charter</small><strong>v12</strong></span>
          <span><small>Started</small><strong>38 min ago</strong></span>
          <span><small>Initiated by</small><strong>s.graham@acme.com</strong></span>
        </div>
        <div className="run-actions">
          <button className="primary" type="button" onClick={onToggleRun}>
            {running ? <IconPlayerPause size={18} /> : <IconPlayerPlay size={18} />}
            {running ? "Pause run" : "Resume run"}
          </button>
          <button className="secondary-button icon-only" type="button" aria-label="Run actions"><IconDots size={19} /></button>
        </div>
      </section>

      <div className="run-metrics">
        <Metric value={`${summary.mapped}%`} label="Mapped" />
        <Metric value={summary.gaps} label="Explicit gaps" tone="amber" />
        <Metric value={summary.denied} label="Denied surfaces" tone="red" />
        <Metric value={summary.unreachable} label="Unreachable" />
        <Metric value={summary.secrets} label="Secret values collected" tone="green" />
        <Metric value="100%" label="Evidence signed" tone="green" />
        <Metric value="100%" label="Terminal accounting" tone="green" />
      </div>

      <div className="discovery-workspace">
        <div className="discovery-main">
          <Panel className="journey-table-panel">
            <div className="table-toolbar">
              <Segmented
                value={view}
                onChange={setView}
                options={[{ value: "journeys", label: "Business journeys" }, { value: "control", label: "Control planes" }]}
              />
              <label className="search-control compact"><IconSearch size={16} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search workstreams" /></label>
            </div>
            {view === "journeys" ? (
              <div className="data-table journey-table">
                <div className="table-head">
                  <span>Business journey</span><span>Progress</span><span>Mapped</span><span>Gaps</span><span>Denied</span><span>Unreachable</span><span>Last activity</span>
                </div>
                {journeys.map((journey) => (
                  <button className="table-row" type="button" key={journey.id}>
                    <span className="strong-cell"><IconChevronRight size={15} />{journey.name}</span>
                    <span><Progress value={journey.progress} /></span>
                    <span>{journey.mapped}</span>
                    <span className="amber-text">{journey.gaps}</span>
                    <span className="red-text">{journey.denied}</span>
                    <span>{journey.unreachable}</span>
                    <span>{journey.updated}</span>
                  </button>
                ))}
              </div>
            ) : (
              <div className="control-plane-grid">
                {workstreams.slice(0, 8).map((item) => (
                  <button key={item.id} type="button" onClick={() => setSelectedId(item.id)} className={selectedId === item.id ? "selected" : ""}>
                    <SourceMark name={item.source} />
                    <span><strong>{item.source}</strong><small>{item.task}</small></span>
                    <Status value={item.state} />
                  </button>
                ))}
              </div>
            )}
          </Panel>

          <Panel title="Active workstreams" eyebrow="26 enrolled machines" action={<span className="live-time"><span /> Updated 10:58:15 AM</span>}>
            <div className="data-table workstream-table">
              <div className="table-head">
                <span>Source</span><span>Task</span><span>Machine</span><span>State</span><span>Progress</span><span>Evidence</span><span>Updated</span>
              </div>
              {filtered.map((item) => (
                <button className={`table-row ${selectedId === item.id ? "selected" : ""}`} type="button" key={item.id} onClick={() => setSelectedId(item.id)}>
                  <span className="source-cell"><SourceMark name={item.source} size="sm" />{item.source}</span>
                  <span>{item.task}</span>
                  <span className="mono">{item.machine}</span>
                  <span><Status value={item.state} /></span>
                  <span>{item.progress ? <Progress value={item.progress} /> : "—"}</span>
                  <span>{item.evidence.toLocaleString()}</span>
                  <span>{item.updated}</span>
                </button>
              ))}
            </div>
          </Panel>
        </div>

        <aside className="inspector">
          <div className="inspector-title">
            <SourceMark name={selected.source} />
            <div><strong>{selected.task}</strong><span>{selected.source}</span></div>
            <button className="icon-button" type="button"><IconExternalLink size={17} /></button>
          </div>
          <Segmented
            value={inspectorTab}
            onChange={setInspectorTab}
            options={[{ value: "inspector", label: "Inspector" }, { value: "evidence", label: `Evidence (${selected.evidence.toLocaleString()})` }, { value: "log", label: "Log" }]}
          />

          {inspectorTab === "inspector" && (
            <div className="inspector-scroll">
              <section className="inspector-section">
                <h3>Machine identity (signed)</h3>
                <KeyValue label="Machine">{selected.machine}</KeyValue>
                <KeyValue label="Machine ID"><CopyValue>m-7f3c2a8d</CopyValue></KeyValue>
                <KeyValue label="Public key"><CopyValue>ed25519:3f2a…9b7e</CopyValue></KeyValue>
                <KeyValue label="Signed by"><span className="good-text">Clark Root CA</span></KeyValue>
                <KeyValue label="Last heartbeat">10:58:15 AM (12s ago)</KeyValue>
              </section>
              <section className="inspector-section">
                <h3>Lease & fence</h3>
                <KeyValue label="Lease ID"><CopyValue>lease-184-aws-02-001</CopyValue></KeyValue>
                <KeyValue label="Lease TTL">15 min</KeyValue>
                <KeyValue label="Fence"><span className="good-text">Enabled · 42</span></KeyValue>
              </section>
              <section className="inspector-section">
                <h3>Pagination progress</h3>
                <KeyValue label="Service">organizations:listAccounts</KeyValue>
                <KeyValue label="Pages">32 / 47</KeyValue>
                <KeyValue label="Items">1,276 / ~1,900</KeyValue>
                <Progress value={68} label="68%" />
              </section>
              <section className="inspector-section">
                <h3>Evidence receipt</h3>
                <KeyValue label="Evidence items">{selected.evidence.toLocaleString()}</KeyValue>
                <KeyValue label="Evidence signed"><span className="good-text">100%</span></KeyValue>
                <KeyValue label="Validation">All good</KeyValue>
              </section>
              <section className="inspector-section">
                <h3>Scope</h3>
                <KeyValue label="Account">Root (111111111111)</KeyValue>
                <KeyValue label="Regions">All enabled (23)</KeyValue>
                <KeyValue label="Partitions">aws, aws-us-gov</KeyValue>
                <KeyValue label="Excluded OUs">None</KeyValue>
              </section>
              <section className="inspector-section safe-states">
                <h3>Safe terminal states</h3>
                {["Exhausted", "Rate limited", "Access denied", "Empty", "Unreachable", "Error"].map((state) => <div key={state}><IconShieldCheck size={15} />{state}<span>Accounted</span></div>)}
              </section>
            </div>
          )}

          {inspectorTab === "evidence" && (
            <div className="inspector-scroll receipt-list">
              {Array.from({ length: 8 }, (_, index) => (
                <div key={index}>
                  <IconShieldCheck size={16} />
                  <span><strong>Signed page receipt #{32 - index}</strong><small>SHA-256 {`a${index}f3…${index}29c`} · {index + 1} min ago</small></span>
                </div>
              ))}
            </div>
          )}

          {inspectorTab === "log" && (
            <div className="inspector-scroll run-log">
              {["Page 32 accepted · 40 new entities", "Evidence object verified", "Lease heartbeat renewed", "Page 31 accepted · 38 new entities", "Rate window healthy"].map((line, index) => (
                <div key={line}><IconClock size={14} /><span>{`10:58:${15 - index * 3}`}</span><strong>{line}</strong></div>
              ))}
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}
