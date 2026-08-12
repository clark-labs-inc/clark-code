import { useEffect, useState } from "react";
import {
  ArrowDownRight,
  ArrowLeft,
  ArrowRight,
  Check,
  ExternalLink,
  MousePointer2,
} from "lucide-react";
import { cn } from "../../lib/cn";

export type MacPermissionGuideProps = {
  ownerName: string;
  accessibilityGranted: boolean;
  screenRecordingGranted: boolean;
  working: boolean;
  onRequestPermissions: () => void;
};

type PermissionStep = "accessibility" | "screen-recording";

const STEPS: Array<{
  id: PermissionStep;
  title: string;
  setting: string;
  description: string;
  detail: string;
}> = [
  {
    id: "accessibility",
    title: "Allow Accessibility",
    setting: "Accessibility",
    description: "Let the agent read the controls and safely target the window it is working in.",
    detail: "Find the agent Computer Use service, then turn it on.",
  },
  {
    id: "screen-recording",
    title: "Allow Screen Recording",
    setting: "Screen Recording",
    description: "Let the agent see the current app window and show you what it is doing.",
    detail: "Find the same service, then turn it on and restart the agent if macOS asks.",
  },
];

export function initialMacPermissionStep(
  accessibilityGranted: boolean,
  screenRecordingGranted: boolean,
): PermissionStep {
  return accessibilityGranted && !screenRecordingGranted
    ? "screen-recording"
    : "accessibility";
}

function SettingRow({
  label,
  active,
  enabled,
}: {
  label: string;
  active?: boolean;
  enabled?: boolean;
}) {
  return (
    <div className={cn(
      "flex items-center gap-2 rounded-md px-2 py-1.5 text-xs",
      active ? "bg-accent/12 font-semibold text-accent" : "text-ink-muted",
    )}>
      <span className={cn("size-2 rounded-full", active ? "bg-accent" : "bg-ink-faint/50")} />
      <span className="truncate">{label}</span>
      {enabled && <Check className="ml-auto size-3 text-success" />}
    </div>
  );
}

function SettingsPreview({
  step,
  ownerName,
  accessibilityGranted,
  screenRecordingGranted,
}: {
  step: (typeof STEPS)[number];
  ownerName: string;
  accessibilityGranted: boolean;
  screenRecordingGranted: boolean;
}) {
  const accessibility = step.id === "accessibility";
  const granted = accessibility ? accessibilityGranted : screenRecordingGranted;

  return (
    <div className="permission-guide-preview relative overflow-hidden rounded-xl border border-border-subtle bg-bg-sunken p-3 shadow-soft">
      <div className="mx-auto max-w-[25rem] overflow-hidden rounded-lg border border-border bg-bg-secondary shadow-lifted">
        <div className="flex h-6 items-center gap-1.5 border-b border-border-subtle bg-bg-tertiary px-2.5">
          <span className="size-1.5 rounded-full bg-danger/70" />
          <span className="size-1.5 rounded-full bg-warning/70" />
          <span className="size-1.5 rounded-full bg-success/70" />
          <span className="ml-2 text-xs font-medium text-ink-muted">System Settings</span>
        </div>
        <div className="flex min-h-[10.5rem]">
          <div className="w-[38%] border-r border-border-subtle bg-bg-tertiary/55 p-2">
            <div className="mb-2 h-3 w-16 rounded bg-ink-faint/20" />
            <SettingRow label="General" />
            <SettingRow label="Privacy & Security" active />
            <div className="ml-3 mt-1 space-y-0.5 border-l border-border-subtle pl-2">
              <SettingRow label="Accessibility" active={accessibility} enabled={accessibilityGranted} />
              <SettingRow label="Screen Recording" active={!accessibility} enabled={screenRecordingGranted} />
            </div>
          </div>
          <div className="relative flex-1 p-3">
            <div className="text-xs font-semibold text-ink">Privacy &amp; Security</div>
            <div className="mt-0.5 text-xs text-ink-faint">{step.setting}</div>
            <div className="mt-2 rounded-md border border-border-subtle bg-bg-tertiary/55 p-2">
              <div className="text-xs text-ink-faint">Allow these applications to control your Mac:</div>
              <div className={cn(
                "permission-guide-target relative mt-2 flex items-center gap-2 rounded-md border px-2 py-1.5",
                granted ? "border-success/35 bg-success/8" : "border-accent/45 bg-accent/8",
              )}>
                <div className="grid size-5 place-items-center rounded bg-accent text-xs font-bold text-on-accent">C</div>
                <span className="min-w-0 flex-1 truncate text-xs font-medium text-ink">{ownerName}</span>
                <span className={cn(
                  "relative h-3.5 w-6 rounded-full",
                  granted ? "bg-success" : "bg-ink-faint/40",
                )}>
                  <span className={cn(
                    "absolute top-0.5 size-2.5 rounded-full bg-white shadow-sm transition-transform",
                    granted ? "translate-x-3" : "left-0.5",
                  )} />
                </span>
                {!granted && (
                  <>
                    <ArrowDownRight className="permission-guide-arrow absolute -bottom-7 right-2 size-6 text-accent" />
                    <span className="permission-guide-cursor absolute -bottom-8 right-0.5 text-accent">
                      <MousePointer2 className="size-4 fill-accent/20" />
                    </span>
                  </>
                )}
              </div>
            </div>
            <div className="mt-2 h-1.5 w-32 rounded bg-ink-faint/15" />
            <div className="mt-1 h-1.5 w-24 rounded bg-ink-faint/15" />
          </div>
        </div>
      </div>
      <div className="mt-2 flex items-center justify-center gap-1.5 text-xs text-ink-faint">
        <ExternalLink className="size-3" />
        <span>Privacy &amp; Security</span>
        <ArrowRight className="size-3 text-accent" />
        <span className="font-medium text-ink-muted">{step.setting}</span>
      </div>
    </div>
  );
}

export function MacPermissionGuide({
  ownerName,
  accessibilityGranted,
  screenRecordingGranted,
  working,
  onRequestPermissions,
}: MacPermissionGuideProps) {
  const [stepId, setStepId] = useState<PermissionStep>(() => (
    initialMacPermissionStep(accessibilityGranted, screenRecordingGranted)
  ));
  const stepIndex = STEPS.findIndex((step) => step.id === stepId);
  const step = STEPS[stepIndex] ?? STEPS[0];

  useEffect(() => {
    if (accessibilityGranted && !screenRecordingGranted && stepId === "accessibility") {
      setStepId("screen-recording");
    }
  }, [accessibilityGranted, screenRecordingGranted, stepId]);

  return (
    <div className="permission-guide overflow-hidden rounded-2xl border border-accent/20 bg-accent/5 p-3.5 sm:p-4" data-testid="mac-permission-guide">
      <div className="flex items-start gap-3">
        <div className="grid size-8 shrink-0 place-items-center rounded-xl bg-accent text-on-accent shadow-sm">
          <MousePointer2 className="size-4" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-ink">Let’s set up the agent on your Mac</div>
          <p className="mt-0.5 text-xs leading-5 text-ink-muted">
            Follow the two steps below. the agent only uses these grants to observe the app it is controlling; it does not give you a takeover button.
          </p>
        </div>
        <div className="shrink-0 rounded-full bg-bg-secondary px-2 py-1 text-xs font-semibold tabular-nums text-ink-muted">
          {stepIndex + 1} / {STEPS.length}
        </div>
      </div>

      <div className="mt-3 flex gap-1.5" aria-label="Permission setup progress">
        {STEPS.map((candidate, index) => {
          const complete = candidate.id === "accessibility" ? accessibilityGranted : screenRecordingGranted;
          return (
            <button
              key={candidate.id}
              type="button"
              aria-label={`Show ${candidate.title}`}
              aria-current={candidate.id === stepId ? "step" : undefined}
              onClick={() => setStepId(candidate.id)}
              className={cn(
                "group flex min-w-0 flex-1 items-center gap-1.5 rounded-lg px-2 py-1.5 text-left transition",
                candidate.id === stepId ? "bg-bg-secondary text-ink shadow-sm" : "text-ink-faint hover:bg-bg-secondary/60 hover:text-ink-muted",
              )}
            >
              <span className={cn(
                "grid size-5 shrink-0 place-items-center rounded-full border text-xs font-semibold",
                complete ? "border-success/40 bg-success/10 text-success" : candidate.id === stepId ? "border-accent/40 bg-accent/10 text-accent" : "border-border text-ink-faint",
              )}>
                {complete ? <Check className="size-3" /> : index + 1}
              </span>
              <span className="truncate text-xs font-medium">{candidate.setting}</span>
            </button>
          );
        })}
      </div>

      <div className="mt-3 rounded-xl bg-bg-secondary/80 p-2.5">
        <div key={step.id} className="permission-guide-step">
          <SettingsPreview
            step={step}
            ownerName={ownerName}
            accessibilityGranted={accessibilityGranted}
            screenRecordingGranted={screenRecordingGranted}
          />
          <div className="mt-3 px-0.5">
            <div className="text-sm font-semibold text-ink">{step.title}</div>
            <p className="mt-1 text-xs leading-5 text-ink-muted">{step.description}</p>
            <p className="mt-1 text-xs leading-5 text-ink-secondary">{step.detail}</p>
          </div>
        </div>
      </div>

      <div className="mt-3 flex flex-wrap items-center justify-between gap-2">
        <button
          type="button"
          onClick={() => setStepId(STEPS[Math.max(0, stepIndex - 1)].id)}
          disabled={stepIndex === 0}
          className="inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium text-ink-muted transition hover:bg-bg-secondary hover:text-ink disabled:pointer-events-none disabled:opacity-35"
        >
          <ArrowLeft className="size-3.5" />
          Back
        </button>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <button
            type="button"
            onClick={onRequestPermissions}
            disabled={working}
            className="inline-flex items-center gap-1.5 rounded-lg border border-accent/30 bg-bg-secondary px-2.5 py-1.5 text-xs font-semibold text-accent transition hover:bg-accent/10 disabled:opacity-50"
          >
            <ExternalLink className="size-3.5" />
            {working ? "Opening…" : "Open macOS settings"}
          </button>
          <button
            type="button"
            onClick={() => setStepId(STEPS[Math.min(STEPS.length - 1, stepIndex + 1)].id)}
            disabled={stepIndex === STEPS.length - 1}
            className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1.5 text-xs font-semibold text-on-accent transition hover:bg-accent-hover disabled:pointer-events-none disabled:opacity-35"
          >
            Next step
            <ArrowRight className="size-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
}
