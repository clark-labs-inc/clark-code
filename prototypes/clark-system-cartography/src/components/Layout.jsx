import { useEffect, useMemo, useState } from "react";
import {
  IconAdjustments,
  IconBell,
  IconBuilding,
  IconChartDots3,
  IconChevronDown,
  IconCirclesRelation,
  IconFileCheck,
  IconFlask,
  IconGauge,
  IconLayoutDashboard,
  IconNetwork,
  IconPlus,
  IconRadar,
  IconSearch,
  IconServer,
  IconSettings,
  IconShieldCheck,
  IconX,
} from "@tabler/icons-react";

const primaryItems = [
  ["overview", "Overview", IconLayoutDashboard],
  ["discoveries", "Discoveries", IconRadar],
  ["coverage", "Coverage", IconCirclesRelation],
  ["graph", "System graph", IconNetwork],
  ["evidence", "Evidence", IconFileCheck],
  ["simulations", "Simulations", IconFlask],
  ["machines", "Machines", IconServer],
  ["capsules", "Isolation capsules", IconShieldCheck],
  ["benchmarks", "Qualifications", IconGauge],
  ["governance", "Governance", IconShieldCheck],
];

export function Sidebar({ active, onNavigate, onNewDiscovery }) {
  return (
    <aside className="sidebar">
      <div className="brand-row">
        <button className="brand" type="button" onClick={() => onNavigate("overview")} aria-label="Clark home">
          clark<span />
        </button>
        <div className="brand-actions">
          <button type="button" aria-label="Notifications"><IconBell size={18} /></button>
        </div>
      </div>

      <button className="primary full" type="button" onClick={onNewDiscovery}>
        <IconPlus size={18} /> New discovery
      </button>

      <div className="nav-group">
        <div className="nav-label">System Cartography</div>
        {primaryItems.map(([id, label, Icon]) => (
          <button
            key={id}
            type="button"
            className={`nav-item ${active === id ? "active" : ""}`}
            onClick={() => onNavigate(id)}
          >
            <Icon size={18} />
            <span>{label}</span>
          </button>
        ))}
      </div>

      <div className="nav-group secondary">
        <div className="nav-label">Workspace</div>
        <button className="nav-item" type="button" onClick={() => onNavigate("sources")}>
          <IconAdjustments size={18} />
          <span>Sources & adapters</span>
        </button>
        <button className="nav-item" type="button" onClick={() => onNavigate("benchmarks")}>
          <IconChartDots3 size={18} />
          <span>Benchmarks</span>
        </button>
      </div>

      <div className="sidebar-spacer" />
      <button className="workspace-switcher" type="button">
        <span className="workspace-avatar"><IconBuilding size={17} /></span>
        <span><strong>Acme Corp</strong><small>Global production</small></span>
        <IconChevronDown size={16} />
      </button>
      <button className="nav-item subtle" type="button"><IconSettings size={18} /><span>Settings</span></button>
      <button className="profile" type="button">
        <span className="avatar">SG</span>
        <span><strong>Sarah Graham</strong><small>s.graham@acme.com</small></span>
        <IconChevronDown size={16} />
      </button>
    </aside>
  );
}

export function AppHeader({ title, subtitle, children, onSearch }) {
  return (
    <header className="app-header">
      <div className="app-title">
        <h1>{title}</h1>
        {subtitle && <p>{subtitle}</p>}
      </div>
      <div className="header-actions">
        {onSearch && (
          <label className="search-control">
            <IconSearch size={17} />
            <input placeholder="Search Acme Corp" onChange={(event) => onSearch(event.target.value)} />
          </label>
        )}
        {children}
      </div>
    </header>
  );
}

export function NewDiscoveryModal({ open, onClose, onCreated }) {
  const [step, setStep] = useState(1);
  const [selected, setSelected] = useState(["AWS Organizations", "GitHub Enterprise", "Google Cloud"]);
  const [name, setName] = useState("Global production refresh");
  const sources = ["AWS Organizations", "GitHub Enterprise", "Google Cloud", "Okta", "Kubernetes", "Snowflake", "Datadog", "PagerDuty"];

  useEffect(() => {
    if (open) setStep(1);
  }, [open]);

  const canContinue = useMemo(() => name.trim().length > 2 && selected.length > 0, [name, selected]);

  if (!open) return null;
  const toggleSource = (source) => setSelected((current) => (
    current.includes(source) ? current.filter((item) => item !== source) : [...current, source]
  ));

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <section className="modal" role="dialog" aria-modal="true" aria-labelledby="new-discovery-title" onMouseDown={(event) => event.stopPropagation()}>
        <header className="modal-header">
          <div>
            <div className="eyebrow">New discovery · Step {step} of 3</div>
            <h2 id="new-discovery-title">
              {step === 1 ? "Define the discovery" : step === 2 ? "Choose control planes" : "Review safety charter"}
            </h2>
          </div>
          <button className="icon-button" type="button" onClick={onClose} aria-label="Close"><IconX size={20} /></button>
        </header>

        {step === 1 && (
          <div className="modal-body form-stack">
            <label>
              Discovery name
              <input value={name} onChange={(event) => setName(event.target.value)} />
            </label>
            <label>
              Business objective
              <textarea defaultValue="Refresh the end-to-end Order to Cash system map and identify gaps that block safe simulation." rows={4} />
            </label>
            <div className="form-grid">
              <label>Environment<select defaultValue="global"><option value="global">Global production</option><option>EU production</option><option>Staging</option></select></label>
              <label>Maximum pass age<select defaultValue="24h"><option value="24h">24 hours</option><option>8 hours</option><option>7 days</option></select></label>
            </div>
          </div>
        )}

        {step === 2 && (
          <div className="modal-body">
            <p className="muted">Scout will use only enrolled machines and read-only, typed adapters. Credentials remain on their source hosts.</p>
            <div className="source-select-list">
              {sources.map((source) => (
                <label key={source} className="check-row">
                  <input type="checkbox" checked={selected.includes(source)} onChange={() => toggleSource(source)} />
                  <span>{source}</span>
                  <small>{source === "Datadog" ? "Scope needs review" : "Ready"}</small>
                </label>
              ))}
            </div>
          </div>
        )}

        {step === 3 && (
          <div className="modal-body">
            <div className="safety-banner">
              <IconShieldCheck size={22} />
              <div><strong>Read-only charter</strong><p>No production writes, interactive logins, tool installation, or secret payload retrieval.</p></div>
            </div>
            <dl className="review-list">
              <div><dt>Name</dt><dd>{name}</dd></div>
              <div><dt>Control planes</dt><dd>{selected.length} selected</dd></div>
              <div><dt>Execution</dt><dd>26 enrolled machines · bounded leases</dd></div>
              <div><dt>Completion</dt><dd>Two identical fixed-point passes</dd></div>
              <div><dt>Evidence</dt><dd>Signed, append-only, tenant scoped</dd></div>
            </dl>
          </div>
        )}

        <footer className="modal-footer">
          <button className="secondary-button" type="button" onClick={step === 1 ? onClose : () => setStep(step - 1)}>{step === 1 ? "Cancel" : "Back"}</button>
          <button
            className="primary"
            type="button"
            disabled={!canContinue}
            onClick={step === 3 ? () => onCreated(name) : () => setStep(step + 1)}
          >
            {step === 3 ? "Start discovery" : "Continue"}
          </button>
        </footer>
      </section>
    </div>
  );
}

export function Toast({ message, onDismiss }) {
  useEffect(() => {
    if (!message) return undefined;
    const timer = window.setTimeout(onDismiss, 3600);
    return () => window.clearTimeout(timer);
  }, [message, onDismiss]);

  if (!message) return null;
  return (
    <div className="toast" role="status">
      <IconShieldCheck size={18} />
      <span>{message}</span>
      <button type="button" onClick={onDismiss} aria-label="Dismiss notification"><IconX size={16} /></button>
    </div>
  );
}
