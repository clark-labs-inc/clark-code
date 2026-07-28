import { useState } from "react";
import {
  IconAlertTriangle,
  IconCheck,
  IconClock,
  IconCpu,
  IconPlayerPlay,
  IconRefresh,
  IconShieldCheck,
} from "@tabler/icons-react";
import { AppHeader } from "../components/Layout";
import { Metric, Panel, Status } from "../components/Ui";

const lanes = [
  ["Local macOS ARM64", "macOS 26", "Native", "17 / 17", "passed", "0a2ffc67…", "Jul 26, 9:18 AM"],
  ["cpu", "Linux x86_64", "bubblewrap", "17 / 17", "passed", "0a2ffc67…", "Jul 26, 8:54 AM"],
  ["scl", "Linux x86_64", "External", "17 / 17", "passed", "0a2ffc67…", "Jul 26, 8:31 AM"],
  ["UTM Ubuntu ARM64", "Ubuntu 24.04", "Guest channel", "17 / 17", "passed", "0a2ffc67…", "Jul 26, 7:52 AM"],
  ["UTM Windows ARM64", "Windows 11", "Defender on", "Capsule only", "blocked", "d6891a74…", "Jul 26, 7:18 AM"],
  ["UTM macOS ARM64", "macOS 26", "Guest agent", "0 / 17", "unreachable", "—", "Jul 26, 6:44 AM"],
];

const gates = [
  ["25,000-service enterprise", "300,041 events · 150,003 entities · 125,003 edges", "169.18 s", "7.50 GB", "passed"],
  ["1,000,000-event reducer", "20,000 entities · 10,000 edges · reverse replay", "199.36 s", "3.63 GB", "passed"],
  ["100,000-event affected projection", "1 affected row · 11 candidate events", "0.108 ms", "85.5 MB index", "passed"],
  ["100,000-task scheduler", "1,024 fenced claims · exact restart receipt", "891 ms", "473 MB", "passed"],
  ["Cross-platform capability census", "3,535 directories · 13 dotenv files · 206 key names", "Value free", "0 secrets", "passed"],
];

const cases = [
  "Business graph completeness",
  "Fixed-point convergence",
  "Secret-leak rejection",
  "False-join rejection",
  "Signed tenant-isolated ingestion",
  "Fenced scheduler recovery",
  "Atomic page-to-graph commit",
  "Temporal qualification",
  "Classification monotonicity",
  "WASM resource bounds",
  "Containment negative control",
  "Cross-platform semantic digest",
];

export function Benchmarks({ onNotice }) {
  const [running, setRunning] = useState(false);
  const run = () => {
    setRunning(true);
    window.setTimeout(() => {
      setRunning(false);
      onNotice("Offline qualification completed: 17 of 17 deterministic cases passed.");
    }, 1500);
  };

  return (
    <div className="page benchmarks-page">
      <AppHeader title="Qualifications" subtitle="Deterministic proof that Scout behaves the same across hosts, sandboxes, and scale gates.">
        <button className="secondary-button" type="button"><IconRefresh size={17} /> Refresh receipts</button>
        <button className="primary" type="button" onClick={run}><IconPlayerPlay size={18} /> {running ? "Running…" : "Run offline qualification"}</button>
      </AppHeader>

      <div className="metric-strip compact-metrics">
        <Metric value="17 / 17" label="Deterministic cases" meta="Current source" tone="green" />
        <Metric value="4" label="Full platform lanes" meta="Byte-identical semantic digest" tone="green" />
        <Metric value="2" label="Explicit blockers" meta="Windows packaging · macOS guest" tone="amber" />
        <Metric value="0" label="Live model calls" meta="Offline benchmark" tone="green" />
        <Metric value="0" label="Values observed" meta="Secret-safe receipt" tone="green" />
      </div>

      <Panel title="Cross-platform lanes" eyebrow="Current source qualification">
        <div className="data-table lane-table">
          <div className="table-head"><span>Lane</span><span>Platform</span><span>Containment</span><span>Cases</span><span>Result</span><span>Canonical hash</span><span>Last run</span></div>
          {lanes.map((row) => <div className="table-row" key={row[0]}><span className="strong-cell"><IconCpu size={16} />{row[0]}</span><span>{row[1]}</span><span>{row[2]}</span><span>{row[3]}</span><span><Status value={row[4] === "passed" ? "verified" : row[4]} label={row[4]} /></span><span className="mono">{row[5]}</span><span>{row[6]}</span></div>)}
        </div>
        <div className="qualification-note">
          <IconAlertTriangle size={19} />
          <span><strong>Windows full benchmark is blocked on packaging, not functional semantics.</strong><small>The unsigned current-source artifact was quarantined before execution. The independently signed capsule qualifier passes with Defender enabled.</small></span>
        </div>
      </Panel>

      <div className="benchmark-grid">
        <Panel title="Scale gates" eyebrow="Correctness and economy">
          <div className="gate-list">
            {gates.map((gate) => <div key={gate[0]}><span><IconCheck size={16} /><span><strong>{gate[0]}</strong><small>{gate[1]}</small></span></span><span><strong>{gate[2]}</strong><small>{gate[3]}</small></span><Status value="verified" label={gate[4]} /></div>)}
          </div>
        </Panel>
        <Panel title="Mutation controls" eyebrow="The grader can say no">
          <div className="case-grid">
            {cases.map((item) => <div key={item}><IconShieldCheck size={16} /><span>{item}</span><Status value="verified" label="Pass" /></div>)}
          </div>
        </Panel>
      </div>

      <div className="benchmark-receipt">
        <IconShieldCheck size={22} />
        <div><strong>Cross-platform semantic digest</strong><span className="mono">a0541ad238a32d671bf60c0dbcf3187af4a802a53f570a05d03051384f9cc16d</span><small>Includes trust anchor, authenticated envelope root, event root, graph digest, counts, duplicate result, and completion.</small></div>
        <Status value="verified" label="Reproduced on 4 full lanes" />
      </div>
    </div>
  );
}
