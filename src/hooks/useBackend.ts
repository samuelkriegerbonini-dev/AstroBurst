let _convertFileSrc: ((path: string) => string) | null = null;
let _invoke: ((cmd: string, args?: Record<string, any>) => Promise<any>) | null = null;

const isTauri = (): boolean => !!window.__TAURI_INTERNALS__;

const DEFAULT_OUTPUT_DIR = "./output";

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
    let cleanPath = path.startsWith("\\\\?\\") ? path.slice(4) : path;
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

export function useBackend() {
  return useMemo(() => ({
    processFits: async (path: string, outputDir = DEFAULT_OUTPUT_DIR) => {
      const res = await safeInvoke("process_fits", { path, outputDir });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    processFitsFull: async (path: string, outputDir = DEFAULT_OUTPUT_DIR) => {
      const res = await safeInvoke("process_fits_full", { path, outputDir });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    processBatch: async (paths: string[], outputDir = DEFAULT_OUTPUT_DIR) => {
      const res = await safeInvoke("process_batch", { paths, outputDir });
      if (res.results) {
        await Promise.all(
          res.results.map(async (r: any) => {
            if (r.png_path) r.previewUrl = await getPreviewUrl(r.png_path);
          }),
        );
      }
      return res;
    },

    getRawPixelsBinary: async (path: string) => {
      const buffer = await safeInvoke("get_raw_pixels_binary", { path });
      return parseRawPixelBuffer(buffer);
    },

    getRawPixelsPreview: async (path: string, maxDim = 2048) => {
      const buffer = await safeInvoke("get_raw_pixels_preview", { path, maxDim });
      return parseRawPixelBuffer(buffer);
    },

    exportFits: (path: string, outputPath: string, options: Record<string, any> = {}) =>
      safeInvoke("export_fits", { path, outputPath, ...options }),

    exportFitsRgb: (
      rPath: string | null,
      gPath: string | null,
      bPath: string | null,
      outputPath: string,
    ) => safeInvoke("export_fits_rgb", { rPath, gPath, bPath, outputPath }),

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
      const bytes = toUint8Array(raw);

      if (bytes.length < 24) {
        throw new Error(`FFT: response too small (${bytes.length} bytes)`);
      }

      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
      const width = view.getUint32(0, true);
      const height = view.getUint32(4, true);
      const dc_magnitude = view.getFloat32(8, true);
      const max_magnitude = view.getFloat32(12, true);
      const elapsed_ms = view.getUint32(16, true);

      const expectedLen = 24 + width * height;
      if (bytes.length < expectedLen) {
        throw new Error(`FFT: expected ${expectedLen} bytes, got ${bytes.length}`);
      }

      const pixels = new Uint8Array(bytes.buffer, bytes.byteOffset + 24, width * height);

      return { width, height, dc_magnitude, max_magnitude, elapsed_ms, pixels };
    },

    detectStars: (path: string, sigma = 5.0, maxStars = 200) =>
      safeInvoke("detect_stars", { path, sigma, maxStars }),

    applyStfRender: async (
      path: string,
      outputDir = DEFAULT_OUTPUT_DIR,
      shadow: number,
      midtone: number,
      highlight: number,
    ) => {
      const res = await safeInvoke("apply_stf_render", {
        path,
        outputDir,
        shadow,
        midtone,
        highlight,
      });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    generateTiles: (path: string, outputDir: string, tileSize = 256) =>
      safeInvoke("generate_tiles", { path, outputDir, tileSize }),

    getTile: (path: string, outputDir: string, level: number, col: number, row: number) =>
      safeInvoke("get_tile", { path, outputDir, level, col, row }),

    processCube: async (path: string, outputDir = DEFAULT_OUTPUT_DIR, frameStep = 5) => {
      const res = await safeInvoke("process_cube_cmd", { path, outputDir, frameStep });
      if (res.collapsed_path) res.collapsedPreviewUrl = await getPreviewUrl(res.collapsed_path);
      if (res.collapsed_median_path)
        res.collapsedMedianPreviewUrl = await getPreviewUrl(res.collapsed_median_path);
      return res;
    },

    processCubeLazy: async (path: string, outputDir = DEFAULT_OUTPUT_DIR, frameStep = 5) => {
      const res = await safeInvoke("process_cube_lazy_cmd", { path, outputDir, frameStep });
      if (res.collapsed_path) res.collapsedPreviewUrl = await getPreviewUrl(res.collapsed_path);
      if (res.collapsed_median_path)
        res.collapsedMedianPreviewUrl = await getPreviewUrl(res.collapsed_median_path);
      return res;
    },

    getCubeInfo: (path: string) => safeInvoke("get_cube_info", { path }),

    getCubeFrame: (path: string, frameIndex: number, outputPath: string, outputFits?: string) =>
      safeInvoke("get_cube_frame", { path, frameIndex, outputPath, outputFits }),

    getCubeSpectrum: (path: string, x: number, y: number) =>
      safeInvoke("get_cube_spectrum", { path, x, y }),

    plateSolve: (path: string, options: Record<string, any> = {}) =>
      safeInvoke("plate_solve_cmd", { path, ...options }),

    getWcsInfo: (path: string) => safeInvoke("get_wcs_info", { path }),

    pixelToWorld: (path: string, x: number, y: number) =>
      safeInvoke("pixel_to_world", { path, x, y }),

    worldToPixel: (path: string, ra: number, dec: number) =>
      safeInvoke("world_to_pixel", { path, ra, dec }),

    calibrate: async (sciencePath: string, outputDir = DEFAULT_OUTPUT_DIR, options: Record<string, any> = {}) => {
      const res = await safeInvoke("calibrate", { sciencePath, outputDir, ...options });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    stackFrames: async (paths: string[], outputDir = DEFAULT_OUTPUT_DIR, options: Record<string, any> = {}) => {
      const res = await safeInvoke("stack", { paths, outputDir, ...options });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    drizzleStack: async (paths: string[], outputDir = DEFAULT_OUTPUT_DIR, options: Record<string, any> = {}) => {
      const res = await safeInvoke("drizzle_stack_cmd", { paths, outputDir, ...options });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      if (res.weight_map_path) res.weightMapUrl = await getPreviewUrl(res.weight_map_path);
      return res;
    },

    drizzleRgb: async (
      rPaths: string[] | null,
      gPaths: string[] | null,
      bPaths: string[] | null,
      outputDir = DEFAULT_OUTPUT_DIR,
      options: Record<string, any> = {},
    ) => {
      const res = await safeInvoke("drizzle_rgb_cmd", {
        rPaths,
        gPaths,
        bPaths,
        outputDir,
        ...options,
      });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    composeRgb: async (
      rPath: string | null,
      gPath: string | null,
      bPath: string | null,
      outputDir = DEFAULT_OUTPUT_DIR,
      options: Record<string, any> = {},
    ) => {
      const res = await safeInvoke("compose_rgb_cmd", { rPath, gPath, bPath, outputDir, ...options });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    resampleFits: async (
      path: string,
      targetWidth: number,
      targetHeight: number,
      outputDir = DEFAULT_OUTPUT_DIR,
    ) => {
      const res = await safeInvoke("resample_fits_cmd", { path, targetWidth, targetHeight, outputDir });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    deconvolveRL: async (
      path: string,
      outputDir = DEFAULT_OUTPUT_DIR,
      options: {
        iterations?: number;
        psfSigma?: number;
        psfSize?: number;
        regularization?: number;
        deringing?: boolean;
        deringThreshold?: number;
      } = {},
    ) => {
      const res = await safeInvoke("deconvolve_rl_cmd", {
        path,
        outputDir,
        iterations: options.iterations ?? 20,
        psfSigma: options.psfSigma ?? 2.0,
        psfSize: options.psfSize ?? 15,
        regularization: options.regularization ?? 0.001,
        deringing: options.deringing ?? true,
        deringThreshold: options.deringThreshold ?? 0.1,
      });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    extractBackground: async (
      path: string,
      outputDir = DEFAULT_OUTPUT_DIR,
      options: {
        gridSize?: number;
        polyDegree?: number;
        sigmaClip?: number;
        iterations?: number;
        mode?: string;
      } = {},
    ) => {
      const res = await safeInvoke("extract_background_cmd", {
        path,
        outputDir,
        gridSize: options.gridSize ?? 8,
        polyDegree: options.polyDegree ?? 3,
        sigmaClip: options.sigmaClip ?? 2.5,
        iterations: options.iterations ?? 3,
        mode: options.mode ?? "subtract",
      });
      if (res.corrected_png) res.previewUrl = await getPreviewUrl(res.corrected_png);
      if (res.model_png) res.modelUrl = await getPreviewUrl(res.model_png);
      return res;
    },

    waveletDenoise: async (
      path: string,
      outputDir = DEFAULT_OUTPUT_DIR,
      options: {
        numScales?: number;
        thresholds?: number[];
        linear?: boolean;
      } = {},
    ) => {
      const res = await safeInvoke("wavelet_denoise_cmd", {
        path,
        outputDir,
        numScales: options.numScales ?? 5,
        thresholds: options.thresholds ?? [3.0, 2.5, 2.0, 1.5, 1.0],
        linear: options.linear ?? true,
      });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    runPipeline: async (inputPath: string, outputDir = DEFAULT_OUTPUT_DIR, frameStep = 5) => {
      const res = await safeInvoke("run_pipeline_cmd", { inputPath, outputDir, frameStep });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      if (res.collapsed_path) res.collapsedPreviewUrl = await getPreviewUrl(res.collapsed_path);
      return res;
    },

    getConfig: () => safeInvoke("get_config"),
    updateConfig: (field: string, value: any) => safeInvoke("update_config", { field, value }),
    saveApiKey: (key: string, service?: string) => safeInvoke("save_api_key", { key, service }),
    getApiKey: () => safeInvoke("get_api_key"),
  }), []);
}
