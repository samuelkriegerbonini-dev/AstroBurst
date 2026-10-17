import type { DisplaySettings } from "./display";

export type DisplayHint = Partial<Pick<DisplaySettings, "colormap" | "invert" | "symmetric" | "centre" | "stretch" | "limits">>;

export type ProcessedKind = "processing" | "pixelmath" | "debayer" | "stacking" | "cube" | "wizard";

export type ChainStep = "background" | "denoise" | "deconv" | "stretch" | "maskedStretch" | "localContrast" | "tone" | "pixelMath";

export interface ChainEntry {
  fitsPath: string;
  previewUrl: string | null;
  dimensions: [number, number] | null;
  psfUsed?: boolean;
}

export interface ChainBase {
  path: string;
  label: string;
  previewUrl: string | null;
  dimensions: [number, number] | null;
  kind: ProcessedKind;
  inputPath: string;
}

export interface ProcessingChain {
  steps: Partial<Record<ChainStep, ChainEntry>>;
  psfKernel: number[][] | null;
  base: ChainBase | null;
}

export interface ProcessedResult {
  fitsPath: string | null;
  previewUrl: string | null;
  dimensions: [number, number] | null;
  label: string;
  kind: ProcessedKind;
  inputPath: string;
  inputPreviewUrl?: string | null;
  frameIndex?: number;
  displayHint?: DisplayHint;
}

export interface FileRenderState {
  processed: ProcessedResult | null;
  chain: ProcessingChain;
  version: number;
  historyId: string | null;
}
