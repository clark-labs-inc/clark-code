import type { useSubscriptionWorkflowGate } from "../lib/useSubscriptionWorkflowGate";
import { SubscriptionWorkflowGate } from "./SubscriptionWorkflowGate";

type SubscriptionAccess = ReturnType<typeof useSubscriptionWorkflowGate>;

export function ComposerSubscriptionWorkflowGate({
  access,
  onRun,
  onDismissed,
}: {
  access: SubscriptionAccess;
  onRun: () => Promise<void>;
  onDismissed: () => void;
}) {
  if (!access.workflow) return null;

  return (
    <SubscriptionWorkflowGate
      workflow={access.workflow}
      covered={access.covered}
      checkingCoverage={access.checkingCoverage}
      running={access.running}
      onRun={() => void access.runWithCoverage(onRun)}
      onViewPlans={access.viewPlans}
      onDismiss={() => {
        access.dismiss();
        requestAnimationFrame(onDismissed);
      }}
    />
  );
}
