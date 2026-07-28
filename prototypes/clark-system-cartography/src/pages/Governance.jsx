import { useState } from "react";
import {
  IconBuilding,
  IconCheck,
  IconFileDescription,
  IconHistory,
  IconLock,
  IconPlus,
  IconShieldCheck,
  IconUsers,
} from "@tabler/icons-react";
import { auditEvents } from "../data";
import { AppHeader } from "../components/Layout";
import { Panel, Segmented, Status } from "../components/Ui";

const policies = [
  ["Public", "Public metadata approved for customer-facing documentation", "Everyone in workspace"],
  ["Internal", "Default for system identities, relationships, and operational metadata", "All active members"],
  ["Confidential", "Runtime samples, business-sensitive topology, and owner assignments", "Architects, SRE, Security"],
  ["Restricted", "Cloud identity observations and sensitive control-plane metadata", "Security admins"],
  ["Secret reference only", "Names and references only; payload retrieval is prohibited", "Security admins"],
];

export function Governance({ onNotice }) {
  const [tab, setTab] = useState("charter");
  return (
    <div className="page governance-page">
      <AppHeader title="Governance" subtitle="Control who can discover, inspect, adjudicate, and simulate Acme Corp.">
        <button className="primary" type="button" onClick={() => onNotice("Draft charter v13 created from the current verified scope.")}><IconPlus size={18} /> New charter</button>
      </AppHeader>

      <div className="subnav">
        <Segmented value={tab} onChange={setTab} options={[{ value: "charter", label: "Discovery charter" }, { value: "classification", label: "Classification" }, { value: "access", label: "Access" }, { value: "audit", label: "Audit log" }]} />
        <Status value="verified" label="Policy enforced" />
      </div>

      {tab === "charter" && (
        <div className="governance-layout">
          <Panel title="Global production charter" eyebrow="Version 12 · Active" className="charter-panel">
            <div className="charter-summary">
              <div><IconBuilding size={22} /><span><strong>Organization</strong><small>Acme Corp</small></span></div>
              <div><IconFileDescription size={22} /><span><strong>Objective</strong><small>Map end-to-end business journeys and prepare safe simulations.</small></span></div>
              <div><IconShieldCheck size={22} /><span><strong>Safety ceiling</strong><small>Read-only production; no secret payloads; no interactive login.</small></span></div>
              <div><IconHistory size={22} /><span><strong>Completion</strong><small>Two identical fixed-point passes; maximum pass age 24 hours.</small></span></div>
            </div>
            <div className="charter-section">
              <h3>Authoritative seeds</h3>
              <div className="seed-grid">
                {["GitHub Enterprise · acme-corp", "AWS Organizations · o-a1b2c3d4", "Google Cloud · organizations/81720341", "Okta · acme.okta.com", "Route 53 · 48 zones", "Service catalog · catalog.acme.internal", "Datadog · acme.datadoghq.com", "PagerDuty · acme.pagerduty.com"].map((seed) => <span key={seed}><IconCheck size={15} />{seed}</span>)}
              </div>
            </div>
            <div className="charter-section">
              <h3>Required business journeys</h3>
              <div className="tag-list">{["Order to Cash", "Procure to Pay", "Record to Report", "Hire to Retire", "Service to Support", "Design to Release", "Source to Contract", "IT Operations", "Data to Insight"].map((item) => <span key={item}>{item}</span>)}</div>
            </div>
            <div className="charter-section">
              <h3>Explicit exclusions</h3>
              <p>Sandbox accounts, personal developer projects, archived repositories older than seven years, and customer-owned SaaS tenants.</p>
            </div>
          </Panel>
          <Panel title="Charter history" eyebrow="Append-only">
            <div className="version-list">
              {[["v12", "Active", "Jul 26, 2026", "Added EU production and carrier ownership requirements"], ["v11", "Superseded", "Jul 18, 2026", "Expanded Order to Cash simulation contracts"], ["v10", "Superseded", "Jul 02, 2026", "Added Google Cloud organization hierarchy"], ["v9", "Superseded", "Jun 11, 2026", "Established global production baseline"]].map(([version, state, date, body]) => <button type="button" key={version}><span className="version-badge">{version}</span><span><strong>{date}</strong><small>{body}</small></span><Status value={state === "Active" ? "active" : "superseded"} label={state} /></button>)}
            </div>
          </Panel>
        </div>
      )}

      {tab === "classification" && (
        <Panel title="Information classification" eyebrow="Monotone by default">
          <p className="panel-intro">New evidence can raise an object’s classification but cannot silently lower it. Secret payloads and do-not-store data are rejected before event construction.</p>
          <div className="policy-table">
            <div className="table-head"><span>Level</span><span>Policy</span><span>Who can view</span><span>Storage</span></div>
            {policies.map(([level, policy, access]) => <div className="table-row" key={level}><span><span className={`classification classification-${level.toLowerCase().replaceAll(" ", "-")}`}>{level}</span></span><span>{policy}</span><span>{access}</span><span><Status value="verified" label="Encrypted" /></span></div>)}
          </div>
        </Panel>
      )}

      {tab === "access" && (
        <div className="governance-layout">
          <Panel title="Organization access" eyebrow="Exact membership">
            <div className="privacy-callout"><IconLock size={22} /><div><strong>Email domains never grant access</strong><p>Users with @gmail.com or any other shared domain remain isolated unless explicitly added to this organization and workspace.</p></div></div>
            <div className="member-list">
              {[["Sarah Graham", "Owner", "s.graham@acme.com"], ["Alex Morgan", "Enterprise architect", "alex.morgan@acme.com"], ["Priya Shah", "Security admin", "priya.shah@acme.com"], ["Data Platform", "Viewer group", "42 members"]].map(([name, role, detail]) => <div key={name}><span className="avatar">{name.split(" ").map((part) => part[0]).join("").slice(0, 2)}</span><span><strong>{name}</strong><small>{detail}</small></span><em>{role}</em></div>)}
            </div>
          </Panel>
          <Panel title="Role capabilities" eyebrow="Least privilege">
            <div className="role-list">
              {["Owner · all governance and workspace controls", "Admin · enroll machines and manage sources", "Architect · inspect graph, evidence, and simulations", "Investigator · create and adjudicate assigned claims", "Viewer · read qualified objects within clearance"].map((role) => <div key={role}><IconUsers size={17} /><span>{role}</span></div>)}
            </div>
          </Panel>
        </div>
      )}

      {tab === "audit" && (
        <Panel title="Audit events" eyebrow="Organization-scoped">
          <div className="audit-table">
            <div className="table-head"><span>Event</span><span>Actor</span><span>Details</span><span>Time</span><span>Receipt</span></div>
            {auditEvents.map((event, index) => <div className="table-row" key={event.action}><span className="strong-cell"><IconHistory size={16} />{event.action}</span><span>{event.actor}</span><span>{event.detail}</span><span>{event.time}</span><span className="mono">audit:{String(index + 1).padStart(4, "0")}…</span></div>)}
          </div>
        </Panel>
      )}
    </div>
  );
}
