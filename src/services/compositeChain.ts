import { typedInvoke, withPreview } from "../infrastructure/tauri";
import type { DbeConfig } from "../shared/types/dbe";
import type { MaskChoice } from "../shared/types/masks";
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
import { compositeChainResets } from "../utils/compositeChainResets";
import { BACKGROUND_DEFAULTS, DECONV_DEFAULTS, WAVELET_DEFAULTS } from "./processingDefaults";

export const CHAIN_PREVIEWS: [string, string][] = [
  ["png_path", "previewUrl"],
  ["base_png_path", "basePreviewUrl"],
];

const BACKGROUND_PREVIEWS: [string, string][] = [...CHAIN_PREVIEWS, ["model_png_path", "modelPreviewUrl"]];

export function chainArgs(chain: CompositeChainCall): { chainInput: CompositeChainInput; displayStf: CompositeChainCall["displayStf"]; mask: MaskChoice | null } {
  return { chainInput: chain.chainInput, displayStf: chain.displayStf, mask: chain.mask ?? null };
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
    gridSize: options.gridSize ?? BACKGROUND_DEFAULTS.gridSize,
    polyDegree: options.polyDegree ?? BACKGROUND_DEFAULTS.polyDegree,
    sigmaClip: options.sigmaClip ?? BACKGROUND_DEFAULTS.sigmaClip,
    iterations: options.iterations ?? BACKGROUND_DEFAULTS.iterations,
    mode: options.mode ?? BACKGROUND_DEFAULTS.mode,
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
    numScales: options.numScales ?? WAVELET_DEFAULTS.numScales,
    thresholds: options.thresholds ?? [...WAVELET_DEFAULTS.thresholds],
    linear: options.linear ?? WAVELET_DEFAULTS.linear,
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
    iterations: options.iterations ?? DECONV_DEFAULTS.iterations,
    psfSigma: options.psfSigma ?? DECONV_DEFAULTS.psfSigma,
    psfSize: options.psfSize ?? DECONV_DEFAULTS.psfSize,
    regularization: options.regularization ?? DECONV_DEFAULTS.regularization,
    deringing: options.deringing ?? DECONV_DEFAULTS.deringing,
    deringThreshold: options.deringThreshold ?? DECONV_DEFAULTS.deringThreshold,
    useEmpiricalPsf: options.useEmpiricalPsf ?? DECONV_DEFAULTS.useEmpiricalPsf,
    psfNumStars: options.psfNumStars ?? DECONV_DEFAULTS.psfNumStars,
    psfCutoutRadius: options.psfCutoutRadius ?? DECONV_DEFAULTS.psfCutoutRadius,
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

export async function compositeChainReset(): Promise<CompositeChainResetResult> {
  const res = await typedInvoke<CompositeChainResetResult>("composite_chain_reset_cmd", {});
  compositeChainResets.notify(res.restored);
  return res;
}

export function compositeChainState(): Promise<CompositeChainStateResult> {
  return typedInvoke<CompositeChainStateResult>("composite_chain_state_cmd", {});
}
