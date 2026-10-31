import type { RegionShape, RegionSystem } from "./regions";

export type MarkMode = "nonzero" | "zero" | "nan" | "nonnan" | "range";

export const MARK_MODES: readonly MarkMode[] = ["nonzero", "zero", "nan", "nonnan", "range"];

export interface MaskMods {
  feather_px?: number;
  invert?: boolean;
}

export interface WcsSummary {
  crval: [number, number];
  crpix: [number, number];
  cd: [[number, number], [number, number]];
  naxis: [number, number];
}

export interface StarMaskParams {
  growth_factor?: number;
  softness?: number;
  min_fwhm?: number;
  max_fwhm?: number;
  max_eccentricity?: number;
  luminance_protect?: boolean;
  luminance_ceiling?: number;
}

export type MaskRecipe =
  | { kind: "luminance"; auto_stf?: boolean }
  | { kind: "range"; lo: number; hi: number; fuzz?: number }
  | { kind: "stars"; sigma?: number; config?: StarMaskParams }
  | { kind: "regions"; shapes: RegionShape[]; exclude_shapes?: RegionShape[]; source_wcs?: WcsSummary }
  | { kind: "file"; path: string; mark?: MarkMode; lo?: number | null; hi?: number | null };

export type MaskKind = MaskRecipe["kind"];

export type MaskSpec = MaskRecipe & MaskMods;

export type MaskChoice = string | MaskSpec;

export type MaskShapesSystem = RegionSystem;

export interface MaskCreateOptions {
  shapesSystem?: MaskShapesSystem;
  mods?: MaskMods | null;
}

export interface MaskCreateResult {
  mask_id: string;
  kind: MaskKind;
  coverage: number;
  dims: [number, number];
  stars: number | null;
  recipe: MaskSpec;
}

export interface MaskDeleteResult {
  mask_id: string;
  recipe: MaskSpec;
  used_by: string[];
  kept_for_replay: boolean;
}

export type MaskExportKind = "float" | "label";

export interface MaskExportResult {
  mask_id: string;
  kind: MaskExportKind;
  fits_path: string;
  bitpix: -32 | 16 | 32;
  dims: [number, number];
}

export interface MaskPreview {
  width: number;
  height: number;
  cells: Uint8Array;
}

export type MaskTint = readonly [number, number, number];

export interface MaskStyle {
  opacity: number;
  tint: MaskTint;
  displayInvert: boolean;
}
