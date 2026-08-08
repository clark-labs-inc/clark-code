import { useCallback, useEffect, useState } from "react";
import { useSessionStore } from "../store/sessionStore";
import { codeKeyAccountBinding, openExternal } from "./account";
import {
  capabilityAccess,
  productAccessSnapshot,
  type ProductAccessProjection,
} from "./productAccess";
import {
  gatedWorkflowForSubmission,
  type GatedWorkflow,
} from "./slashCommands";
import { productModule } from "../product/productModule";

export function gatedWorkflowNeedsAccessGate(
  workflow: GatedWorkflow | null,
  workflowAccessApproved: boolean,
  covered: boolean,
): boolean {
  return Boolean(workflow && !workflowAccessApproved && !covered);
}

export function useGatedWorkflowGate(sessionId: string | null) {
  const accountScope = useSessionStore((state) => codeKeyAccountBinding(state.auth));
  const [workflow, setWorkflow] = useState<GatedWorkflow | null>(null);
  const [running, setRunning] = useState(false);
  const [access, setAccess] = useState<ProductAccessProjection | null>(null);
  const [checkingAccess, setCheckingAccess] = useState(false);
  const workflowAccess = productModule().localAgent.workflowAccess;
  const accessCapability = capabilityAccess(access, workflowAccess?.capability ?? "");
  const covered = accessCapability?.availability === "available";

  useEffect(() => {
    setWorkflow(null);
    setRunning(false);
    setAccess(null);
    setCheckingAccess(false);
  }, [accountScope, sessionId]);

  const refreshAccess = useCallback(async () => {
    const requestScope = accountScope;
    setCheckingAccess(true);
    try {
      const next = await productAccessSnapshot();
      if (codeKeyAccountBinding(useSessionStore.getState().auth) === requestScope) {
        setAccess(next);
      }
    } finally {
      if (codeKeyAccountBinding(useSessionStore.getState().auth) === requestScope) {
        setCheckingAccess(false);
      }
    }
  }, [accountScope]);

  const shouldGate = useCallback((
    text: string,
    selectedSkillNames: string[],
    workflowAccessApproved: boolean,
  ) => {
    const nextWorkflow = gatedWorkflowForSubmission(text, selectedSkillNames);
    if (!gatedWorkflowNeedsAccessGate(nextWorkflow, workflowAccessApproved, covered)) {
      setWorkflow(null);
      return false;
    }
    setWorkflow(nextWorkflow);
    if (!checkingAccess) void refreshAccess().catch(() => undefined);
    return true;
  }, [checkingAccess, covered, refreshAccess]);

  const runWithAccess = useCallback(async (run: () => Promise<void>) => {
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
    accessCopy: workflowAccess,
    covered,
    checkingAccess,
    running,
    shouldGate,
    runWithAccess,
    viewAccess: () => {
      if (accessCapability?.actionUrl) void openExternal(accessCapability.actionUrl);
    },
    dismiss: () => setWorkflow(null),
  };
}
