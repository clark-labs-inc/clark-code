import {
  SPECIALIST_MODEL_ID,
  SPECIALIST_REASONING_EFFORT,
} from "./localAgent";
import type { SpecialistContext } from "./specialists";

export interface SpecialistModelSettings {
  model: string;
  reasoningEffort: string;
}

export function specialistModelSettings(
  context: SpecialistContext | null | undefined,
): SpecialistModelSettings | null {
  if (!context) return null;
  return {
    model: SPECIALIST_MODEL_ID,
    reasoningEffort: SPECIALIST_REASONING_EFFORT,
  };
}
