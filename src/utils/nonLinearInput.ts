import type { ChainStep, ProcessingChain } from "../shared/types/preview";
import type { CompositeChain, CompositeChainInput, CompositeDisplayed } from "../shared/types/compositeChain";
import { CHAIN_ORDER, stageOf } from "./processingChain";

export type NonLinearSection = "local_contrast" | "hdr" | "curves";

export type NonLinearInput =
  | { kind: "file"; upstream: readonly ChainStep[] }
  | { kind: "composite"; displayed: CompositeDisplayed | null };

const STRETCH_STAGE = stageOf("stretch");

const FILE_NOTES: Record<NonLinearSection, string> = {
  local_contrast: "No stretch step in this chain: LHE on linear data equalizes the noise floor; Stretch or Masked first gives the usual result.",
  hdr: "No stretch step in this chain: HDRMT refuses images whose median is under 1 % of the maximum; Stretch or Masked first.",
  curves: "No stretch step in this chain: if the input is linear, Curves applies an automatic STF first, then your levels and curve.",
};

const COMPOSITE_NOTES: Record<NonLinearSection, string | null> = {
  local_contrast: "The composite input is linear: LHE runs only on a stretched composite; Stretch or Masked first.",
  hdr: "The composite input is linear: HDRMT runs only on a stretched composite; Stretch or Masked first.",
  curves: null,
};

export function upstreamSteps(chain: ProcessingChain, step: ChainStep): ChainStep[] {
  const stage = stageOf(step);
  return CHAIN_ORDER.filter((s) => chain.steps[s] !== undefined && stageOf(s) < stage);
}

export function compositeInputDisplayed(chain: CompositeChain, input: CompositeChainInput): CompositeDisplayed | null {
  return input === "base" ? chain.base?.displayed ?? null : chain.steps[input]?.displayed ?? null;
}

export function nonLinearInputNote(section: NonLinearSection, input: NonLinearInput): string | null {
  if (input.kind === "composite") return input.displayed === "linear" ? COMPOSITE_NOTES[section] : null;
  return input.upstream.some((s) => stageOf(s) === STRETCH_STAGE) ? null : FILE_NOTES[section];
}

export function hdrLinearInputHint(compositeMode: boolean): string {
  const where = compositeMode ? "Stretch or Masked in this tab, or GHS in Compose › Stretch," : "Stretch (Arcsinh or GHS) or Masked in this tab,";
  return `HDRMT works on stretched data. Run ${where} then run HDRMT on its output.`;
}

export function curvesAutoStfNote(result: { input_normalized: { min: number; max: number } | null; curves_applied: boolean }): string | null {
  if (!result.input_normalized) return null;
  return result.curves_applied ? "Auto-STF applied before the curve" : "Auto-STF applied before the levels";
}
