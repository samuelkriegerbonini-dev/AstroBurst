import type { StfParams } from "../shared/types";
import type { CompositeStfTriple } from "./compositeStfResponse";

export type StretchCommitState = "not_applied" | "applied" | "changed" | "adjusted";

export const STRETCH_COMMIT_LINE: Record<StretchCommitState, string> = {
  not_applied: "Not applied: Adjust and Export use the composite STF",
  applied: "Applied (STF shown): Adjust and Export use this stretch",
  changed: "Changed since applied: press Apply stretch",
  adjusted: "Adjusted: Export uses the Adjust result",
};

export function sameStfParams(a: StfParams, b: StfParams): boolean {
  return a.shadow === b.shadow && a.midtone === b.midtone && a.highlight === b.highlight;
}

export function sameStfTriple(a: CompositeStfTriple, b: CompositeStfTriple): boolean {
  return a.linked === b.linked && sameStfParams(a.r, b.r) && sameStfParams(a.g, b.g) && sameStfParams(a.b, b.b);
}

export function hasStretchBake(state: { compositeReady: boolean; completedSteps: Record<string, boolean> }): boolean {
  return state.compositeReady && state.completedSteps.stretch === true;
}

export function stretchCommitState(
  committedStf: CompositeStfTriple | null,
  sliderStf: CompositeStfTriple,
  hasBake: boolean,
  adjustDone: boolean,
): StretchCommitState {
  if (committedStf === null || !sameStfTriple(committedStf, sliderStf)) return "changed";
  if (adjustDone) return "adjusted";
  return hasBake ? "applied" : "not_applied";
}

export function curvesInputLine(hasBake: boolean): string {
  return hasBake ? "Curves apply to: the applied stretch" : "Curves apply to: the composite STF";
}
