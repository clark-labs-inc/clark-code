import { useMemo, useState } from "react";
import {
  IconArrowRight,
  IconCalendar,
  IconCheck,
  IconChevronLeft,
  IconChevronRight,
  IconExternalLink,
  IconFileCheck,
  IconFlask,
  IconHistory,
  IconSearch,
  IconShieldCheck,
} from "@tabler/icons-react";
import { evidenceRows, provenance } from "../data";
import { AppHeader } from "../components/Layout";
import { CopyValue, KeyValue, Segmented, SourceMark, Status } from "../components/Ui";

export function Evidence({ onNotice }) {
  const [selectedId, setSelectedId] = useState("aurora");
  const [section, setSection] = useState("claims");
  const [query, setQuery] = useState("");
  const selected = evidenceRows.find((row) => row.id === selectedId) || evidenceRows[0];
  const filtered = useMemo(() => evidenceRows.filter((row) => `${row.artifact} ${row.source} ${row.collector}`.toLowerCase().includes(query.toLowerCase())), [query]);

  return (
    <div className="page evidence-page">
      <AppHeader title="Evidence" subtitle="Inspect the artifacts, tests, and corrections behind every system claim.">
        <label className="search-control"><IconSearch size={17} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search evidence" /></label>
      </AppHeader>

      <div className="subnav">
        <Segmented
          value={section}
          onChange={setSection}
          options={[{ value: "claims", label: "Investigations" }, { value: "ledger", label: "Evidence ledger" }, { value: "retractions", label: "Corrections & retractions" }]}
        />
        <span className="data-freshness"><IconHistory size={16} /> Data as of Jul 26, 2026 10:58 AM</span>
      </div>

      <section className="claim-header">
        <div>
          <div className="eyebrow">Claim · INV-2026-0725-0006</div>
          <h2>Checkout API writes orders to Aurora production</h2>
          <p>Investigated by Alex Morgan · Created Jul 25, 2026 2:14 PM</p>
        </div>
        <div className="verdict-box">
          <span className="verdict-icon"><IconCheck size={18} /></span>
          <div><strong>Supported — T2 live-state confirmation</strong><small>7 supporting artifacts · 2 independent reproductions</small></div>
          <button className="primary" type="button" onClick={() => onNotice("Claim adjudicated and signed by Alex Morgan.")}><IconShieldCheck size={18} /> Adjudicate claim</button>
        </div>
      </section>

      <div className="temporal-controls">
        <label><span>Effective at</span><button type="button"><IconCalendar size={17} /> Jul 26, 2026&nbsp; 10:58 AM</button></label>
        <div className="stepper"><button type="button" aria-label="Previous observation"><IconChevronLeft size={16} /></button><button type="button" aria-label="Next observation"><IconChevronRight size={16} /></button></div>
        <label><span>Known at</span><button type="button"><IconCalendar size={17} /> Jul 26, 2026&nbsp; 10:58 AM</button></label>
      </div>

      <div className="evidence-workspace">
        <div className="evidence-main">
          <section className="provenance-section">
            <div className="section-heading"><span className="eyebrow">Provenance</span><small>Why this claim exists</small></div>
            <div className="provenance-chain">
              {provenance.map((item, index) => (
                <div className="provenance-item" key={item.label}>
                  <div className="provenance-node">
                    <SourceMark name={item.label} />
                    <strong>{item.label}</strong>
                    <small>{item.detail}</small>
                  </div>
                  {index < provenance.length - 1 && <IconArrowRight className="provenance-arrow" size={18} />}
                </div>
              ))}
            </div>
          </section>

          <section className="panel evidence-ledger">
            <header className="panel-header"><div><div className="eyebrow">Evidence ledger</div><h2>8 artifacts</h2></div></header>
            <div className="data-table evidence-table">
              <div className="table-head">
                <span>Artifact</span><span>Signed hash</span><span>Collected by</span><span>Source</span><span>Proof</span><span>Classification</span><span>Observed</span><span>Status</span>
              </div>
              <div className="table-group-label">Supporting evidence (7)</div>
              {filtered.filter((row) => row.status === "verified").map((row) => (
                <button className={`table-row ${selectedId === row.id ? "selected" : ""}`} key={row.id} type="button" onClick={() => setSelectedId(row.id)}>
                  <span className="artifact-cell"><IconFileCheck size={17} /><span><strong>{row.artifact}</strong><small>{row.detail}</small></span></span>
                  <span className="mono">{row.hash}</span>
                  <span className="mono">{row.collector}</span>
                  <span>{row.source}</span>
                  <span>{row.tier}</span>
                  <span><span className={`classification classification-${row.classification.toLowerCase()}`}>{row.classification}</span></span>
                  <span>{row.observed}</span>
                  <span><Status value={row.status} /></span>
                </button>
              ))}
              {filtered.some((row) => row.status === "superseded") && <div className="table-group-label correction">Superseded / corrections (1)</div>}
              {filtered.filter((row) => row.status === "superseded").map((row) => (
                <button className={`table-row ${selectedId === row.id ? "selected" : ""}`} key={row.id} type="button" onClick={() => setSelectedId(row.id)}>
                  <span className="artifact-cell"><IconFileCheck size={17} /><span><strong>{row.artifact}</strong><small>{row.detail}</small></span></span>
                  <span className="mono">{row.hash}</span>
                  <span className="mono">{row.collector}</span>
                  <span>{row.source}</span>
                  <span>{row.tier}</span>
                  <span><span className="classification">{row.classification}</span></span>
                  <span>{row.observed}</span>
                  <span><Status value={row.status} /></span>
                </button>
              ))}
            </div>
            <div className="reproduction-strip">
              <div><IconFlask size={19} /><span><strong>Independent reproductions (2)</strong><small>Both passed against staging or synthetic sinks.</small></span></div>
              <div><Status value="verified" label="Replay to staging Aurora" /><Status value="verified" label="Synthetic order write" /></div>
            </div>
          </section>
        </div>

        <aside className="inspector evidence-inspector">
          <div className="inspector-title">
            <SourceMark name={selected.source} />
            <div><strong>{selected.artifact}</strong><span>{selected.detail}</span></div>
            <button className="icon-button" type="button"><IconExternalLink size={17} /></button>
          </div>
          <div className="inspector-scroll">
            <section className="inspector-section">
              <h3>Evidence identity</h3>
              <KeyValue label="Artifact">{selected.artifact}</KeyValue>
              <KeyValue label="Proof tier">{selected.tier} · Live state</KeyValue>
              <KeyValue label="Observed at">Jul 26, 2026 {selected.observed}</KeyValue>
              <KeyValue label="Classification">{selected.classification}</KeyValue>
            </section>
            <section className="inspector-section">
              <h3>Scope</h3>
              <KeyValue label="Resource">cluster/acme-orders-prod</KeyValue>
              <KeyValue label="Region">us-east-1</KeyValue>
              <KeyValue label="Account">123456789012</KeyValue>
              <KeyValue label="Time window">10:16 AM – 10:21 AM</KeyValue>
            </section>
            <section className="inspector-section">
              <h3>Signed by machine</h3>
              <KeyValue label="Machine ID"><CopyValue>clark-aurora-sensor-1</CopyValue></KeyValue>
              <KeyValue label="Identity">arn:aws:iam::123…:role/clark-sensor</KeyValue>
              <KeyValue label="Signature"><CopyValue>ed25519:6f3a…9b7e</CopyValue></KeyValue>
              <Status value="verified" label="Signature verified" />
            </section>
            <section className="inspector-section">
              <h3>Replay recipe</h3>
              <p className="inspector-note">Use Clark Repro Lab runbook AURORA_CONN_PG_STAT_ACTIVITY_V1.</p>
              <button className="text-button" type="button">View runbook <IconExternalLink size={14} /></button>
            </section>
            <section className="inspector-section">
              <h3>Attempt falsification</h3>
              <p className="inspector-note">Design a test or change that could prove this claim false.</p>
              <button className="secondary-button" type="button" onClick={() => onNotice("Falsification experiment drafted in Clark Repro Lab.")}><IconFlask size={17} /> Attempt falsification</button>
            </section>
          </div>
        </aside>
      </div>
    </div>
  );
}
