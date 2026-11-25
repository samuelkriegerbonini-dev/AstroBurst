import { createContext, useContext } from "react";
import { UNGUARDED_RUNS, type WizardRunGuard } from "../utils/wizardRunGuard";

export const WizardRunGuardCtx = createContext<WizardRunGuard>(UNGUARDED_RUNS);

export function useWizardRunGuard(): WizardRunGuard {
  return useContext(WizardRunGuardCtx);
}
