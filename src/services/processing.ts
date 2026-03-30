import { safeInvoke, withPreview } from "../infrastructure/tauri";

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
) {
  return withPreview("deconvolve_rl_cmd", outputDir, {
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
  } = {},
) {
  return withPreview("extract_background_cmd", outputDir, {
    path,
    gridSize: options.gridSize ?? 8,
    polyDegree: options.polyDegree ?? 3,
    sigmaClip: options.sigmaClip ?? 2.5,
    iterations: options.iterations ?? 3,
    mode: options.mode ?? "subtract",
  }, [
    ["corrected_png", "previewUrl"],
    ["model_png", "modelUrl"],
  ]);
}

export function waveletDenoise(
  path: string,
  outputDir?: string,
  options: {
    numScales?: number;
    thresholds?: number[];
    linear?: boolean;
  } = {},
) {
  return withPreview("wavelet_denoise_cmd", outputDir, {
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
) {
  return safeInvoke("estimate_psf_cmd", {
    path,
    numStars: options.numStars ?? 30,
    cutoutRadius: options.cutoutRadius ?? 15,
    saturationThreshold: options.saturationThreshold ?? 0.95,
    maxEllipticity: options.maxEllipticity ?? 0.3,
  });
}

export function applyArcsinhStretch(path: string, outputDir?: string, factor = 50.0) {
  return withPreview("apply_arcsinh_stretch_cmd", outputDir, { path, factor });
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
  } = {},
) {
  return withPreview("masked_stretch_cmd", outputDir, {
    path,
    iterations: options.iterations ?? 10,
    targetBackground: options.targetBackground ?? 0.25,
    maskGrowth: options.maskGrowth ?? 2.5,
    maskSoftness: options.maskSoftness ?? 4.0,
    protectionAmount: options.protectionAmount ?? 0.85,
    luminanceProtect: options.luminanceProtect ?? true,
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
  } = {},
) {
  return safeInvoke("spcc_calibrate_cmd", {
    rPath,
    gPath,
    bPath,
    wcsPath: options.wcsPath ?? null,
    whiteReference: options.whiteReference ?? "average_spiral",
    minSnr: options.minSnr ?? 20.0,
    maxStars: options.maxStars ?? 200,
  });
}
