import type { ResolutionRequest, ResolutionSettings } from "../../shared/types/resolution";
import type { SaguiBand, SaguiInputInfo } from "../../shared/types/sagui";
import { pixelScaleText } from "../../utils/coordFormat";
import { RESOLUTION_LIMITS, resolutionRequest, saguiRegionBudget, significant, type PsfChoice } from "../../utils/resolutionText";
import { isFiniteNumber } from "../../utils/values";

export interface RegionBudgetText {
  text: string;
  tone: "ok" | "warn" | "muted";
}

const NO_PSF: PsfChoice = { fwhmArcsec: null, halfLightRadiusArcsec: null, reference: null, missing: null };

function isPositive(value: number | null | undefined): value is number {
  return isFiniteNumber(value) && value > 0;
}

function asciiLowerCase(text: string): string {
  return text.replace(/[A-Z]/g, (letter) => letter.toLowerCase());
}

function statesPsf(band: SaguiBand): boolean {
  return isPositive(band.psf_fwhm_arcsec) && isPositive(band.pixscale_arcsec);
}

export function bandName(band: SaguiBand): string {
  return `${band.filter} (${band.pivot_um.toPrecision(4)} µm)`;
}

export function cubeRedshift(input: SaguiInputInfo): number | null {
  return isPositive(input.redshift) && input.redshift <= RESOLUTION_LIMITS.redshiftMax ? input.redshift : null;
}

export function targetBand(input: SaguiInputInfo, psfTarget: string): SaguiBand | null {
  const target = asciiLowerCase(psfTarget.trim());
  if (target === "") return null;
  return input.bands.find((band) => asciiLowerCase(band.filter.trim()) === target) ?? null;
}

function bandPsfLine(band: SaguiBand): string {
  if (!statesPsf(band)) return `${bandName(band)} · PSF not stated`;
  const area = isPositive(band.psf_area_px) ? ` · PSF area ${band.psf_area_px.toFixed(1)} px` : "";
  return `${bandName(band)} · PSF FWHM ${significant(band.psf_fwhm_arcsec, 4)} arcsec · ${pixelScaleText(band.pixscale_arcsec)}${area}`;
}

export function bandPsfLines(input: SaguiInputInfo): string[] | null {
  return input.bands.some(statesPsf) ? input.bands.map(bandPsfLine) : null;
}

export function regionBudgetText(input: SaguiInputInfo, psfTarget: string, nRegions: number): RegionBudgetText | null {
  const budget = saguiRegionBudget(input, psfTarget, nRegions);
  if (budget === null) {
    const band = targetBand(input, psfTarget);
    if (band === null || !Number.isInteger(nRegions) || nRegions < 1) return null;
    return {
      tone: "muted",
      text: `The cube does not state the PSF FWHM and pixel scale of ${band.filter} (FILTERS columns PSF_FWHM and PIXSCALE): the region count is compared with the PSF only after the run.`,
    };
  }
  const perRegion = `${nRegions} regions over ${input.width} × ${input.height} pixels give ${budget.pixelsPerRegion.toFixed(1)} pixels per region`;
  const area = `the PSF noise-equivalent area of ${budget.psfAreaPx.toFixed(1)} pixels (${budget.filter})`;
  return budget.enough
    ? {
        tone: "ok",
        text: `${perRegion}, above ${area}. Up to ${budget.maxRegions} regions keep one PSF area each on the full footprint; masks shrink the support, so the run can still warn.`,
      }
    : {
        tone: "warn",
        text: `${perRegion}, below ${area}: regions smaller than the PSF are not resolved. Request at most ${budget.maxRegions} regions, fewer when masks shrink the support.`,
      };
}

export function physicalRequest(input: SaguiInputInfo, psfTarget: string, settings: ResolutionSettings): ResolutionRequest | null {
  const redshift = cubeRedshift(input);
  if (redshift === null) return null;
  const band = targetBand(input, psfTarget);
  const psf: PsfChoice =
    band !== null && statesPsf(band)
      ? { fwhmArcsec: band.psf_fwhm_arcsec ?? null, halfLightRadiusArcsec: null, reference: `PSF_FWHM of ${band.filter} in the cube`, missing: null }
      : NO_PSF;
  const scale = band !== null && isPositive(band.pixscale_arcsec) ? band.pixscale_arcsec : input.bands.find((candidate) => isPositive(candidate.pixscale_arcsec))?.pixscale_arcsec ?? null;
  return resolutionRequest({ ...settings, redshift }, psf, scale, {});
}
