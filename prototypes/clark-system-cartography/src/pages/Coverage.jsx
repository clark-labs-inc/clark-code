import { useMemo, useState } from "react";
import {
  IconAlertCircle,
  IconCheck,
  IconChevronRight,
  IconClock,
  IconFilter,
  IconRefresh,
  IconSearch,
  IconShieldCheck,
} from "@tabler/icons-react";
import { AppHeader } from "../components/Layout";
import { Metric, Panel, SourceMark, Status } from "../components/Ui";

const rows = [
  ["AWS Organizations", "Root organization", "Global", "Accounts", "verified", "1,276", "32 pages", "12 min ago"],
  ["AWS Resource Explorer", "Production accounts", "17 regions", "Resources", "scanning", "18,441", "68%", "Now"],
  ["GitHub Enterprise", "acme-corp", "Global", "Repositories", "verified", "184", "1 page", "28 min ago"],
  ["Google Cloud", "organizations/81720341", "4 folders", "Projects", "verified", "12", "Complete", "34 min ago"],
  ["Google Cloud Asset", "12 projects", "Global", "Resources", "resumable", "9,208", "Cursor saved", "9 min ago"],
  ["Okta", "acme.okta.com", "Global", "Directory", "verified", "4,608", "Complete", "41 min ago"],
  ["Kubernetes", "8 production clusters", "4 regions", "Workloads", "scanning", "3,118", "72%", "Now"],
  ["Snowflake", "acme-org", "3 accounts", "Data objects", "verified", "1,990", "Complete", "1 hr ago"],
  ["Datadog", "acme.datadoghq.com", "Global", "Integrations", "denied", "0", "Scope denied", "2 hrs ago"],
  ["PagerDuty", "acme.pagerduty.com", "Global", "Services", "verified", "423", "Complete", "1 hr ago"],
  ["Jenkins", "jenkins.acme.internal", "us-east-1", "Pipelines", "unreachable", "0", "DNS timeout", "4 hrs ago"],
  ["Service Catalog", "catalog.acme.internal", "Global", "Ownership", "stale", "1,106", "3 days old", "3 days ago"],
];

const dimensions = [
  ["Source provenance", 93, "1,161 / 1,248 services"],
  ["Deployment identity", 89, "1,111 / 1,248 services"],
  ["Runtime identity", 88, "1,099 / 1,248 services"],
  ["Dependencies", 84, "1,048 / 1,248 services"],
  ["Ownership", 82, "1,023 / 1,248 services"],
  ["Observability", 79, "986 / 1,248 services"],
  ["Behavioral contracts", 76, "948 / 1,248 services"],
];

export function Coverage() {
  const [status, setStatus] = useState("all");
  const [query, setQuery] = useState("");
  const filtered = useMemo(() => rows.filter((row) => (
    (status === "all" || row[4] === status) && row.join(" ").toLowerCase().includes(query.toLowerCase())
  )), [status, query]);

  return (
    <div className="page coverage-page">
      <AppHeader title="Coverage" subtitle="Every expected control-plane cell must end in an explicit terminal state.">
        <button className="secondary-button" type="button"><IconRefresh size={17} /> Refresh status</button>
      </AppHeader>

      <div className="metric-strip compact-metrics">
        <Metric value="312" label="Expected cells" meta="Charter v12" />
        <Metric value="258" label="Verified" meta="Authoritative enumeration complete" tone="green" />
        <Metric value="36" label="Explicit gaps" meta="Visible and actionable" tone="amber" />
        <Metric value="18" label="In progress" meta="Resumable pages or active leases" />
        <Metric value="0" label="Silent omissions" meta="Required for completion" tone="green" />
      </div>

      <div className="coverage-layout">
        <Panel
          title="Control-plane matrix"
          eyebrow="Adapter × context × scope × region × resource"
          action={(
            <div className="table-actions">
              <label className="search-control compact"><IconSearch size={16} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search cells" /></label>
              <button className="secondary-button icon-only" type="button"><IconFilter size={17} /></button>
            </div>
          )}
        >
          <div className="status-filter">
            {["all", "verified", "scanning", "resumable", "denied", "unreachable", "stale"].map((item) => (
              <button key={item} type="button" onClick={() => setStatus(item)} className={status === item ? "active" : ""}>{item === "all" ? "All cells" : item}</button>
            ))}
          </div>
          <div className="data-table coverage-table">
            <div className="table-head"><span>Adapter</span><span>Authority scope</span><span>Region / project</span><span>Resource kind</span><span>Status</span><span>Observed</span><span>Receipt</span><span>Updated</span><span /></div>
            {filtered.map((row) => (
              <button className="table-row" type="button" key={`${row[0]}-${row[3]}`}>
                <span className="source-cell"><SourceMark name={row[0]} size="sm" />{row[0]}</span>
                <span>{row[1]}</span><span>{row[2]}</span><span>{row[3]}</span>
                <span><Status value={row[4]} /></span><span>{row[5]}</span><span>{row[6]}</span><span>{row[7]}</span><span><IconChevronRight size={15} /></span>
              </button>
            ))}
          </div>
        </Panel>

        <aside className="coverage-side">
          <Panel title="Business completeness" eyebrow="Production services">
            <div className="dimension-list">
              {dimensions.map(([name, value, detail]) => (
                <div key={name}>
                  <span><strong>{name}</strong><small>{detail}</small></span>
                  <div><span className="mini-track"><i style={{ width: `${value}%` }} /></span><strong>{value}%</strong></div>
                </div>
              ))}
            </div>
          </Panel>
          <Panel title="Completion contract" eyebrow="Fixed point">
            <div className="completion-list">
              <div><IconCheck size={17} /><span><strong>Pass 41 verified</strong><small>Event root 21a76e8b…</small></span></div>
              <div><IconCheck size={17} /><span><strong>Pass 42 verified</strong><small>Identical graph and membership roots</small></span></div>
              <div><IconClock size={17} /><span><strong>Fresh for 5h 42m</strong><small>Maximum pass age: 24 hours</small></span></div>
              <div><IconShieldCheck size={17} /><span><strong>Graph converged</strong><small>No new qualified entities or edges</small></span></div>
            </div>
          </Panel>
          <Panel title="Largest known unknown" eyebrow="Action required" className="known-unknown">
            <div><IconAlertCircle size={21} /><span><strong>Datadog integration scope denied</strong><p>Scout can enumerate monitors but cannot verify service integration ownership with the current token.</p><button type="button">Resolve access gap</button></span></div>
          </Panel>
        </aside>
      </div>
    </div>
  );
}
