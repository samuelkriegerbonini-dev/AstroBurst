import type { DbeConfig } from "./dbe";
import type { HdrConfig, LheConfig } from "./localContrast";

export type OutputDomain = "linear" | "rescaled" | "display_referred";

export interface PolynomialBackgroundConfig {
  gridSize: number;
  polyDegree: number;
  sigmaClip: number;
  iterations: number;
  mode: "subtract" | "divide";
}

export interface DebandSettings {
  sigmaClip: number;
  iterations: number;
}

export type BackgroundModel =
  | { model: "polynomial"; config: PolynomialBackgroundConfig }
  | { model: "spline"; config: DbeConfig }
  | { model: "deband_rows" | "deband_cols" | "deband_both" | "deband_auto"; config: DebandSettings };

export type PsfSpec =
  | { kind: "gaussian"; sigma: number; size: number }
  | { kind: "empirical"; numStars: number; cutoutRadius: number }
  | { kind: "kernel"; hash: string; rows: number; cols: number };

export interface OpPixelMathSlot {
  name: string;
  ref: string;
}

export interface ToneLevels {
  black: number;
  gamma: number;
  white: number;
}

export interface ToneCurve {
  points: [number, number][];
}

interface Versioned {
  v: 1;
}

export type OpSpec =
  | (Versioned & { op: "background" } & BackgroundModel)
  | (Versioned & { op: "wavelet_denoise"; numScales: number; thresholds: number[]; linearDenoise: boolean; layerBias: number[] | null })
  | (Versioned & {
      op: "rl_deconv";
      iterations: number;
      regularization: number;
      deringing: boolean;
      deringingThreshold: number;
      psf: PsfSpec;
    })
  | (Versioned & { op: "arcsinh"; factor: number })
  | (Versioned & { op: "ghs"; d: number; b: number; sp: number; lp: number; hp: number })
  | (Versioned & {
      op: "masked_stretch";
      iterations: number;
      targetBackground: number;
      maskGrowth: number;
      maskSoftness: number;
      detectionSigma: number;
      maxEccentricity: number;
      luminanceProtect: boolean;
      luminanceCeiling: number;
      protectionAmount: number;
      convergenceThreshold: number;
    })
  | (Versioned & { op: "lhe" } & LheConfig)
  | (Versioned & { op: "hdrmt" } & HdrConfig)
  | (Versioned & { op: "tone"; levels?: ToneLevels; curve?: ToneCurve })
  | (Versioned & { op: "pixelmath"; expression: string; slots: OpPixelMathSlot[]; truncate: boolean; rescale: boolean });

export type OpName = OpSpec["op"];

export type MaskRef = string | Record<string, unknown>;

export interface NodeOutput {
  path: string;
  png: string;
  extras: string[];
  dims: [number, number];
  planes: number;
  domain: OutputDomain;
}

export interface HistoryNode {
  id: string;
  parent: string | null;
  op: OpSpec;
  mask: MaskRef | null;
  output: NodeOutput;
  at: string;
  elapsed_ms: number;
  app: string;
  threads: number;
  present: boolean;
  bytes: number;
  checkpoint: boolean;
  replayable?: boolean;
}

export interface FileFingerprint {
  ref: string;
  size: number;
  mtime: string | null;
}

export interface HistoryImage extends FileFingerprint {
  id: string;
  base?: FileFingerprint;
}

export interface HistoryDoc {
  format: "astroburst-history";
  version: number;
  image: HistoryImage;
  image_key: string;
  base_path: string | null;
  head: string | null;
  last_issued?: number;
  policy: { k: number };
  app: string;
  threads: number;
  masks: Record<string, unknown>;
  nodes: HistoryNode[];
}

export interface HistoryScope {
  imageKey: string;
  basePath: string | null;
}

export interface HistoryState {
  image_id: string;
  history_dir: string;
  root_path: string;
  doc: HistoryDoc | null;
  read_only: string | null;
  fingerprint_matches: boolean | null;
  busy: boolean;
}

export interface NodeView {
  id: string;
  parent: string | null;
  op: OpSpec;
  fits_path: string;
  png_path: string;
  extras: string[];
  dimensions: [number, number];
  planes: number;
  domain: OutputDomain;
  present: boolean;
  previewUrl?: string;
}

export interface HeadView {
  image_id: string;
  history_dir: string;
  head: string | null;
  node: NodeView | null;
  root_path: string;
  can_undo: boolean;
  can_redo: boolean;
  node_count: number;
}

export interface RerunResult extends HeadView {
  created: string[];
  run: RunOpResult;
}

export interface DeleteBranchResult extends HeadView {
  removed: string[];
}

export interface StepCopyResult {
  node: string;
  fits_path: string;
}

export interface RehydrateResult {
  image_id: string;
  head: string | null;
  node: NodeView | null;
  rebuilt: string[];
}

export interface RunOpResult {
  node: string;
  parent: string | null;
  image_id: string;
  history_dir: string;
  fits_path: string;
  png_path: string;
  model_png: string | null;
  dimensions: [number, number];
  is_rgb: boolean;
  domain: OutputDomain;
  elapsed_ms: number;
  previewUrl?: string;
  modelPreviewUrl?: string;
  [extra: string]: unknown;
}

export interface NodeEstimate {
  next_node_bytes: number;
  dimensions: [number, number];
  planes: number;
  history_bytes: number;
  history_files: number;
  history_root: string;
}

export interface KernelRef {
  hash: string;
  rows: number;
  cols: number;
}
