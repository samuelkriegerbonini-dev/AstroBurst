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

export interface ExportAlignedOptions {
  alignMethod?: string;
  copyWcs?: boolean;
  copyMetadata?: boolean;
}

export interface ExportPngOptions {
  bitDepth?: number;
  applyStfStretch?: boolean;
  shadow?: number;
  midtone?: number;
  highlight?: number;
}

export interface ExportRgbPngOptions {
  bitDepth?: number;
  applyStfStretch?: boolean;
  shadowR?: number;
  midtoneR?: number;
  highlightR?: number;
  shadowG?: number;
  midtoneG?: number;
  highlightG?: number;
  shadowB?: number;
  midtoneB?: number;
  highlightB?: number;
}

export function exportPng(
  path: string,
  outputPath: string,
  options: ExportPngOptions = {},
) {
  return safeInvoke("export_png", {
    path,
    outputPath,
    bitDepth: options.bitDepth ?? 16,
    applyStfStretch: options.applyStfStretch ?? false,
    shadow: options.shadow,
    midtone: options.midtone,
    highlight: options.highlight,
  });
}

export function exportRgbPng(
  rPath: string | null,
  gPath: string | null,
  bPath: string | null,
  outputPath: string,
  options: ExportRgbPngOptions = {},
) {
  return safeInvoke("export_rgb_png", {
    rPath,
    gPath,
    bPath,
    outputPath,
    bitDepth: options.bitDepth ?? 16,
    applyStfStretch: options.applyStfStretch ?? false,
    shadowR: options.shadowR,
    midtoneR: options.midtoneR,
    highlightR: options.highlightR,
    shadowG: options.shadowG,
    midtoneG: options.midtoneG,
    highlightG: options.highlightG,
    shadowB: options.shadowB,
    midtoneB: options.midtoneB,
    highlightB: options.highlightB,
  });
}

export function exportAlignedChannels(
  rPath: string | null,
  gPath: string | null,
  bPath: string | null,
  outputDir: string,
  options: ExportAlignedOptions = {},
) {
  return safeInvoke("export_aligned_channels_cmd", {
    rPath,
    gPath,
    bPath,
    outputDir,
    alignMethod: options.alignMethod ?? "phase_correlation",
    copyWcs: options.copyWcs ?? true,
    copyMetadata: options.copyMetadata ?? true,
  });
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
