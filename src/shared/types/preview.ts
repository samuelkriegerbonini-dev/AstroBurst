export type ProcessedKind = "processing" | "pixelmath" | "debayer" | "stacking" | "cube" | "wizard";

export type ChainStep = "background" | "denoise" | "deconv" | "stretch" | "maskedStretch" | "localContrast" | "pixelMath";

export interface ChainEntry {
  fitsPath: string;
  previewUrl: string | null;
  dimensions: [number, number] | null;
}

export interface ProcessingChain {
  steps: Partial<Record<ChainStep, ChainEntry>>;
  psfKernel: number[][] | null;
}

export interface ProcessedResult {
  fitsPath: string | null;
  previewUrl: string | null;
  dimensions: [number, number] | null;
  label: string;
  kind: ProcessedKind;
  inputPath: string;
}

export interface FileRenderState {
  processed: ProcessedResult | null;
  chain: ProcessingChain;
  version: number;
}
