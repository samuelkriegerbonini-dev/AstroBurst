import type { NoiseFactorOffer } from "../../utils/forcedPhotometryRequest";

export function noiseOfferText(offer: NoiseFactorOffer, fCorr: number | null): string | null {
  if (offer.kind === "none") return null;
  const at = `r = ${String(Number(offer.radius.toPrecision(4)))} ${offer.unit}`;
  if (offer.kind === "usable") {
    return fCorr === offer.fCorr
      ? `Noise factor from the Depth section (${at}).`
      : `Depth section: f_corr ${offer.fCorr.toFixed(2)} for this image at ${at}.`;
  }
  const measured = `The Depth section measured this image at ${at}, not with this aperture`;
  return fCorr === null
    ? `${measured}: its f_corr does not apply. Measure the depth again with this radius to get one.`
    : `${measured}. Check that the noise factor in the field belongs to this aperture.`;
}
