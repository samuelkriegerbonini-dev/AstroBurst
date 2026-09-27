import { typedInvoke, withPreview } from "../infrastructure/tauri";
import type { DbeConfig } from "../shared/types/dbe";
import type { PixelMathSlot } from "../shared/types/pixelmath";
import type { PsfEstimate } from "../shared/types/processing";
import type {
  CompositeBackgroundResult,
  CompositeChainCall,
  CompositeChainInput,
  CompositeChainResetResult,
  CompositeChainStateResult,
  CompositeDeconvolveResult,
  CompositePixelMathResult,
  CompositeWaveletResult,
} from "../shared/types/compositeChain";

export const CHAIN_PREVIEWS: [string, string][] = [
  ["png_path", "previewUrl"],
  ["base_png_path", "basePreviewUrl"],
];

const BACKGROUND_PREVIEWS: [string, string][] = [...CHAIN_PREVIEWS, ["model_png_path", "modelPreviewUrl"]];

export function chainArgs(chain: CompositeChainCall): { chainInput: CompositeChainInput; displayStf: CompositeChainCall["displayStf"] } {
  return { chainInput: chain.chainInput, displayStf: chain.displayStf };
}

export type CompositeBackgroundModel = "polynomial" | "spline";

export function compositeBackground(
  outputDir: string | undefined,
  chain: CompositeChainCall,
  options: {
    model: CompositeBackgroundModel;
    gridSize?: number;
    polyDegree?: number;
    sigmaClip?: number;
    iterations?: number;
    mode?: string;
    dbe?: DbeConfig | null;
  },
): Promise<CompositeBackgroundResult> {
  return withPreview<CompositeBackgroundResult>("composite_background_cmd", outputDir, {
    ...chainArgs(chain),
    model: options.model,
    gridSize: options.gridSize ?? 8,
    polyDegree: options.polyDegree ?? 3,
    sigmaClip: options.sigmaClip ?? 2.5,
    iterations: options.iterations ?? 3,
    mode: options.mode ?? "subtract",
    dbe: options.dbe ?? null,
  }, BACKGROUND_PREVIEWS);
}

export function compositeWaveletDenoise(
  outputDir: string | undefined,
  chain: CompositeChainCall,
  options: {
    numScales?: number;
    thresholds?: number[];
    linear?: boolean;
    layerBias?: number[];
  } = {},
): Promise<CompositeWaveletResult> {
  return withPreview<CompositeWaveletResult>("composite_wavelet_denoise_cmd", outputDir, {
    ...chainArgs(chain),
    numScales: options.numScales ?? 5,
    thresholds: options.thresholds ?? [3.0, 2.5, 2.0, 1.5, 1.0],
    linear: options.linear ?? true,
    layerBias: options.layerBias ?? null,
  }, CHAIN_PREVIEWS);
}

export function compositeDeconvolveRL(
  outputDir: string | undefined,
  chain: CompositeChainCall,
  options: {
    iterations?: number;
    psfSigma?: number;
    psfSize?: number;
    regularization?: number;
    deringing?: boolean;
    deringThreshold?: number;
    useEmpiricalPsf?: boolean;
    psfNumStars?: number;
    psfCutoutRadius?: number;
    psfKernel?: number[][] | null;
  } = {},
): Promise<CompositeDeconvolveResult> {
  return withPreview<CompositeDeconvolveResult>("composite_deconvolve_rl_cmd", outputDir, {
    ...chainArgs(chain),
    iterations: options.iterations ?? 20,
    psfSigma: options.psfSigma ?? 2.0,
    psfSize: options.psfSize ?? 15,
    regularization: options.regularization ?? 0.001,
    deringing: options.deringing ?? true,
    deringThreshold: options.deringThreshold ?? 0.1,
    useEmpiricalPsf: options.useEmpiricalPsf ?? false,
    psfNumStars: options.psfNumStars ?? 30,
    psfCutoutRadius: options.psfCutoutRadius ?? 15,
    psfKernel: options.psfKernel ?? null,
  }, CHAIN_PREVIEWS);
}

export function compositeEstimatePsf(
  chainInput: CompositeChainInput,
  options: {
    numStars?: number;
    cutoutRadius?: number;
    saturationThreshold?: number;
    maxEllipticity?: number;
  } = {},
): Promise<PsfEstimate> {
  return typedInvoke<PsfEstimate>("composite_estimate_psf_cmd", {
    chainInput,
    numStars: options.numStars ?? 30,
    cutoutRadius: options.cutoutRadius ?? 15,
    saturationThreshold: options.saturationThreshold ?? 0.95,
    maxEllipticity: options.maxEllipticity ?? 0.3,
  });
}

export function compositePixelMath(
  outputDir: string | undefined,
  chain: CompositeChainCall,
  expression: string,
  options: { slots?: PixelMathSlot[]; truncate?: boolean; rescale?: boolean } = {},
): Promise<CompositePixelMathResult> {
  return withPreview<CompositePixelMathResult>("pixelmath_composite_cmd", outputDir, {
    ...chainArgs(chain),
    expression,
    slots: (options.slots ?? []).map((s) => ({ name: s.name.trim(), path: s.path })),
    truncate: options.truncate ?? false,
    rescale: options.rescale ?? false,
  }, CHAIN_PREVIEWS);
}

export function compositeChainReset(): Promise<CompositeChainResetResult> {
  return typedInvoke<CompositeChainResetResult>("composite_chain_reset_cmd", {});
}

export function compositeChainState(): Promise<CompositeChainStateResult> {
  return typedInvoke<CompositeChainStateResult>("composite_chain_state_cmd", {});
}
