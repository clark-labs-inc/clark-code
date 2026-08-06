import { useCallback, useEffect, useState } from "react";
import { useSessionStore } from "../store/sessionStore";
import {
  clarkBillingUrl,
  codeKeyAccountBinding,
  openExternal,
} from "./account";
import { projectClarkCodeBilling } from "./billing";
import {
  subscriptionWorkflowForSubmission,
  type SubscriptionWorkflow,
} from "./slashCommands";

export function subscriptionWorkflowNeedsCoverageGate(
  workflow: SubscriptionWorkflow | null,
  subscriptionApproved: boolean,
  covered: boolean,
): boolean {
  return Boolean(workflow && !subscriptionApproved && !covered);
}

export function useSubscriptionWorkflowGate(sessionId: string | null) {
  const accountScope = useSessionStore((state) => codeKeyAccountBinding(state.auth));
  const billing = useSessionStore((state) => state.billing);
  const loadingBilling = useSessionStore((state) => state.loadingBilling);
  const loadBilling = useSessionStore((state) => state.loadBilling);
  const [workflow, setWorkflow] = useState<SubscriptionWorkflow | null>(null);
  const [running, setRunning] = useState(false);
  const covered = projectClarkCodeBilling(billing).coverage.canRunSubscriberWorkflows;

  useEffect(() => {
    setWorkflow(null);
    setRunning(false);
  }, [accountScope, sessionId]);

  const shouldGate = useCallback((
    text: string,
    selectedSkillNames: string[],
    subscriptionApproved: boolean,
  ) => {
    const nextWorkflow = subscriptionWorkflowForSubmission(text, selectedSkillNames);
    if (!subscriptionWorkflowNeedsCoverageGate(nextWorkflow, subscriptionApproved, covered)) {
      setWorkflow(null);
      return false;
    }
    setWorkflow(nextWorkflow);
    if (!billing && !loadingBilling) void loadBilling();
    return true;
  }, [billing, covered, loadBilling, loadingBilling]);

  const runWithCoverage = useCallback(async (run: () => Promise<void>) => {
    if (!workflow || !covered) return;
    const requestScope = accountScope;
    setRunning(true);
    try {
      if (codeKeyAccountBinding(useSessionStore.getState().auth) !== requestScope) return;
      setWorkflow(null);
      await run();
    } finally {
      setRunning(false);
    }
  }, [accountScope, covered, workflow]);

  return {
    workflow,
    covered,
    checkingCoverage: loadingBilling,
    running,
    shouldGate,
    runWithCoverage,
    viewPlans: () => void openExternal(clarkBillingUrl()),
    dismiss: () => setWorkflow(null),
  };
}
