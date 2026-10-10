import type { ChainStep } from "../shared/types/preview";
import type {
  CompositeChain,
  CompositeChainBase,
  CompositeChainEntry,
  CompositeChainInput,
  CompositeDisplayed,
  CompositeStepMeta,
  DisplayStf,
} from "../shared/types/compositeChain";
import { CHAIN_ORDER, normalizeOutputPath } from "./processingChain";

export type { CompositeChain, CompositeChainBase, CompositeChainEntry, CompositeChainInput, CompositeDisplayed, CompositeStepMeta, DisplayStf };

export const COMPOSITE_RUN_KEY = "__composite__";

const STAGE: Record<ChainStep, number> = {
  background: 0,
  denoise: 1,
  deconv: 2,
  stretch: 3,
  maskedStretch: 3,
  localContrast: 4,
  tone: 5,
  pixelMath: 6,
};

const INPUT_LABELS: Record<ChainStep, string> = {
  background: "Background",
  denoise: "Denoise",
  deconv: "Deconvolution",
  stretch: "Stretch",
  maskedStretch: "Masked stretch",
  localContrast: "LHE / HDRMT",
  tone: "Curves",
  pixelMath: "PixelMath",
};

export const EMPTY_COMPOSITE_CHAIN: CompositeChain = Object.freeze({
  generation: null,
  base: null,
  steps: Object.freeze({}),
  psfKernel: null,
});

function previewKey(url: string): string {
  const end = url.search(/[?#]/);
  return normalizeOutputPath(end >= 0 ? url.slice(0, end) : url);
}

export function samePreview(a: string, b: string): boolean {
  return previewKey(a) === previewKey(b);
}

export function lastCompositeStep(chain: CompositeChain): ChainStep | null {
  for (let i = CHAIN_ORDER.length - 1; i >= 0; i--) {
    const s = CHAIN_ORDER[i];
    if (chain.steps[s]) return s;
  }
  return null;
}

export function compositeInputFor(chain: CompositeChain, step: ChainStep): CompositeChainInput {
  const stage = STAGE[step];
  for (let i = CHAIN_ORDER.length - 1; i >= 0; i--) {
    const s = CHAIN_ORDER[i];
    if (STAGE[s] >= stage) continue;
    if (chain.steps[s]) return s;
  }
  return "base";
}

export function compositeInputPreview(chain: CompositeChain, input: CompositeChainInput): string | null {
  if (input === "base") return chain.base?.previewUrl ?? null;
  return chain.steps[input]?.previewUrl ?? null;
}

export function compositeInputLabel(input: CompositeChainInput): string {
  return input === "base" ? "Composite" : INPUT_LABELS[input];
}

function capturedBase(result: CompositeStepMeta, callStf: DisplayStf): CompositeChainBase | null {
  return result.basePreviewUrl === undefined ? null : { previewUrl: result.basePreviewUrl, stf: callStf };
}

export function withCompositeStep(
  chain: CompositeChain,
  step: ChainStep,
  entry: CompositeChainEntry,
  result: CompositeStepMeta,
  callStf: DisplayStf,
): CompositeChain {
  const restart = result.chain_restarted || chain.base === null;
  const base = restart ? capturedBase(result, callStf) : chain.base;
  if (base === null) return chain;
  const stage = STAGE[step];
  const steps: Partial<Record<ChainStep, CompositeChainEntry>> = {};
  if (!restart) {
    for (const s of CHAIN_ORDER) {
      const existing = chain.steps[s];
      if (existing && STAGE[s] < stage) steps[s] = existing;
    }
  }
  steps[step] = entry;
  return { generation: result.chain_generation, base, steps, psfKernel: chain.psfKernel };
}

export function compositeChainHolds(chain: CompositeChain, step: ChainStep, previewUrl: string | null | undefined): boolean {
  if (!previewUrl) return true;
  const entry = chain.steps[step];
  return entry !== undefined && samePreview(entry.previewUrl, previewUrl);
}

export function withCompositePsfKernel(chain: CompositeChain, kernel: number[][], liveGeneration: number): CompositeChain {
  return { ...chain, psfKernel: kernel, generation: chain.base === null ? liveGeneration : chain.generation };
}

export function isCompositeChainStale(
  chain: CompositeChain,
  state: { chain_generation: number | null; live_generation: number },
): boolean {
  if (chain.base === null) return chain.generation !== state.live_generation;
  return (
    state.chain_generation === null ||
    state.chain_generation !== state.live_generation ||
    state.chain_generation !== chain.generation
  );
}

export function hasCompositeReset(chain: CompositeChain): boolean {
  return lastCompositeStep(chain) !== null || chain.psfKernel !== null;
}

export function compositeStepInvalidatesWizard(step: ChainStep, displayed: CompositeDisplayed): boolean {
  return displayed === "linear" || step === "stretch" || step === "maskedStretch";
}

export type CompositeResultTarget = "screen" | "parked" | "none";

export function compositeResultTarget(accepted: boolean, onScreen: boolean): CompositeResultTarget {
  if (!accepted) return "none";
  return onScreen ? "screen" : "parked";
}

export interface CompositeStepOutcome {
  target: CompositeResultTarget;
  invalidatesWizard: boolean;
}

export function compositeStepOutcome(input: {
  step: ChainStep;
  displayed: CompositeDisplayed;
  accepted: boolean;
  compositeMode: boolean;
}): CompositeStepOutcome {
  return {
    target: compositeResultTarget(input.accepted, input.compositeMode),
    invalidatesWizard: compositeStepInvalidatesWizard(input.step, input.displayed),
  };
}
