export interface WcsInfo {
  center_ra: number;
  center_dec: number;
  pixel_scale_arcsec: number;
  fov_arcmin: [number, number];
  field_of_view_w_arcmin: number;
  field_of_view_h_arcmin: number;
  naxis1: number;
  naxis2: number;
}

export type SkyFrame = "icrs" | "fk5" | "galactic" | "ecliptic";

export interface PixelToWorldResult {
  points: ([number, number] | null)[];
  frame: SkyFrame;
}

export type GridKind = "lon" | "lat";
export type GridEdge = "left" | "right" | "top" | "bottom";

export interface GridLine {
  kind: GridKind;
  value_deg: number;
  points: [number, number][];
  label: string;
}

export interface GridLabel {
  x: number;
  y: number;
  text: string;
  edge: GridEdge;
  kind: GridKind;
}

export interface WcsGrid {
  frame: SkyFrame;
  lines: GridLine[];
  labels: GridLabel[];
  lon_step_deg: number;
  lat_step_deg: number;
  notes: string[];
}

export interface WcsGridError {
  error: string;
}

export type WcsGridResult = WcsGrid | WcsGridError;

export interface PointingOverlapFile {
  path: string;
  has_wcs: boolean;
  error?: string;
  center_ra?: number;
  center_dec?: number;
  fov_w_arcmin?: number;
  fov_h_arcmin?: number;
}

export interface PointingOverlapPair {
  a: string;
  b: string;
  status: "disjoint" | "unknown";
  fraction: number | null;
  separation_arcmin: number | null;
}

export interface PointingOverlapResult {
  files: PointingOverlapFile[];
  pairs: PointingOverlapPair[];
  any_disjoint: boolean;
  threshold: number;
}

export interface PlateSolveOptions {
  apiKey?: string;
  scaleLower?: number;
  scaleUpper?: number;
  scaleUnits?: string;
  downsampleFactor?: number;
  centerRa?: number;
  centerDec?: number;
  radius?: number;
}
