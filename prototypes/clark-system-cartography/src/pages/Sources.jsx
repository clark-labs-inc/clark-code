import { useMemo, useState } from "react";
import {
  IconAlertCircle,
  IconChevronRight,
  IconCloud,
  IconPlus,
  IconRefresh,
  IconSearch,
  IconShieldLock,
} from "@tabler/icons-react";
import { sources } from "../data";
import { AppHeader } from "../components/Layout";
import { Metric, Panel, SourceMark, Status } from "../components/Ui";

export function Sources({ onNotice }) {
  const [query, setQuery] = useState("");
  const filtered = useMemo(() => sources.filter((source) => `${source.name} ${source.kind} ${source.namespace}`.toLowerCase().includes(query.toLowerCase())), [query]);

  return (
    <div className="page sources-page">
      <AppHeader title="Sources & adapters" subtitle="Authoritative control planes and credential contexts available to Scout.">
        <button className="secondary-button" type="button"><IconRefresh size={17} /> Verify all</button>
        <button className="primary" type="button" onClick={() => onNotice("Source registration wizard opened in safe, metadata-only mode.")}><IconPlus size={18} /> Add source</button>
      </AppHeader>

      <div className="metric-strip compact-metrics">
        <Metric value="8" label="Registered sources" meta="Across 7 control-plane types" />
        <Metric value="6" label="Connected" meta="Identity verified" tone="green" />
        <Metric value="1" label="Needs attention" meta="Datadog scope denied" tone="amber" />
        <Metric value="1" label="Unreachable" meta="Internal service catalog" tone="red" />
        <Metric value="0" label="Secret payloads" meta="Never retrieved" tone="green" />
      </div>

      <Panel
        title="Control planes"
        eyebrow="Registered authority"
        action={<label className="search-control compact"><IconSearch size={16} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search sources" /></label>}
      >
        <div className="source-list">
          {filtered.map((source) => (
            <button type="button" key={source.id}>
              <SourceMark name={source.name} />
              <span className="source-main"><strong>{source.name}</strong><small>{source.namespace}</small></span>
              <span><small>Type</small><strong>{source.kind}</strong></span>
              <span><small>Coverage</small><strong>{source.coverage}</strong></span>
              <span><small>Last verified</small><strong>{source.last}</strong></span>
              <Status value={source.state} />
              <IconChevronRight size={17} />
            </button>
          ))}
        </div>
      </Panel>

      <div className="three-column source-info-grid">
        <Panel title="Target-affine credentials" eyebrow="Boundary">
          <div className="feature-copy"><IconShieldLock size={24} /><div><strong>Credentials stay on their machine</strong><p>Scout selects and verifies candidates on the enrolled target. Tokens and provider credentials never enter Clark’s model context.</p></div></div>
        </Panel>
        <Panel title="Curated operation registry" eyebrow="Routing">
          <div className="feature-copy"><IconCloud size={24} /><div><strong>Typed operations only</strong><p>Every adapter route pins its provider, query, projection, identity authority, limits, and safe pagination behavior.</p></div></div>
        </Panel>
        <Panel title="Missing instruments stay visible" eyebrow="Coverage">
          <div className="feature-copy"><IconAlertCircle size={24} /><div><strong>No silent omissions</strong><p>Denied, unreachable, unsupported, unsafe, stale, truncated, and untested cells remain explicit gaps in the map.</p></div></div>
        </Panel>
      </div>
    </div>
  );
}
