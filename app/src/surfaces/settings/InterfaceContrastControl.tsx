import { cn } from "../../lib/cn";
import {
  INTERFACE_CONTRASTS,
  type InterfaceContrast,
} from "../../lib/useAppearance";
import { Row } from "./Primitives";

const LABELS: Record<InterfaceContrast, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  "extra-high": "Extra high",
};

export function InterfaceContrastControl({
  value: selectedValue,
  onChange,
}: {
  value: InterfaceContrast;
  onChange: (contrast: InterfaceContrast) => void;
}) {
  return (
    <Row name="Overall contrast" sub="Text and controls across the whole interface">
      <div
        role="group"
        aria-label="Overall contrast"
        className="flex max-w-full flex-wrap justify-end rounded-lg bg-bg-sunken p-0.5 text-xs"
      >
        {INTERFACE_CONTRASTS.map((option) => (
          <button
            key={option}
            type="button"
            aria-pressed={selectedValue === option}
            onClick={() => onChange(option)}
            className={cn(
              "rounded-md px-2.5 py-1 transition",
              selectedValue === option
                ? "bg-bg-elevated text-ink shadow-sm"
                : "text-ink-muted hover:text-ink-secondary",
            )}
          >
            {LABELS[option]}
          </button>
        ))}
      </div>
    </Row>
  );
}
