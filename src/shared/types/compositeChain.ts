import type { StfParams } from "./fits.types";
import type { ChainStep } from "./preview";
import type { PixelMathStats } from "./pixelmath";
import type { ArcsinhResult, MaskedStretchResult } from "./processing";
import type { LocalContrastResult } from "./localContrast";

export type CompositeChainInput = "base" | ChainStep;

export type CompositeDisplayed = "linear" | "stretched" | "toned";

export interface DisplayStf {
  r: StfParams;
  g: StfParams;
  b: StfParams;
  linked: boolean;
}

export interface CompositeChainEntry {
  previewUrl: string;
  label: string;
  displayed: CompositeDisplayed;
  modelPreviewUrl?: string | null;
}

export interface CompositeChainBase {
  previewUrl: string;
  stf: DisplayStf;
}

export interface CompositeChain {
  generation: number | null;
  base: CompositeChainBase | null;
  steps: Partial<Record<ChainStep, CompositeChainEntry>>;
  psfKernel: number[][] | null;
}

export interface CompositeChainCall {
  chainInput: CompositeChainInput;
  displayStf: DisplayStf;
}

export type ChannelTriple = [number, number, number];

export interface CompositeStepMeta {
  png_path: string;
  base_png_path: string | null;
  dimensions: [number, number];
  elapsed_ms: number;
  chain_generation: number;
  chain_restarted: boolean;
  chain_input: string;
  displayed: CompositeDisplayed;
  stf: DisplayStf | null;
  previewUrl?: string;
  basePreviewUrl?: string;
}

export interface CompositeStepResult extends CompositeStepMeta {
  modelPreviewUrl?: string;
}

export interface CompositeBackgroundResult extends CompositeStepResult {
  model_png_path: string;
  sample_count: ChannelTriple;
  rms_residual: ChannelTriple;
  rejected_count: ChannelTriple | null;
}

export interface CompositeWaveletResult extends CompositeStepResult {
  scales_processed: number;
  noise_estimate: ChannelTriple;
}

export type PsfSource = "gaussian" | "estimated" | "provided";

export interface CompositeDeconvolveResult extends CompositeStepResult {
  iterations_run: ChannelTriple;
  convergence: ChannelTriple;
  psf_source: PsfSource;
}

export interface CompositePixelMathResult extends CompositeStepResult {
  stats: { r: PixelMathStats | null; g: PixelMathStats | null; b: PixelMathStats | null };
  non_finite_count: ChannelTriple;
  warnings: string[];
}

export type CompositeStretchResult = ArcsinhResult & CompositeStepResult;

export type CompositeMaskedStretchResult = MaskedStretchResult & CompositeStepResult;

export type CompositeLocalContrastResult = LocalContrastResult & CompositeStepResult;

export interface CompositeChainResetResult {
  restored: boolean;
  live_generation: number;
}

export interface CompositeChainStateResult {
  chain_generation: number | null;
  live_generation: number;
  steps: string[];
}
