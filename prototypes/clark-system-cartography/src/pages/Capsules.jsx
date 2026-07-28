import { useState } from "react";
import {
  IconBox,
  IconCheck,
  IconClock,
  IconCpu,
  IconFingerprint,
  IconFlask,
  IconLock,
  IconPlayerPlay,
  IconRefresh,
  IconShieldCheck,
} from "@tabler/icons-react";
import { AppHeader } from "../components/Layout";
import { KeyValue, Metric, Panel, Status } from "../components/Ui";

const capsules = [
  ["normalize-adapter-page", "Deterministic graph normalization", "4adce0c8…", "generation 7", "qualified"],
  ["parse-ci-manifest", "Parse structured CI manifests", "a932bf10…", "generation 4", "qualified"],
  ["validate-contract", "Validate simulation contract shape", "9f01bc47…", "generation 5", "qualified"],
  ["hash-evidence-set", "Canonical evidence-set commitment", "78b412ae…", "generation 2", "qualified"],
  ["classify-schema", "Apply monotone classification policy", "c18d9e20…", "generation 3", "review"],
];

const invocations = [
  ["normalize-adapter-page", "scout-aws-02", "42 ms", "18,441 records", "verified", "10:58:12 AM"],
  ["validate-contract", "scout-gcp-01", "8 ms", "12 contracts", "verified", "10:56:04 AM"],
  ["hash-evidence-set", "scout-gh-01", "19 ms", "1,842 objects", "verified", "10:54:22 AM"],
  ["parse-ci-manifest", "scl-build-host", "31 ms", "184 workflows", "verified", "10:51:11 AM"],
];

export function Capsules({ onNotice }) {
  const [selected, setSelected] = useState(capsules[0]);
  const [invoking, setInvoking] = useState(false);
  const invoke = () => {
    setInvoking(true);
    window.setTimeout(() => {
      setInvoking(false);
      onNotice("Capsule invocation completed with byte-identical native and WASM output.");
    }, 900);
  };

  return (
    <div className="page capsules-page">
      <AppHeader title="Isolation capsules" subtitle="Signed, zero-import WASM transforms for deterministic work on untrusted inputs.">
        <button className="secondary-button" type="button"><IconRefresh size={17} /> Verify registry</button>
        <button className="primary" type="button" onClick={invoke}><IconPlayerPlay size={18} /> {invoking ? "Invoking…" : "Test selected capsule"}</button>
      </AppHeader>

      <div className="metric-strip compact-metrics">
        <Metric value="5" label="Registered capsules" meta="Administrator signed" />
        <Metric value="4" label="Qualified" meta="Native/WASM parity" tone="green" />
        <Metric value="0" label="Ambient imports" meta="Filesystem, network, clock, process" tone="green" />
        <Metric value="5" label="Platforms" meta="Local, CPU, SCL, Ubuntu, Windows" />
        <Metric value="100%" label="Digest verified" meta="Module, input, and output" tone="green" />
      </div>

      <div className="capsule-layout">
        <Panel title="Signed registry" eyebrow="Generation-controlled">
          <div className="capsule-list">
            {capsules.map((capsule) => (
              <button key={capsule[0]} type="button" className={selected[0] === capsule[0] ? "selected" : ""} onClick={() => setSelected(capsule)}>
                <span className="capsule-icon"><IconBox size={19} /></span>
                <span><strong>{capsule[0]}</strong><small>{capsule[1]}</small></span>
                <span className="mono">{capsule[2]}</span>
                <span>{capsule[3]}</span>
                <Status value={capsule[4] === "qualified" ? "verified" : "attention"} label={capsule[4]} />
              </button>
            ))}
          </div>
        </Panel>

        <aside className="inspector capsule-inspector">
          <div className="inspector-title"><span className="capsule-icon"><IconBox size={20} /></span><div><strong>{selected[0]}</strong><span>{selected[1]}</span></div><Status value={selected[4] === "qualified" ? "verified" : "attention"} label={selected[4]} /></div>
          <div className="inspector-scroll">
            <section className="inspector-section">
              <h3>Registry binding</h3>
              <KeyValue label="Module digest">{selected[2]}</KeyValue>
              <KeyValue label="Generation">{selected[3]}</KeyValue>
              <KeyValue label="Input schema">scout.adapter-page.v3</KeyValue>
              <KeyValue label="Output schema">scout.graph-events.v2</KeyValue>
              <KeyValue label="Signed by">Clark Capsule Root 2026</KeyValue>
            </section>
            <section className="inspector-section">
              <h3>Host-owned limits</h3>
              <KeyValue label="Memory">64 MiB</KeyValue>
              <KeyValue label="Fuel">10,000,000</KeyValue>
              <KeyValue label="Input">8 MiB maximum</KeyValue>
              <KeyValue label="Output">8 MiB maximum</KeyValue>
              <KeyValue label="Concurrency">4 per target</KeyValue>
            </section>
            <section className="inspector-section">
              <h3>Isolation receipt</h3>
              <div className="isolation-checks">
                {["Zero imports", "Fresh instance", "Finite fuel", "Signed module", "Bounded memory", "Output digest"].map((item) => <span key={item}><IconCheck size={15} />{item}</span>)}
              </div>
            </section>
          </div>
        </aside>
      </div>

      <Panel title="Recent invocations" eyebrow="Authenticated receipts">
        <div className="data-table invocation-table">
          <div className="table-head"><span>Capsule</span><span>Target</span><span>Duration</span><span>Input</span><span>Result</span><span>Time</span></div>
          {invocations.map((row) => <div className="table-row" key={`${row[0]}-${row[1]}`}><span className="strong-cell"><IconFlask size={16} />{row[0]}</span><span>{row[1]}</span><span>{row[2]}</span><span>{row[3]}</span><span><Status value={row[4]} /></span><span>{row[5]}</span></div>)}
        </div>
      </Panel>

      <div className="three-column capsule-guarantees">
        <Panel title="No ambient authority" eyebrow="Imports"><div className="feature-copy"><IconLock size={24} /><div><strong>Zero filesystem, network, clock, process, or credential imports</strong><p>Ambient discovery stays behind brokered, typed host capabilities.</p></div></div></Panel>
        <Panel title="Content addressed" eyebrow="Integrity"><div className="feature-copy"><IconFingerprint size={24} /><div><strong>Module, input, and output digests in every receipt</strong><p>Any change creates a different identity and invalidates silent replay.</p></div></div></Panel>
        <Panel title="Portable parity" eyebrow="Qualification"><div className="feature-copy"><IconCpu size={24} /><div><strong>Byte-identical native and WASM results</strong><p>Qualified on macOS, Linux, Windows, SSH targets, and UTM guests.</p></div></div></Panel>
      </div>
    </div>
  );
}
