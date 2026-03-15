let _convertFileSrc: ((path: string) => string) | null = null;
let _invoke: ((cmd: string, args?: Record<string, any>) => Promise<any>) | null = null;

const isTauri = (): boolean => !!window.__TAURI_INTERNALS__;

async function ensureConvertFileSrc(): Promise<(path: string) => string> {
  if (_convertFileSrc) return _convertFileSrc;
  const { convertFileSrc } = await import("@tauri-apps/api/core");
  _convertFileSrc = convertFileSrc;
  return convertFileSrc;
}

async function ensureInvoke(): Promise<(cmd: string, args?: Record<string, any>) => Promise<any>> {
  if (_invoke) return _invoke;
  const { invoke } = await import("@tauri-apps/api/core");
  _invoke = invoke;
  return invoke;
}

async function getPreviewUrl(path: string): Promise<string> {
  if (!path) return "";
  if (isTauri()) {
    const convert = await ensureConvertFileSrc();
    const cleanPath = path.startsWith("\\\\?\\") ? path.slice(4) : path;
    return convert(cleanPath);
  }
  return path;
}

function parseRawPixelBuffer(arrayBuffer: ArrayBuffer) {
  const header = new DataView(arrayBuffer, 0, 16);
  return {
    width: header.getUint32(0, true),
    height: header.getUint32(4, true),
    dataMin: header.getFloat32(8, true),
    dataMax: header.getFloat32(12, true),
    pixels: new Float32Array(arrayBuffer, 16),
  };
}

function toUint8Array(raw: any): Uint8Array {
  if (raw instanceof ArrayBuffer) return new Uint8Array(raw);
  if (raw instanceof Uint8Array) return raw;
  if (ArrayBuffer.isView(raw)) return new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength);
  if (Array.isArray(raw)) return new Uint8Array(raw);
  throw new Error(`Unexpected IPC response type: ${typeof raw} / ${raw?.constructor?.name}`);
}

const safeInvoke = async (command: string, args: Record<string, any> = {}): Promise<any> => {
  if (isTauri()) {
    const invoke = await ensureInvoke();
    return invoke(command, args);
  }
  throw new Error(`Command "${command}" requires Tauri desktop environment.`);
};

import { useMemo } from "react";
import { getOutputDir, getOutputDirTiles } from "../utils/outputdir";

async function resolveDir(explicit?: string): Promise<string> {
  if (explicit && explicit !== "./output") return explicit;
  return getOutputDir();
}

async function resolvePreview(res: any, key = "png_path", urlKey = "previewUrl"): Promise<any> {
  if (res[key]) res[urlKey] = await getPreviewUrl(res[key]);
  return res;
}

async function withDirInvoke(
  cmd: string,
  outputDir: string | undefined,
  args: Record<string, any> = {},
): Promise<any> {
  const dir = await resolveDir(outputDir);
  return safeInvoke(cmd, { outputDir: dir, ...args });
}

async function withPreview(
  cmd: string,
  outputDir: string | undefined,
  args: Record<string, any> = {},
  previews: [string, string][] = [["png_path", "previewUrl"]],
): Promise<any> {
  const res = await withDirInvoke(cmd, outputDir, args);
  for (const [key, urlKey] of previews) {
    await resolvePreview(res, key, urlKey);
  }
  return res;
}

const FFT_HEADER_SIZE = 32;

function parseFftBuffer(bytes: Uint8Array) {
  if (bytes.length < FFT_HEADER_SIZE) {
    throw new Error(`FFT: response too small (${bytes.length} bytes)`);
  }

  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);

  const expectedLen = FFT_HEADER_SIZE + width * height;
  if (bytes.length < expectedLen) {
    throw new Error(`FFT: expected ${expectedLen} bytes, got ${bytes.length}`);
  }

  return {
    width,
    height,
    dc_magnitude: view.getFloat32(8, true),
    max_magnitude: view.getFloat32(12, true),
    elapsed_ms: view.getUint32(16, true),
    original_size: view.getUint32(20, true),
    windowed: view.getUint32(24, true) !== 0,
    pixels: new Uint8Array(bytes.buffer, bytes.byteOffset + FFT_HEADER_SIZE, width * height),
  };
}

const CUBE_PREVIEWS: [string, string][] = [
  ["collapsed_path", "collapsedPreviewUrl"],
  ["collapsed_median_path", "collapsedMedianPreviewUrl"],
];

export interface ExportFitsOptions {
  applyStfStretch?: boolean;
  shadow?: number;
  midtone?: number;
  highlight?: number;
  copyWcs?: boolean;
  copyMetadata?: boolean;
  bitpix?: number;
}

export interface ExportFitsRgbOptions {
  copyWcs?: boolean;
  copyMetadata?: boolean;
}

export function useBackend() {
  return useMemo(() => ({
    processFits: (path: string, outputDir?: string) =>
      withPreview("process_fits", outputDir, { path }),

    processFitsFull: (path: string, outputDir?: string) =>
      withPreview("process_fits_full", outputDir, { path }),

    getRawPixelsBinary: async (path: string) => {
      const buffer = await safeInvoke("get_raw_pixels_binary", { path });
      return parseRawPixelBuffer(buffer);
    },

    getRawPixelsPreview: async (path: string, maxDim = 2048) => {
      const buffer = await safeInvoke("get_raw_pixels_preview", { path, maxDim });
      return parseRawPixelBuffer(buffer);
    },

    exportFits: (path: string, outputPath: string, options: ExportFitsOptions = {}) =>
      safeInvoke("export_fits", {
        path,
        outputPath,
        applyStfStretch: options.applyStfStretch ?? false,
        shadow: options.shadow,
        midtone: options.midtone,
        highlight: options.highlight,
        copyWcs: options.copyWcs ?? true,
        copyMetadata: options.copyMetadata ?? true,
        bitpix: options.bitpix,
      }),

    exportFitsRgb: (
      rPath: string | null,
      gPath: string | null,
      bPath: string | null,
      outputPath: string,
      options: ExportFitsRgbOptions = {},
    ) => safeInvoke("export_fits_rgb", {
      rPath,
      gPath,
      bPath,
      outputPath,
      copyWcs: options.copyWcs ?? true,
      copyMetadata: options.copyMetadata ?? true,
    }),

    getHeader: (path: string) => safeInvoke("get_header", { path }),
    getFullHeader: (path: string) => safeInvoke("get_full_header", { path }),
    getFitsExtensions: (path: string) => safeInvoke("get_fits_extensions", { path }),

    getHeaderByHdu: (path: string, hduIndex: number) =>
      safeInvoke("get_header_by_hdu", { path, hduIndex }),

    detectNarrowbandFilters: (paths: string[]) =>
      safeInvoke("detect_narrowband_filters", { paths }),

    computeHistogram: (path: string) => safeInvoke("compute_histogram", { path }),

    computeFftSpectrum: async (path: string) => {
      const raw = await safeInvoke("compute_fft_spectrum", { path });
      return parseFftBuffer(toUint8Array(raw));
    },

    detectStars: (path: string, sigma = 5.0, maxStars = 200) =>
      safeInvoke("detect_stars", { path, sigma, maxStars }),

    applyStfRender: (
      path: string,
      outputDir: string | undefined,
      shadow: number,
      midtone: number,
      highlight: number,
    ) => withPreview("apply_stf_render", outputDir, { path, shadow, midtone, highlight }),

    generateTiles: async (path: string, outputDir?: string, tileSize = 256) => {
      const dir = outputDir || await getOutputDirTiles();
      return safeInvoke("generate_tiles", { path, outputDir: dir, tileSize });
    },

    getTile: (path: string, outputDir: string, level: number, col: number, row: number) =>
      safeInvoke("get_tile", { path, outputDir, level, col, row }),

    processCube: (path: string, outputDir?: string, frameStep = 5) =>
      withPreview("process_cube_cmd", outputDir, { path, frameStep }, CUBE_PREVIEWS),

    processCubeLazy: (path: string, outputDir?: string, frameStep = 5) =>
      withPreview("process_cube_lazy_cmd", outputDir, { path, frameStep }, CUBE_PREVIEWS),

    getCubeInfo: (path: string) => safeInvoke("get_cube_info", { path }),

    getCubeFrame: async (path: string, frameIndex: number, outputPath: string, outputFits?: string) => {
      const dir = await getOutputDir();
      const resolve = (p: string) => p.startsWith("./output") ? p.replace("./output", dir) : p;
      return safeInvoke("get_cube_frame", {
        path,
        frameIndex,
        outputPath: resolve(outputPath),
        outputFits: outputFits ? resolve(outputFits) : undefined,
      });
    },

    getCubeSpectrum: (path: string, x: number, y: number) =>
      safeInvoke("get_cube_spectrum", { path, x, y }),

    plateSolve: (path: string, options: Record<string, any> = {}) =>
      safeInvoke("plate_solve_cmd", { path, ...options }),

    getWcsInfo: (path: string) => safeInvoke("get_wcs_info", { path }),

    pixelToWorld: (path: string, x: number, y: number) =>
      safeInvoke("pixel_to_world", { path, x, y }),

    calibrate: (sciencePath: string, outputDir?: string, options: Record<string, any> = {}) =>
      withPreview("calibrate", outputDir, { sciencePath, ...options }),

    stackFrames: (paths: string[], outputDir?: string, options: Record<string, any> = {}) =>
      withPreview("stack", outputDir, { paths, ...options }),

    drizzleStack: (paths: string[], outputDir?: string, options: Record<string, any> = {}) =>
      withPreview("drizzle_stack_cmd", outputDir, { paths, ...options }, [
        ["png_path", "previewUrl"],
        ["weight_map_path", "weightMapUrl"],
      ]),

    drizzleRgb: (
      rPaths: string[] | null,
      gPaths: string[] | null,
      bPaths: string[] | null,
      outputDir?: string,
      options: Record<string, any> = {},
    ) => withPreview("drizzle_rgb_cmd", outputDir, { rPaths, gPaths, bPaths, ...options }),

    composeRgb: (
      rPath: string | null,
      gPath: string | null,
      bPath: string | null,
      outputDir?: string,
      options: Record<string, any> = {},
    ) => withPreview("compose_rgb_cmd", outputDir, { rPath, gPath, bPath, ...options }),

    resampleFits: (
      path: string,
      targetWidth: number,
      targetHeight: number,
      outputDir?: string,
    ) => withPreview("resample_fits_cmd", outputDir, { path, targetWidth, targetHeight }),

    deconvolveRL: (
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
    ) => withPreview("deconvolve_rl_cmd", outputDir, {
      path,
      iterations: options.iterations ?? 20,
      psfSigma: options.psfSigma ?? 2.0,
      psfSize: options.psfSize ?? 15,
      regularization: options.regularization ?? 0.001,
      deringing: options.deringing ?? true,
      deringThreshold: options.deringThreshold ?? 0.1,
      useEmpiricalPsf: options.useEmpiricalPsf ?? false,
      psfNumStars: options.psfNumStars ?? 3,
      psfCutoutRadius: options.psfCutoutRadius ?? 15,
    }),

    extractBackground: (
      path: string,
      outputDir?: string,
      options: {
        gridSize?: number;
        polyDegree?: number;
        sigmaClip?: number;
        iterations?: number;
        mode?: string;
      } = {},
    ) => withPreview("extract_background_cmd", outputDir, {
      path,
      gridSize: options.gridSize ?? 8,
      polyDegree: options.polyDegree ?? 3,
      sigmaClip: options.sigmaClip ?? 2.5,
      iterations: options.iterations ?? 3,
      mode: options.mode ?? "subtract",
    }, [
      ["corrected_png", "previewUrl"],
      ["model_png", "modelUrl"],
    ]),

    waveletDenoise: (
      path: string,
      outputDir?: string,
      options: {
        numScales?: number;
        thresholds?: number[];
        linear?: boolean;
      } = {},
    ) => withPreview("wavelet_denoise_cmd", outputDir, {
      path,
      numScales: options.numScales ?? 5,
      thresholds: options.thresholds ?? [3.0, 2.5, 2.0, 1.5, 1.0],
      linear: options.linear ?? true,
    }),

    getConfig: () => safeInvoke("get_config"),
    updateConfig: (field: string, value: any) => safeInvoke("update_config", { field, value }),
    saveApiKey: (key: string, service?: string) => safeInvoke("save_api_key", { key, service }),
    getApiKey: () => safeInvoke("get_api_key"),

    estimatePsf: (path: string, options: {
      numStars?: number;
      cutoutRadius?: number;
      saturationThreshold?: number;
      maxEllipticity?: number;
    } = {}) => safeInvoke("estimate_psf_cmd", {
      path,
      numStars: options.numStars ?? 3,
      cutoutRadius: options.cutoutRadius ?? 15,
      saturationThreshold: options.saturationThreshold ?? 0.95,
      maxEllipticity: options.maxEllipticity ?? 0.3,
    }),

    runCalibrationPipeline: (request: {
      channels: { label: string; paths: string[] }[];
      dark_paths: string[];
      flat_paths: string[];
      bias_paths: string[];
      sigma_low?: number;
      sigma_high?: number;
      normalize?: boolean;
    }) => safeInvoke("run_pipeline_cmd", { request }),

    applyArcsinhStretch: (path: string, outputDir?: string, factor = 50.0) =>
      withPreview("apply_arcsinh_stretch_cmd", outputDir, { path, factor }),

    resolveOutputDir: getOutputDir,
  }), []);
}
