const isTauri = (): boolean => !!window.__TAURI_INTERNALS__;

const DEFAULT_OUTPUT_DIR = "./output";

async function getPreviewUrl(path: string): Promise<string> {
  if (!path) return "";
  if (isTauri()) {
    const { convertFileSrc } = await import("@tauri-apps/api/core");
    let cleanPath = path.startsWith("\\\\?\\") ? path.slice(4) : path;
    const url = convertFileSrc(cleanPath);
    return `${url}?t=${Date.now()}`;
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

export function useBackend() {
  const safeInvoke = async (command: string, args: Record<string, any> = {}): Promise<any> => {
    if (isTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke(command, args);
    }
    throw new Error(`Command "${command}" requires Tauri desktop environment.`);
  };

  return {
    processFits: async (path: string, outputDir = DEFAULT_OUTPUT_DIR) => {
      const res = await safeInvoke("process_fits", { path, outputDir });
      if (res.png_path) res.previewUrl = await getPreviewUrl(res.png_path);
      return res;
    },

    processBatch: async (paths: string[], outputDir = DEFAULT_OUTPUT_DIR) => {
      const res = await safeInvoke("process_batch", { paths, outputDir });
      if (res.results) {
        for (const r of res.results) {
          if (r.png_path) r.previewUrl = await getPreviewUrl(r.png_path);
        }
      }
      return res;
    },

    getRawPixels: (path: string) => safeInvoke("get_raw_pixels", { path }),

    getRawPixelsBinary: async (path: string) => {
      const buffer = await safeInvoke("get_raw_pixels_binary", { path });
      return parseRawPixelBuffer(buffer);
    },

    exportFits: (path: string, outputPath: string, options: Record<string, any> = {}) =>
        safeInvoke("export_fits", { path, outputPath, ...options }),

    exportFitsRgb: (
        rPath: string | null,
        gPath: string | null,
        bPath: string | null,
        outputPath: string,
        options: Record<string, any> = {},
    ) => safeInvoke("export_fits_rgb", { rPath, gPath, bPath, outputPath, ...options }),

    getHeader: (path: string) => safeInvoke("get_header", { path }),

    getFullHeader: (path: string) => safeInvoke("get_full_header", { path }),

    detectNarrowbandFilters: (paths: string[]) =>
        safeInvoke("detect_narrowband_filters", { paths }),

    computeHistogram: (path: string) => safeInvoke("compute_histogram", { path }),

    computeFftSpectrum: (path: string) => safeInvoke("compute_fft_spectrum", { path }),

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

    getCubeFrame: (path: string, frameIndex: number, outputPath: string) =>
        safeInvoke("get_cube_frame", { path, frameIndex, outputPath }),

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

    runPipeline: async (inputPath: string, outputDir = DEFAULT_OUTPUT_DIR, frameStep = 5) => {
      return safeInvoke("run_pipeline_cmd", { inputPath, outputDir, frameStep });
    },

    getConfig: () => safeInvoke("get_config"),
    updateConfig: (field: string, value: any) => safeInvoke("update_config", { field, value }),
    saveApiKey: (key: string, service?: string) => safeInvoke("save_api_key", { key, service }),
    getApiKey: () => safeInvoke("get_api_key"),
  };
}