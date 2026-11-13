import type { PhotometryStatus } from "../shared/types/photometryStatus";
import { pixelScaleText } from "./coordFormat";

export type StatusTone = "ok" | "muted" | "warn";

export interface StatusChip {
  key: "unit" | "errors" | "flags" | "scale" | "filter";
  text: string;
  tone: StatusTone;
  title: string | null;
}

export interface StatusLineModel {
  calibrated: boolean;
  calibration: string;
  chips: StatusChip[];
  warnings: string[];
}

const NO_ERROR_LAYER_TITLE =
  "No error layer was found for this image. The flux error comes from the scatter of the sky annulus, plus the photon term when a gain is given.";
const NO_WCS_TITLE = "No celestial WCS: radii in arcsec and sky positions are not available.";

function calibrationSentence(status: PhotometryStatus): string {
  if (status.photcal !== null) return status.calibration_origin ?? status.photcal.label;
  return status.missing_reason === null ? "none" : `none: ${status.missing_reason}`;
}

function unitChip({ bunit, summed_unit }: PhotometryStatus): StatusChip {
  if (bunit === null) return { key: "unit", text: "unit not stated", tone: "muted", title: null };
  const title = summed_unit !== null && summed_unit !== bunit ? `An aperture sum is in ${summed_unit}` : null;
  return { key: "unit", text: `unit ${bunit}`, tone: "ok", title };
}

function errorsChip({ err }: PhotometryStatus): StatusChip {
  if (err === null) return { key: "errors", text: "errors: sky scatter only", tone: "muted", title: NO_ERROR_LAYER_TITLE };
  return { key: "errors", text: `errors: ${err.origin}`, tone: "ok", title: null };
}

function flagsChip({ dq }: PhotometryStatus): StatusChip {
  if (dq === null) return { key: "flags", text: "flags: none", tone: "muted", title: null };
  if (dq.exclusion_available) return { key: "flags", text: `flags: ${dq.source}`, tone: "ok", title: null };
  return { key: "flags", text: `flags: ${dq.source} (exclusion not available)`, tone: "warn", title: dq.unavailable_reason };
}

function scaleChip({ pixel_scale_arcsec }: PhotometryStatus): StatusChip {
  const text = pixelScaleText(pixel_scale_arcsec);
  if (text === null) return { key: "scale", text: "no pixel scale", tone: "muted", title: NO_WCS_TITLE };
  return { key: "scale", text, tone: "ok", title: null };
}

function filterChip({ filter }: PhotometryStatus): StatusChip[] {
  if (filter === null) return [];
  const origin = `${filter.dataset} ${filter.filter}, pivot ${filter.pivot_um} um, from the ${filter.source}`;
  return [
    {
      key: "filter",
      text: `${filter.filter} ${filter.pivot_um.toFixed(3)} um`,
      tone: filter.warning === null ? "ok" : "warn",
      title: filter.warning === null ? origin : `${origin}. ${filter.warning}`,
    },
  ];
}

export function statusLineModel(status: PhotometryStatus): StatusLineModel {
  return {
    calibrated: status.photcal !== null,
    calibration: calibrationSentence(status),
    chips: [unitChip(status), errorsChip(status), flagsChip(status), scaleChip(status), ...filterChip(status)],
    warnings: status.warnings,
  };
}
