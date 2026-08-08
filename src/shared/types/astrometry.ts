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

export interface PixelToWorldResult {
  points: ([number, number] | null)[];
}

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
