import type { StfParams } from "../shared/types";
import type { CompositeStfFields } from "../shared/types/compose";

export interface CompositeStfTriple {
  r: StfParams;
  g: StfParams;
  b: StfParams;
  linked: boolean;
}

export function stfTripleOf(response: CompositeStfFields | null | undefined): CompositeStfTriple | null {
  if (!response) return null;
  const { stf_r: r, stf_g: g, stf_b: b, auto_stf: auto, stf_linked: linked } = response;
  if (r && g && b) return { r, g, b, linked: linked ?? true };
  if (auto && linked !== false) return { r: auto, g: auto, b: auto, linked: true };
  return null;
}

export const WB_STRETCH_NOTE = "White balance needs a linked stretch, so the unlinked Blend stretch was replaced by a linked one.";

const IDENTITY_FACTOR_TOLERANCE = 1e-6;

export function wbStretchNote(linkedBefore: boolean, factors: readonly [number, number, number]): string | null {
  const identity = factors.every((f) => Math.abs(f - 1) <= IDENTITY_FACTOR_TOLERANCE);
  return !linkedBefore && !identity ? WB_STRETCH_NOTE : null;
}
