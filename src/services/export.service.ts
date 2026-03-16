import { safeInvoke } from "../infrastructure/tauri";

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

export function exportFits(path: string, outputPath: string, options: ExportFitsOptions = {}) {
  return safeInvoke("export_fits", {
    path,
    outputPath,
    applyStfStretch: options.applyStfStretch ?? false,
    shadow: options.shadow,
    midtone: options.midtone,
    highlight: options.highlight,
    copyWcs: options.copyWcs ?? true,
    copyMetadata: options.copyMetadata ?? true,
    bitpix: options.bitpix,
  });
}

export function exportFitsRgb(
  rPath: string | null,
  gPath: string | null,
  bPath: string | null,
  outputPath: string,
  options: ExportFitsRgbOptions = {},
) {
  return safeInvoke("export_fits_rgb", {
    rPath,
    gPath,
    bPath,
    outputPath,
    copyWcs: options.copyWcs ?? true,
    copyMetadata: options.copyMetadata ?? true,
  });
}
