import {
  IconAlertCircle,
  IconAlertTriangle,
  IconCheck,
  IconChevronRight,
  IconClock,
  IconCopy,
  IconLoader2,
  IconMinus,
} from "@tabler/icons-react";

export function Status({ value, label }) {
  const normalized = String(value).toLowerCase();
  const icon = normalized === "verified" || normalized === "connected" || normalized === "active" || normalized === "ready" || normalized === "supported"
    ? <IconCheck size={13} />
    : normalized === "scanning" || normalized === "running"
      ? <IconLoader2 className="spin" size={13} />
      : normalized === "denied" || normalized === "unreachable" || normalized === "blocked"
        ? <IconAlertCircle size={13} />
        : normalized === "partial" || normalized === "attention" || normalized === "upgrade" || normalized === "gap"
          ? <IconAlertTriangle size={13} />
          : <IconMinus size={13} />;
  return <span className={`status status-${normalized}`}>{icon}{label || value}</span>;
}

export function Progress({ value, tone = "green", label }) {
  return (
    <div className="progress-wrap" aria-label={label || `${value}% complete`}>
      <div className="progress-track">
        <span className={`progress-fill progress-${tone}`} style={{ width: `${Math.max(0, Math.min(100, value))}%` }} />
      </div>
      {label && <span className="progress-label">{label}</span>}
    </div>
  );
}

export function Panel({ title, eyebrow, action, className = "", children }) {
  return (
    <section className={`panel ${className}`}>
      {(title || eyebrow || action) && (
        <header className="panel-header">
          <div>
            {eyebrow && <div className="eyebrow">{eyebrow}</div>}
            {title && <h2>{title}</h2>}
          </div>
          {action}
        </header>
      )}
      {children}
    </section>
  );
}

export function Metric({ label, value, meta, tone }) {
  return (
    <div className={`metric ${tone ? `metric-${tone}` : ""}`}>
      <strong>{value}</strong>
      <span>{label}</span>
      {meta && <small>{meta}</small>}
    </div>
  );
}

export function SourceMark({ name, size = "md" }) {
  const initials = name
    .replace(/\([^)]*\)/g, "")
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0])
    .join("")
    .toUpperCase();
  const palette = ["violet", "blue", "green", "orange", "slate"];
  const index = [...name].reduce((sum, char) => sum + char.charCodeAt(0), 0) % palette.length;
  return <span className={`source-mark source-${palette[index]} source-${size}`}>{initials}</span>;
}

export function Segmented({ options, value, onChange, label = "View" }) {
  return (
    <div className="segmented" role="tablist" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={value === option.value ? "active" : ""}
          onClick={() => onChange(option.value)}
          role="tab"
          aria-selected={value === option.value}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function CopyValue({ children }) {
  return (
    <span className="copy-value">
      <span>{children}</span>
      <button type="button" aria-label="Copy value"><IconCopy size={14} /></button>
    </span>
  );
}

export function KeyValue({ label, children }) {
  return (
    <div className="key-value">
      <span>{label}</span>
      <strong>{children}</strong>
    </div>
  );
}

export function EmptyState({ title, body, action }) {
  return (
    <div className="empty-state">
      <IconClock size={28} />
      <h3>{title}</h3>
      <p>{body}</p>
      {action}
    </div>
  );
}

export function RowLink({ children }) {
  return <button className="row-link" type="button">{children}<IconChevronRight size={15} /></button>;
}
