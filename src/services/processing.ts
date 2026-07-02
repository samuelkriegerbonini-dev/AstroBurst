import { typedInvoke, withPreview, getOutputDir } from "../infrastructure/tauri";
import type {
  DeconvolveResult,
  BackgroundResult,
  WaveletResult,
  PsfEstimate,
  ArcsinhResult,
  MaskedStretchResult,
  SpccResult,
} from "../shared/types/processing";

export function deconvolveRL(
  path: string,
  outputDir?: string,
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
  } = {},
): Promise<DeconvolveResult> {
  return withPreview<DeconvolveResult>("deconvolve_rl_cmd", outputDir, {
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
  });
}

export function extractBackground(
  path: string,
  outputDir?: string,
  options: {
    gridSize?: number;
    polyDegree?: number;
    sigmaClip?: number;
    iterations?: number;
    mode?: string;
    binId?: string;
  } = {},
): Promise<BackgroundResult> {
  return withPreview<BackgroundResult>("extract_background_cmd", outputDir, {
    path,
    gridSize: options.gridSize ?? 8,
    polyDegree: options.polyDegree ?? 3,
    sigmaClip: options.sigmaClip ?? 2.5,
    iterations: options.iterations ?? 3,
    mode: options.mode ?? "subtract",
    binId: options.binId ?? null,
    persistToDisk: false,
  }, [
    ["corrected_png", "previewUrl"],
    ["model_png", "modelUrl"],
  ]);
}

export interface BackgroundBatchResult {
  results: { bin_id: string; cache_key: string; sample_count: number }[];
  mode: string;
  rms_residual: number;
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
  });
}

export function waveletDenoise(
  path: string,
  outputDir?: string,
  options: {
    numScales?: number;
    thresholds?: number[];
    linear?: boolean;
  } = {},
): Promise<WaveletResult> {
  return withPreview<WaveletResult>("wavelet_denoise_cmd", outputDir, {
    path,
    numScales: options.numScales ?? 5,
    thresholds: options.thresholds ?? [3.0, 2.5, 2.0, 1.5, 1.0],
    linear: options.linear ?? true,
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
  return typedInvoke<DebayerBatchResult>("debayer_batch_cmd", {
    paths,
    outputDir: dir,
    method: options.method ?? "bilinear",
    pattern: options.pattern ?? null,
  });
}

export interface GhsOptions {
  stretchFactor: number;
  localIntensity?: number;
  symmetryPoint?: number;
  shadowProtect?: number;
  highlightProtect?: number;
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

export function maskedStretch(
  path: string,
  outputDir?: string,
  options: {
    iterations?: number;
    targetBackground?: number;
    maskGrowth?: number;
    maskSoftness?: number;
    protectionAmount?: number;
    luminanceProtect?: boolean;
    detectionSigma?: number;
    maxEccentricity?: number;
  } = {},
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

export async function arcsinhStretchComposite(
  factor = 50.0,
  outputDir?: string,
): Promise<ArcsinhResult> {
  const dir = outputDir ?? await getOutputDir();
  return typedInvoke<ArcsinhResult>("arcsinh_stretch_composite_cmd", {
    outputDir: dir,
    factor,
  });
}

export async function maskedStretchComposite(
  outputDir?: string,
  options: {
    iterations?: number;
    targetBackground?: number;
    maskGrowth?: number;
    maskSoftness?: number;
    protectionAmount?: number;
    luminanceProtect?: boolean;
    sharedMask?: boolean;
    detectionSigma?: number;
    maxEccentricity?: number;
  } = {},
): Promise<MaskedStretchResult> {
  const dir = outputDir ?? await getOutputDir();
  return typedInvoke<MaskedStretchResult>("masked_stretch_composite_cmd", {
    outputDir: dir,
    iterations: options.iterations ?? 10,
    targetBackground: options.targetBackground ?? 0.25,
    maskGrowth: options.maskGrowth ?? 2.5,
    maskSoftness: options.maskSoftness ?? 4.0,
    protectionAmount: options.protectionAmount ?? 0.85,
    luminanceProtect: options.luminanceProtect ?? true,
    sharedMask: options.sharedMask ?? true,
    detectionSigma: options.detectionSigma ?? 8.0,
    maxEccentricity: options.maxEccentricity ?? 0.85,
  });
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
  });
}
