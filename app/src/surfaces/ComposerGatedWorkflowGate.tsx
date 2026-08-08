import type { useGatedWorkflowGate } from "../lib/useGatedWorkflowGate";
import { GatedWorkflowGate } from "./GatedWorkflowGate";

type GatedAccess = ReturnType<typeof useGatedWorkflowGate>;

export function ComposerGatedWorkflowGate({
  access,
  onRun,
  onDismissed,
}: {
  access: GatedAccess;
  onRun: () => Promise<void>;
  onDismissed: () => void;
}) {
  if (!access.workflow) return null;

  return (
    <GatedWorkflowGate
      workflow={access.workflow}
      accessCopy={access.accessCopy}
      covered={access.covered}
      checkingAccess={access.checkingAccess}
      running={access.running}
      onRun={() => void access.runWithAccess(onRun)}
      onViewAccess={access.viewAccess}
      onDismiss={() => {
        access.dismiss();
        requestAnimationFrame(onDismissed);
      }}
    />
  );
}
