import {
  EMPTY_COMPOSITE_HISTORY,
  invalidateDownstream,
  resolveChannelPath,
  singleFilledBin,
  spccInputs,
  type PipelineStage,
  type WizardState,
} from "./wizard";

export const WIZARD_INPUTS_CHANGED = "Inputs changed while this step ran; run it again";

export const WIZARD_EXPORT_INPUTS_CHANGED =
  "Inputs changed while exporting; the saved file holds the inputs from before the change";

export type InputKey = (state: WizardState) => string;

export interface WizardRunGuard {
  begin: (inputs: InputKey) => () => boolean;
  beginComposite: () => () => boolean;
  discardLateComposite: () => void;
}

export const UNGUARDED_RUNS: WizardRunGuard = {
  begin: () => () => true,
  beginComposite: () => () => true,
  discardLateComposite: () => {},
};

export function channelInputKey(binId: string, upTo: PipelineStage): InputKey {
  return (state) => resolveChannelPath(state, binId, upTo) ?? "";
}

export function channelsInputKey(binIds: readonly string[], upTo: PipelineStage): InputKey {
  return (state) => `${state.alignRunToken ?? ""}|${binIds.map((id) => `${id}=${resolveChannelPath(state, id, upTo) ?? ""}`).join("|")}`;
}

export function blendInputKey(state: WizardState): string {
  return state.bins
    .filter((b) => b.files.length > 0)
    .map((b) => `${b.id}=${resolveChannelPath(state, b.id) ?? ""}`)
    .join("|");
}

export function cropInputKey(state: WizardState): string {
  const aligned = Object.entries(state.alignedPaths).filter(([, p]) => !!p);
  if (aligned.length > 0) return `${state.alignRunToken ?? ""}|${aligned.map(([id, p]) => `${id}=${p}`).join("|")}`;
  const single = singleFilledBin(state);
  return single ? `${single}=${resolveChannelPath(state, single, "stacked") ?? ""}` : "";
}

export function spccInputKey(state: WizardState): string {
  const inputs = spccInputs(state);
  return [inputs.r?.path, inputs.g?.path, inputs.b?.path].map((p) => p ?? "").join("|");
}

export function bumpsCompositeGeneration(before: WizardState, after: WizardState, actionType: string): boolean {
  return actionType === "RESET" || actionType === "SET_COMPOSITE_READY" || (before.compositeReady && !after.compositeReady);
}

export function lateCompositeDiscarded(state: WizardState): Partial<WizardState> {
  return {
    ...invalidateDownstream(state, "blend"),
    compositeReady: false,
    compositeHistory: EMPTY_COMPOSITE_HISTORY,
    blendLevelScales: null,
  };
}
