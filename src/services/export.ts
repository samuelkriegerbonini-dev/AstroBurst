import { typedInvoke } from "../infrastructure/tauri";

export const DEFAULT_QUANTIZE_LEVEL = 16;

export type FitsCompression = "rice" | "none";

export const RICE_SUPPORTED_BITPIX: readonly number[] = [16, -32];

export function riceSupportsBitpix(bitpix: number): boolean {
  return RICE_SUPPORTED_BITPIX.includes(bitpix);
}

export interface ExportFitsOptions {
  applyStfStretch?: boolean;
  shadow?: number;
  midtone?: number;
  highlight?: number;
  copyWcs?: boolean;
  copyMetadata?: boolean;
  bitpix?: number;
  compress?: FitsCompression;
  quantizeLevel?: number;
}

export interface ExportFitsRgbOptions {
  copyWcs?: boolean;
  copyMetadata?: boolean;
  bitpix?: number;
  history?: string[];
  compress?: FitsCompression;
  quantizeLevel?: number;
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

export interface ExportResult {
  output_path: string;
  elapsed_ms: number;
  file_size_bytes?: number;
  bitpix?: number;
  compress?: string;
  quantize_level?: number;
  channels?: Array<{ path: string; channel: string }>;
}

export interface CompressMefOptions {
  lossless?: boolean;
  quantizeLevel?: number | null;
  dropExtnames?: string[];
  rawExtnames?: string[];
}

export interface CompressMefResult {
  output_path: string;
  dropped: string[];
  kept_raw: string[];
  source_size_bytes: number;
  output_size_bytes: number;
  elapsed_ms: number;
}

export function parseExtnameList(text: string): string[] {
  return text
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

export function compressMef(
  sourcePath: string,
  outputPath: string,
  options: CompressMefOptions = {},
): Promise<CompressMefResult> {
  const lossless = options.lossless ?? true;
  return typedInvoke<CompressMefResult>("compress_mef_cmd", {
    sourcePath,
    outputPath,
    lossless,
    quantizeLevel: lossless ? null : options.quantizeLevel ?? DEFAULT_QUANTIZE_LEVEL,
    dropExtnames: options.dropExtnames ?? [],
    rawExtnames: options.rawExtnames ?? [],
  });
}

export function exportPng(
  path: string,
  outputPath: string,
  options: ExportPngOptions = {},
): Promise<ExportResult> {
  return typedInvoke<ExportResult>("export_png", {
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
): Promise<ExportResult> {
  return typedInvoke<ExportResult>("export_rgb_png", {
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
): Promise<ExportResult> {
  return typedInvoke<ExportResult>("export_aligned_channels_cmd", {
    rPath,
    gPath,
    bPath,
    outputDir,
    alignMethod: options.alignMethod ?? "phase_correlation",
    copyWcs: options.copyWcs ?? true,
    copyMetadata: options.copyMetadata ?? true,
  });
}

export function exportFits(
  path: string,
  outputPath: string,
  options: ExportFitsOptions = {},
): Promise<ExportResult> {
  return typedInvoke<ExportResult>("export_fits", {
    path,
    outputPath,
    applyStfStretch: options.applyStfStretch ?? false,
    shadow: options.shadow,
    midtone: options.midtone,
    highlight: options.highlight,
    copyWcs: options.copyWcs ?? true,
    copyMetadata: options.copyMetadata ?? true,
    bitpix: options.bitpix,
    compress: options.compress ?? "none",
    quantizeLevel: options.quantizeLevel ?? DEFAULT_QUANTIZE_LEVEL,
  });
}

export function exportFitsRgb(
  rPath: string | null,
  gPath: string | null,
  bPath: string | null,
  outputPath: string,
  options: ExportFitsRgbOptions = {},
): Promise<ExportResult> {
  return typedInvoke<ExportResult>("export_fits_rgb", {
    rPath,
    gPath,
    bPath,
    outputPath,
    copyWcs: options.copyWcs ?? true,
    copyMetadata: options.copyMetadata ?? true,
    bitpix: options.bitpix ?? -32,
    history: options.history,
    compress: options.compress ?? "none",
    quantizeLevel: options.quantizeLevel ?? DEFAULT_QUANTIZE_LEVEL,
  });
}
