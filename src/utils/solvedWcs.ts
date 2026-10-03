import type { WriteSolvedWcsResult } from "../shared/types/astrometry";

export const SOLVED_WCS_LABEL = "Plate-solved WCS";
export const NO_SOLVE_YET = "Plate solve the image first.";
export const NO_WCS_FILE = "astrometry.net returned no WCS file for this job; solve again to write the WCS.";

export function wcsWriteBlocker(result: { wcs_cards?: readonly (readonly [string, string])[] } | null): string | null {
  if (!result) return NO_SOLVE_YET;
  if (!result.wcs_cards || result.wcs_cards.length === 0) return NO_WCS_FILE;
  return null;
}

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

export function solvedWcsSummary(result: WriteSolvedWcsResult): string {
  const centre = `${result.center_ra.toFixed(5)}, ${result.center_dec.toFixed(5)} deg`;
  const scale = `${result.pixel_scale_arcsec.toPrecision(4)}"/px`;
  return `Wrote ${baseName(result.fits_path)}: centre ${centre}, ${scale}${result.sip_present ? ", SIP" : ""}`;
}
