import type { PhotCal } from "../../services/analysis";

export interface PhotometryStatusErr {
  origin: string;
}

export interface PhotometryStatusDq {
  source: string;
  exclusion_available: boolean;
  unavailable_reason: string | null;
}

export interface PhotometryStatusFilter {
  dataset: string;
  filter: string;
  pivot_um: number;
  source: string;
  warning: string | null;
}

export interface PhotometryStatus {
  photcal: PhotCal | null;
  calibration_origin: string | null;
  calibration_source?: "header" | "user" | null;
  missing_reason: string | null;
  bunit: string | null;
  summed_unit: string | null;
  err: PhotometryStatusErr | null;
  dq: PhotometryStatusDq | null;
  pixel_scale_arcsec: number | null;
  pixel_scale_axes_arcsec: [number, number] | null;
  filter: PhotometryStatusFilter | null;
  warnings: string[];
}
