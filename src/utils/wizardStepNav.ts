import { STEPS, nextEnabledStep, type StepDef, type WizardState } from "./wizard";

export interface StepNavTarget {
  id: string;
  label: string;
  reason: string | null;
}

export interface WizardStepNav {
  previous: StepNavTarget | null;
  next: StepNavTarget | null;
}

function navTarget(step: StepDef, reason: string | null): StepNavTarget {
  return { id: step.id, label: step.shortLabel, reason };
}

function firstStepHeldBackLikeTheLast(later: readonly StepDef[], state: WizardState): StepNavTarget | null {
  const last = later[later.length - 1];
  if (!last) return null;
  const restOfWizardReason = last.blockedReason(state);
  const held = later.find((step) => step.blockedReason(state) === restOfWizardReason) ?? last;
  return navTarget(held, held.blockedReason(state));
}

export function wizardStepNav(currentId: string, state: WizardState): WizardStepNav {
  const at = STEPS.findIndex((step) => step.id === currentId);
  const previousStep = STEPS.slice(0, Math.max(at, 0)).reverse().find((step) => step.enabled(state));
  const nextId = nextEnabledStep(currentId, state);
  const nextStep = STEPS.find((step) => step.id === nextId);
  return {
    previous: previousStep ? navTarget(previousStep, null) : null,
    next: nextStep ? navTarget(nextStep, null) : firstStepHeldBackLikeTheLast(STEPS.slice(at + 1), state),
  };
}

export function stepNavLabel(direction: "Previous" | "Next", target: StepNavTarget): string {
  return target.reason ? `${direction}: ${target.label}, ${target.reason}` : `${direction}: ${target.label}`;
}
