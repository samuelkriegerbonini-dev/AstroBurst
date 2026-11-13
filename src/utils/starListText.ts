export const STAR_FWHM_HEADER = "FWHM (px)";
export const STAR_FWHM_CARD_LABEL = "Median FWHM";
export const STAR_FWHM_METHOD =
  "FWHM from the second moment of each star's core (the pixels at or above half of its peak, at least its 9 brightest), corrected for the cut as for a Gaussian. A moment estimate, not a profile fit.";
export const STAR_FWHM_CARD_TITLE = `Median over the listed stars. ${STAR_FWHM_METHOD}`;
export const NO_STAR_FWHM = "--";

export function medianStarFwhm(stars: readonly { fwhm: number }[]): number | null {
  const sorted = stars
    .map((s) => s.fwhm)
    .filter((fwhm) => Number.isFinite(fwhm) && fwhm > 0)
    .sort((a, b) => a - b);
  if (sorted.length === 0) return null;
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

export function starFwhmText(fwhmPx: number | null, arcsecPerPx: number | null | undefined): string {
  if (fwhmPx === null) return NO_STAR_FWHM;
  const pixels = `${fwhmPx.toFixed(2)} px`;
  if (arcsecPerPx == null || !Number.isFinite(arcsecPerPx) || arcsecPerPx <= 0) return pixels;
  return `${pixels} (${(fwhmPx * arcsecPerPx).toFixed(3)}")`;
}
