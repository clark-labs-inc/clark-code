import { useMemo, useState } from "react";
import {
  IconCheck,
  IconCopy,
  IconDownload,
  IconFingerprint,
  IconPlus,
  IconRefresh,
  IconSearch,
  IconShieldLock,
  IconTerminal,
} from "@tabler/icons-react";
import { machines } from "../data";
import { AppHeader } from "../components/Layout";
import { Metric, Panel, Status } from "../components/Ui";

export function Machines({ onNotice }) {
  const [query, setQuery] = useState("");
  const [enrolling, setEnrolling] = useState(false);
  const [platform, setPlatform] = useState("Linux");
  const filtered = useMemo(() => machines.filter((machine) => `${machine.name} ${machine.platform} ${machine.location}`.toLowerCase().includes(query.toLowerCase())), [query]);

  return (
    <div className="page machines-page">
      <AppHeader title="Machines" subtitle="Enrolled collection origins that execute bounded Scout tasks.">
        <button className="secondary-button" type="button"><IconRefresh size={17} /> Refresh</button>
        <button className="primary" type="button" onClick={() => setEnrolling(true)}><IconPlus size={18} /> Enroll machine</button>
      </AppHeader>

      <div className="metric-strip compact-metrics">
        <Metric value="26" label="Enrolled" meta="Across 4 platforms" />
        <Metric value="22" label="Active now" meta="Heartbeats under 2 minutes" tone="green" />
        <Metric value="3" label="Need attention" meta="2 offline · 1 upgrade" tone="amber" />
        <Metric value="41" label="Leased tasks" meta="All fenced and resumable" />
        <Metric value="100%" label="Signed evidence" meta="Machine identities verified" tone="green" />
      </div>

      <Panel
        title="Enterprise fleet"
        eyebrow="Collector origins"
        action={<label className="search-control compact"><IconSearch size={16} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search machines" /></label>}
      >
        <div className="data-table machine-table">
          <div className="table-head"><span>Machine</span><span>Platform</span><span>Location</span><span>Version</span><span>Status</span><span>Heartbeat</span><span>Active tasks</span><span /></div>
          {filtered.map((machine) => (
            <button className="table-row" type="button" key={machine.id}>
              <span className="machine-name"><span className="device-icon"><IconTerminal size={17} /></span><span><strong>{machine.name}</strong><small>{machine.id}</small></span></span>
              <span>{machine.platform} · {machine.architecture}</span>
              <span>{machine.location}</span>
              <span className="mono">{machine.version}</span>
              <span><Status value={machine.state} label={machine.state === "upgrade" ? "Upgrade available" : machine.state} /></span>
              <span>{machine.heartbeat}</span>
              <span>{machine.tasks}</span>
              <span className="row-actions">Manage</span>
            </button>
          ))}
        </div>
      </Panel>

      <div className="three-column machine-info-grid">
        <Panel title="Identity boundary" eyebrow="Trust">
          <div className="feature-copy"><IconFingerprint size={24} /><div><strong>Per-machine Ed25519 identities</strong><p>Private seed material never leaves the owner-only host directory. Clark stores only public enrollment records.</p></div></div>
        </Panel>
        <Panel title="Execution boundary" eyebrow="Safety">
          <div className="feature-copy"><IconShieldLock size={24} /><div><strong>Typed, read-only tasks</strong><p>Every task is scope-bound, leased, fenced, and limited by records, pages, time, bytes, and cost.</p></div></div>
        </Panel>
        <Panel title="Portable fleet" eyebrow="Coverage">
          <div className="feature-copy"><IconDownload size={24} /><div><strong>macOS, Linux, Windows, and SSH</strong><p>The same signed wire contract and deterministic receipts work across every supported target.</p></div></div>
        </Panel>
      </div>

      {enrolling && (
        <div className="drawer-backdrop" onMouseDown={() => setEnrolling(false)}>
          <aside className="drawer" onMouseDown={(event) => event.stopPropagation()}>
            <header><div><div className="eyebrow">Enroll collector</div><h2>Add a machine</h2></div><button type="button" onClick={() => setEnrolling(false)}>Close</button></header>
            <div className="drawer-body">
              <div className="platform-picker">
                {["Linux", "macOS", "Windows", "SSH target"].map((item) => <button key={item} className={platform === item ? "active" : ""} type="button" onClick={() => setPlatform(item)}>{item}</button>)}
              </div>
              <div className="enroll-step"><span>1</span><div><strong>Download the signed collector</strong><p>Platform: {platform} · Architecture detected during enrollment.</p><button className="secondary-button" type="button"><IconDownload size={17} /> Download package</button></div></div>
              <div className="enroll-step"><span>2</span><div><strong>Run the one-time enrollment command</strong><pre>clark-scout enroll --workspace global-production</pre><button className="text-button" type="button"><IconCopy size={15} /> Copy command</button></div></div>
              <div className="enroll-step"><span>3</span><div><strong>Verify the fingerprint</strong><p>The machine will appear as pending until an organization admin verifies its public key.</p></div></div>
              <div className="safety-banner"><IconShieldLock size={21} /><div><strong>No credentials are copied to Clark</strong><p>Adapters execute on the enrolled target and return normalized, value-free receipts.</p></div></div>
            </div>
            <footer><button className="secondary-button" type="button" onClick={() => setEnrolling(false)}>Cancel</button><button className="primary" type="button" onClick={() => { setEnrolling(false); onNotice("Enrollment session created for a new Linux collector."); }}><IconCheck size={17} /> Create enrollment</button></footer>
          </aside>
        </div>
      )}
    </div>
  );
}
