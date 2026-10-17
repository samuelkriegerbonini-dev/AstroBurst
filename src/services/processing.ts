import { typedInvoke, withPreview, getOutputDir } from "../infrastructure/tauri";
import { announceCommandOutputs } from "../infrastructure/tauri/outputEvents";
import type {
  DeconvolveResult,
  BackgroundResult,
  WaveletResult,
  PsfEstimate,
  ArcsinhResult,
  MaskedStretchResult,
  SpccResult,
} from "../shared/types/processing";
import type {
  CompositeChainCall,
  CompositeMaskedStretchResult,
  CompositeStretchResult,
  PsfSource,
} from "../shared/types/compositeChain";
import type { CompositeStfFields } from "../shared/types/compose";
import { CHAIN_PREVIEWS, chainArgs } from "./compositeChain";
import { recordOp } from "./ops";
import type { HistoryScope, OpSpec, PsfSpec, RunOpResult } from "../shared/types/ops";
import type { DbeConfig } from "../shared/types/dbe";

export interface DeconvolveRunResult extends DeconvolveResult {
  psf_source: PsfSource;
}

export interface DeconvolveOptions {
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
}

export interface DeconvolveSettings {
  iterations: number;
  psfSigma: number;
  psfSize: number;
  regularization: number;
  deringing: boolean;
  deringThreshold: number;
  useEmpiricalPsf: boolean;
}

const DECONV_DEFAULTS: DeconvolveSettings = {
  iterations: 20,
  psfSigma: 2.0,
  psfSize: 15,
  regularization: 0.001,
  deringing: true,
  deringThreshold: 0.1,
  useEmpiricalPsf: false,
};
const EMPIRICAL_PSF_STARS = 30;
const EMPIRICAL_PSF_RADIUS = 15;

function psfSpecOf(options: DeconvolveOptions): PsfSpec {
  const kernel = options.psfKernel;
  if (!options.useEmpiricalPsf) return { kind: "gaussian", sigma: options.psfSigma ?? DECONV_DEFAULTS.psfSigma, size: options.psfSize ?? DECONV_DEFAULTS.psfSize };
  if (kernel && kernel.length > 0) return { kind: "kernel", hash: "", rows: kernel.length, cols: kernel[0].length };
  return { kind: "empirical", numStars: options.psfNumStars ?? EMPIRICAL_PSF_STARS, cutoutRadius: options.psfCutoutRadius ?? EMPIRICAL_PSF_RADIUS };
}

export function deconvolveOp(options: DeconvolveOptions): OpSpec {
  return {
    op: "rl_deconv",
    v: 1,
    iterations: options.iterations ?? DECONV_DEFAULTS.iterations,
    regularization: options.regularization ?? DECONV_DEFAULTS.regularization,
    deringing: options.deringing ?? DECONV_DEFAULTS.deringing,
    deringingThreshold: options.deringThreshold ?? DECONV_DEFAULTS.deringThreshold,
    psf: psfSpecOf(options),
  };
}

export function deconvolveOptionsOf(op: OpSpec): DeconvolveSettings | null {
  if (op.op !== "rl_deconv") return null;
  const gaussian = op.psf.kind === "gaussian" ? op.psf : null;
  return {
    iterations: op.iterations,
    psfSigma: gaussian?.sigma ?? DECONV_DEFAULTS.psfSigma,
    psfSize: gaussian?.size ?? DECONV_DEFAULTS.psfSize,
    regularization: op.regularization,
    deringing: op.deringing,
    deringThreshold: op.deringingThreshold,
    useEmpiricalPsf: gaussian === null,
  };
}

export type RecordedResult<T> = T & RunOpResult;

export async function recordDeconvolveRL(scope: HistoryScope, path: string, options: DeconvolveOptions = {}): Promise<RecordedResult<DeconvolveRunResult>> {
  const op = deconvolveOp(options);
  const psfKernel = op.op === "rl_deconv" && op.psf.kind === "kernel" ? options.psfKernel ?? null : null;
  return (await recordOp(scope, path, op, { psfKernel })) as RecordedResult<DeconvolveRunResult>;
}

export function deconvolveRL(
  path: string,
  outputDir?: string,
  options: DeconvolveOptions = {},
): Promise<DeconvolveRunResult> {
  return withPreview<DeconvolveRunResult>("deconvolve_rl_cmd", outputDir, {
    path,
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
  });
}

export interface BackgroundOptions {
  gridSize?: number;
  polyDegree?: number;
  sigmaClip?: number;
  iterations?: number;
  mode?: string;
}

const BACKGROUND_DEFAULTS = { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" } as const;
const DEBAND_MODELS = ["deband_rows", "deband_cols", "deband_both", "deband_auto"] as const;
export type DebandModel = (typeof DEBAND_MODELS)[number];

function isDebandModel(mode: string): mode is DebandModel {
  return (DEBAND_MODELS as readonly string[]).includes(mode);
}

export function backgroundOp(options: BackgroundOptions): OpSpec {
  const sigmaClip = options.sigmaClip ?? BACKGROUND_DEFAULTS.sigmaClip;
  const iterations = options.iterations ?? BACKGROUND_DEFAULTS.iterations;
  const mode = options.mode ?? BACKGROUND_DEFAULTS.mode;
  if (isDebandModel(mode)) return { op: "background", v: 1, model: mode, config: { sigmaClip, iterations } };
  return {
    op: "background",
    v: 1,
    model: "polynomial",
    config: {
      gridSize: options.gridSize ?? BACKGROUND_DEFAULTS.gridSize,
      polyDegree: options.polyDegree ?? BACKGROUND_DEFAULTS.polyDegree,
      sigmaClip,
      iterations,
      mode: mode === "divide" ? "divide" : "subtract",
    },
  };
}

export interface PolynomialSettings {
  gridSize?: number;
  polyDegree?: number;
  sigmaClip: number;
  iterations: number;
  mode: "subtract" | "divide" | DebandModel;
}

export type BackgroundSettings = { model: "polynomial"; params: PolynomialSettings } | { model: "spline"; dbe: DbeConfig };

export function backgroundOptionsOf(op: OpSpec): BackgroundSettings | null {
  if (op.op !== "background") return null;
  if (op.model === "spline") return { model: "spline", dbe: op.config };
  if (op.model === "polynomial") return { model: "polynomial", params: { ...op.config } };
  return { model: "polynomial", params: { sigmaClip: op.config.sigmaClip, iterations: op.config.iterations, mode: op.model } };
}

export interface RecordedBackgroundResult extends RecordedResult<BackgroundResult> {
  corrected_fits: string;
  corrected_png: string;
  modelUrl?: string;
}

export function asBackgroundResult(res: RunOpResult): RecordedBackgroundResult {
  return {
    ...res,
    corrected_fits: res.fits_path,
    corrected_png: res.png_path,
    model_png: res.model_png ?? "",
    modelUrl: res.modelPreviewUrl,
  } as RecordedBackgroundResult;
}

export async function recordExtractBackground(scope: HistoryScope, path: string, options: BackgroundOptions = {}): Promise<RecordedBackgroundResult> {
  return asBackgroundResult(await recordOp(scope, path, backgroundOp(options)));
}

export function extractBackground(
  path: string,
  outputDir?: string,
  options: BackgroundOptions & { binId?: string } = {},
): Promise<BackgroundResult> {
  return withPreview<BackgroundResult>("extract_background_cmd", outputDir, {
    path,
    gridSize: options.gridSize ?? 8,
    polyDegree: options.polyDegree ?? 3,
    sigmaClip: options.sigmaClip ?? 2.5,
    iterations: options.iterations ?? 3,
    mode: options.mode ?? "subtract",
    binId: options.binId ?? null,
  }, [
    ["corrected_png", "previewUrl"],
    ["model_png", "modelUrl"],
  ]);
}

export interface BackgroundBatchResult {
  results: { bin_id: string; cache_key: string; sample_count: number | null; axis?: string | null }[];
  mode: string;
  rms_residual: number | null;
  dimensions: [number, number];
  elapsed_ms: number;
}

export function extractBackgroundBatch(
  paths: string[],
  binIds: string[],
  outputDir: string,
  options: {
    gridSize?: number;
    polyDegree?: number;
    sigmaClip?: number;
    iterations?: number;
    mode?: string;
    referenceBin?: string | null;
    runToken?: string | null;
  } = {},
): Promise<BackgroundBatchResult> {
  return typedInvoke<BackgroundBatchResult>("extract_background_batch_cmd", {
    paths,
    binIds,
    outputDir,
    gridSize: options.gridSize ?? 8,
    polyDegree: options.polyDegree ?? 3,
    sigmaClip: options.sigmaClip ?? 2.5,
    iterations: options.iterations ?? 3,
    mode: options.mode ?? "subtract",
    referenceBin: options.referenceBin ?? null,
    runToken: options.runToken ?? null,
  });
}

export interface WaveletOptions {
  numScales?: number;
  thresholds?: number[];
  linear?: boolean;
  layerBias?: number[];
}

export interface WaveletSettings {
  numScales: number;
  thresholds: number[];
  linear: boolean;
  layerBias: number[];
}

const WAVELET_DEFAULT_THRESHOLDS = [3.0, 2.5, 2.0, 1.5, 1.0];

export function waveletOp(options: WaveletOptions): OpSpec {
  return {
    op: "wavelet_denoise",
    v: 1,
    numScales: options.numScales ?? 5,
    thresholds: options.thresholds ?? WAVELET_DEFAULT_THRESHOLDS,
    linearDenoise: options.linear ?? true,
    layerBias: options.layerBias ?? null,
  };
}

export function waveletOptionsOf(op: OpSpec): WaveletSettings | null {
  if (op.op !== "wavelet_denoise") return null;
  return {
    numScales: op.numScales,
    thresholds: [...op.thresholds],
    linear: op.linearDenoise,
    layerBias: op.layerBias ? [...op.layerBias] : op.thresholds.map(() => 0),
  };
}

export async function recordWaveletDenoise(scope: HistoryScope, path: string, options: WaveletOptions = {}): Promise<RecordedResult<WaveletResult>> {
  return (await recordOp(scope, path, waveletOp(options))) as RecordedResult<WaveletResult>;
}

export function waveletDenoise(
  path: string,
  outputDir?: string,
  options: WaveletOptions = {},
): Promise<WaveletResult> {
  return withPreview<WaveletResult>("wavelet_denoise_cmd", outputDir, {
    path,
    numScales: options.numScales ?? 5,
    thresholds: options.thresholds ?? [3.0, 2.5, 2.0, 1.5, 1.0],
    linear: options.linear ?? true,
    layerBias: options.layerBias ?? null,
  });
}

export function estimatePsf(
  path: string,
  options: {
    numStars?: number;
    cutoutRadius?: number;
    saturationThreshold?: number;
    maxEllipticity?: number;
  } = {},
): Promise<PsfEstimate> {
  return typedInvoke<PsfEstimate>("estimate_psf_cmd", {
    path,
    numStars: options.numStars ?? 30,
    cutoutRadius: options.cutoutRadius ?? 15,
    saturationThreshold: options.saturationThreshold ?? 0.95,
    maxEllipticity: options.maxEllipticity ?? 0.3,
  });
}

export function applyArcsinhStretch(path: string, outputDir?: string, factor = 50.0): Promise<ArcsinhResult> {
  return withPreview<ArcsinhResult>("apply_arcsinh_stretch_cmd", outputDir, { path, factor });
}

export function arcsinhOptionsOf(op: OpSpec): { factor: number } | null {
  return op.op === "arcsinh" ? { factor: op.factor } : null;
}

export async function recordArcsinhStretch(scope: HistoryScope, path: string, factor = 50.0): Promise<RecordedResult<ArcsinhResult>> {
  return (await recordOp(scope, path, { op: "arcsinh", v: 1, factor })) as RecordedResult<ArcsinhResult>;
}

export interface DebayerResult {
  png_path?: string;
  previewUrl?: string;
  pattern: string;
  method: string;
  r_path: string;
  g_path: string;
  b_path: string;
  dimensions: [number, number];
  elapsed_ms: number;
}

export interface DebayerBatchItem {
  path: string;
  pattern?: string;
  r_path?: string;
  g_path?: string;
  b_path?: string;
  dimensions?: [number, number];
  error?: string;
}

export interface DebayerBatchResult {
  results: DebayerBatchItem[];
  succeeded: number;
  failed: number;
  method: string;
  elapsed_ms: number;
}

export function debayerFits(
  path: string,
  outputDir?: string,
  options: { method?: "bilinear" | "superpixel"; pattern?: string } = {},
): Promise<DebayerResult> {
  return withPreview<DebayerResult>("debayer_fits_cmd", outputDir, {
    path,
    method: options.method ?? "bilinear",
    pattern: options.pattern ?? null,
  });
}

export async function debayerBatch(
  paths: string[],
  outputDir?: string,
  options: { method?: "bilinear" | "superpixel"; pattern?: string } = {},
): Promise<DebayerBatchResult> {
  const dir = outputDir ?? await getOutputDir();
  const result = await typedInvoke<DebayerBatchResult>("debayer_batch_cmd", {
    paths,
    outputDir: dir,
    method: options.method ?? "bilinear",
    pattern: options.pattern ?? null,
  });
  announceCommandOutputs(result);
  return result;
}

export interface GhsOptions {
  stretchFactor: number;
  localIntensity?: number;
  symmetryPoint?: number;
  shadowProtect?: number;
  highlightProtect?: number;
}

export function ghsOp(options: GhsOptions): OpSpec {
  return {
    op: "ghs",
    v: 1,
    d: options.stretchFactor,
    b: options.localIntensity ?? 0.0,
    sp: options.symmetryPoint ?? 0.05,
    lp: options.shadowProtect ?? 0.0,
    hp: options.highlightProtect ?? 1.0,
  };
}

export function ghsOptionsOf(op: OpSpec): Required<GhsOptions> | null {
  if (op.op !== "ghs") return null;
  return { stretchFactor: op.d, localIntensity: op.b, symmetryPoint: op.sp, shadowProtect: op.lp, highlightProtect: op.hp };
}

export async function recordGhsStretch(scope: HistoryScope, path: string, options: GhsOptions = { stretchFactor: 2.0 }): Promise<RecordedResult<ArcsinhResult>> {
  return (await recordOp(scope, path, ghsOp(options))) as RecordedResult<ArcsinhResult>;
}

export function applyGhsStretch(
  path: string,
  outputDir?: string,
  options: GhsOptions = { stretchFactor: 2.0 },
): Promise<ArcsinhResult> {
  return withPreview<ArcsinhResult>("apply_ghs_stretch_cmd", outputDir, {
    path,
    stretchFactor: options.stretchFactor,
    localIntensity: options.localIntensity ?? 0.0,
    symmetryPoint: options.symmetryPoint ?? 0.05,
    shadowProtect: options.shadowProtect ?? 0.0,
    highlightProtect: options.highlightProtect ?? 1.0,
  });
}

export async function ghsStretchComposite(
  outputDir?: string,
  options: GhsOptions = { stretchFactor: 2.0 },
): Promise<ArcsinhResult> {
  const dir = outputDir ?? await getOutputDir();
  return typedInvoke<ArcsinhResult>("ghs_stretch_composite_cmd", {
    outputDir: dir,
    stretchFactor: options.stretchFactor,
    localIntensity: options.localIntensity ?? 0.0,
    symmetryPoint: options.symmetryPoint ?? 0.05,
    shadowProtect: options.shadowProtect ?? 0.0,
    highlightProtect: options.highlightProtect ?? 1.0,
  });
}

export interface MaskedStretchOptions {
  iterations?: number;
  targetBackground?: number;
  maskGrowth?: number;
  maskSoftness?: number;
  protectionAmount?: number;
  luminanceProtect?: boolean;
  detectionSigma?: number;
  maxEccentricity?: number;
}

const MASKED_STRETCH_LUMINANCE_CEILING = 0.85;
const MASKED_STRETCH_CONVERGENCE = 0.00001;

export function maskedStretchOp(options: MaskedStretchOptions): OpSpec {
  return {
    op: "masked_stretch",
    v: 1,
    iterations: options.iterations ?? 10,
    targetBackground: options.targetBackground ?? 0.25,
    maskGrowth: options.maskGrowth ?? 2.5,
    maskSoftness: options.maskSoftness ?? 4.0,
    detectionSigma: options.detectionSigma ?? 8.0,
    maxEccentricity: options.maxEccentricity ?? 0.85,
    luminanceProtect: options.luminanceProtect ?? true,
    luminanceCeiling: MASKED_STRETCH_LUMINANCE_CEILING,
    protectionAmount: options.protectionAmount ?? 0.85,
    convergenceThreshold: MASKED_STRETCH_CONVERGENCE,
  };
}

export function maskedStretchOptionsOf(op: OpSpec): Required<MaskedStretchOptions> | null {
  if (op.op !== "masked_stretch") return null;
  return {
    iterations: op.iterations,
    targetBackground: op.targetBackground,
    maskGrowth: op.maskGrowth,
    maskSoftness: op.maskSoftness,
    protectionAmount: op.protectionAmount,
    luminanceProtect: op.luminanceProtect,
    detectionSigma: op.detectionSigma,
    maxEccentricity: op.maxEccentricity,
  };
}

export async function recordMaskedStretch(scope: HistoryScope, path: string, options: MaskedStretchOptions = {}): Promise<RecordedResult<MaskedStretchResult>> {
  return (await recordOp(scope, path, maskedStretchOp(options))) as RecordedResult<MaskedStretchResult>;
}

export function maskedStretch(
  path: string,
  outputDir?: string,
  options: MaskedStretchOptions = {},
): Promise<MaskedStretchResult> {
  return withPreview<MaskedStretchResult>("masked_stretch_cmd", outputDir, {
    path,
    iterations: options.iterations ?? 10,
    targetBackground: options.targetBackground ?? 0.25,
    maskGrowth: options.maskGrowth ?? 2.5,
    maskSoftness: options.maskSoftness ?? 4.0,
    protectionAmount: options.protectionAmount ?? 0.85,
    luminanceProtect: options.luminanceProtect ?? true,
    detectionSigma: options.detectionSigma ?? 8.0,
    maxEccentricity: options.maxEccentricity ?? 0.85,
  });
}

export function arcsinhStretchComposite(factor?: number, outputDir?: string): Promise<ArcsinhResult>;
export function arcsinhStretchComposite(
  factor: number,
  outputDir: string | undefined,
  chain: CompositeChainCall,
): Promise<CompositeStretchResult>;
export async function arcsinhStretchComposite(
  factor = 50.0,
  outputDir?: string,
  chain?: CompositeChainCall,
): Promise<ArcsinhResult | CompositeStretchResult> {
  if (chain) {
    return withPreview<CompositeStretchResult>("arcsinh_stretch_composite_cmd", outputDir, {
      factor,
      ...chainArgs(chain),
    }, CHAIN_PREVIEWS);
  }
  const dir = outputDir ?? await getOutputDir();
  return typedInvoke<ArcsinhResult>("arcsinh_stretch_composite_cmd", {
    outputDir: dir,
    factor,
  });
}

export interface MaskedStretchCompositeOptions {
  iterations?: number;
  targetBackground?: number;
  maskGrowth?: number;
  maskSoftness?: number;
  protectionAmount?: number;
  luminanceProtect?: boolean;
  sharedMask?: boolean;
  detectionSigma?: number;
  maxEccentricity?: number;
}

function maskedStretchCompositeArgs(options: MaskedStretchCompositeOptions) {
  return {
    iterations: options.iterations ?? 10,
    targetBackground: options.targetBackground ?? 0.25,
    maskGrowth: options.maskGrowth ?? 2.5,
    maskSoftness: options.maskSoftness ?? 4.0,
    protectionAmount: options.protectionAmount ?? 0.85,
    luminanceProtect: options.luminanceProtect ?? true,
    sharedMask: options.sharedMask ?? true,
    detectionSigma: options.detectionSigma ?? 8.0,
    maxEccentricity: options.maxEccentricity ?? 0.85,
  };
}

export function maskedStretchComposite(outputDir?: string, options?: MaskedStretchCompositeOptions): Promise<MaskedStretchResult>;
export function maskedStretchComposite(
  outputDir: string | undefined,
  options: MaskedStretchCompositeOptions,
  chain: CompositeChainCall,
): Promise<CompositeMaskedStretchResult>;
export async function maskedStretchComposite(
  outputDir?: string,
  options: MaskedStretchCompositeOptions = {},
  chain?: CompositeChainCall,
): Promise<MaskedStretchResult | CompositeMaskedStretchResult> {
  if (chain) {
    return withPreview<CompositeMaskedStretchResult>("masked_stretch_composite_cmd", outputDir, {
      ...maskedStretchCompositeArgs(options),
      ...chainArgs(chain),
    }, CHAIN_PREVIEWS);
  }
  const dir = outputDir ?? await getOutputDir();
  return typedInvoke<MaskedStretchResult>("masked_stretch_composite_cmd", {
    outputDir: dir,
    ...maskedStretchCompositeArgs(options),
  });
}

export interface StarRemovalResult {
  png_path?: string;
  fits_path?: string;
  stars_png_path?: string;
  stars_fits_path?: string;
  stars_masked: number;
  mask_coverage: number;
  elapsed_ms: number;
  dimensions: [number, number];
  previewUrl?: string;
  starsPreviewUrl?: string;
}

export interface StarRemovalOptions {
  detectionSigma?: number;
  maxEccentricity?: number;
  growthFactor?: number;
  softness?: number;
  brightCeiling?: number;
}

function starRemovalArgs(options: StarRemovalOptions) {
  return {
    detectionSigma: options.detectionSigma ?? 4.0,
    maxEccentricity: options.maxEccentricity ?? 0.9,
    growthFactor: options.growthFactor ?? 3.0,
    softness: options.softness ?? 6.0,
    brightCeiling: options.brightCeiling ?? 0.95,
  };
}

export function removeStars(
  path: string,
  outputDir?: string,
  options: StarRemovalOptions = {},
): Promise<StarRemovalResult> {
  return withPreview<StarRemovalResult>("remove_stars_cmd", outputDir, {
    path,
    ...starRemovalArgs(options),
  }, [
    ["png_path", "previewUrl"],
    ["stars_png_path", "starsPreviewUrl"],
  ]);
}

export type CompositeStarRemovalResult = StarRemovalResult & CompositeStfFields;

export function removeStarsComposite(
  outputDir?: string,
  options: StarRemovalOptions = {},
  stfLinked?: boolean,
): Promise<CompositeStarRemovalResult> {
  return withPreview<CompositeStarRemovalResult>("remove_stars_composite_cmd", outputDir, {
    ...starRemovalArgs(options),
    stfLinked,
  }, [
    ["png_path", "previewUrl"],
    ["stars_png_path", "starsPreviewUrl"],
  ]);
}

export function spccCalibrate(
  rPath: string,
  gPath: string,
  bPath: string,
  options: {
    wcsPath?: string;
    whiteReference?: string;
    minSnr?: number;
    maxStars?: number;
    catalog?: "gaia" | "builtin";
    wavelengthsNm?: [number, number, number] | null;
  } = {},
): Promise<SpccResult> {
  return typedInvoke<SpccResult>("spcc_calibrate_cmd", {
    rPath,
    gPath,
    bPath,
    wcsPath: options.wcsPath ?? null,
    whiteReference: options.whiteReference ?? "average_spiral",
    minSnr: options.minSnr ?? 20.0,
    maxStars: options.maxStars ?? 200,
    catalog: options.catalog ?? "gaia",
    wavelengthsNm: options.wavelengthsNm ?? null,
  });
}
