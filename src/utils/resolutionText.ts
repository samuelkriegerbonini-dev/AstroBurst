import type {
  ImagePsfFwhm,
  ResolutionCheck,
  ResolutionRequest,
  ResolutionSettings,
  ResolutionVerdict,
  SeeingCard,
} from "../shared/types/resolution";
import type { SaguiInputInfo } from "../shared/types/sagui";
import { MAD_TO_SIGMA } from "./statisticsUnits";
import { finiteOrNull, isFiniteNumber } from "./values";

export type ResolutionContext = "photometry" | "profile" | "forced" | "manual";

export const DEFAULT_RESOLUTION_SETTINGS: ResolutionSettings = {
  redshift: null,
  h0: 67.66,
  omegaM: 0.30966,
  psfSource: "image",
  psfFwhmArcsec: null,
  eps: 0.03,
  qResolved: 3,
  minPixelsPerFwhm: 2,
  scaleOfInterestKpc: 1,
};

export const RESOLUTION_LIMITS: { h0: [number, number]; redshiftMax: number } = { h0: [20, 200], redshiftMax: 20 };

export const VERDICT_LABEL: Record<ResolutionVerdict, string> = {
  unresolved: "Unresolved",
  marginal: "Marginally resolved",
  resolved: "Resolved",
};

export const VERDICT_SCIENCE: Record<ResolutionVerdict, string> = {
  unresolved: "Total flux, colours and the integrated SED are meaningful; the size is an upper limit only.",
  marginal: "Integrated flux, one size and perhaps an axis ratio; no internal structure, no profile, no clumps.",
  resolved: "Profiles and regions are meaningful down to the PSF FWHM.",
};

export const CONTEXT_NOTE: Record<ResolutionContext, string> = {
  photometry: "The source FWHM is the Photometry FWHM, the estimator the image PSF is measured with.",
  profile:
    "R50 is the observed half-light radius along the semi-major axis; its total is the flux inside the drawn ellipse, so it moves with the region.",
  forced: "The FWHM was measured at the forced position, without recentring.",
  manual: "Sizes typed by hand: use the same estimator for the source and for the PSF.",
};

export const NO_VERDICT_TEXT = "No source size: only the PSF, the size floor and the scale are given.";

const PLANCK18_NEUTRINO_AND_RADIATION_DENSITY = 0.0014147613306646316 + 7.893312479128647e-5;
const SEEING_CARD_KEYS = ["SEEING", "PSF_FWHM", "PSFFWHM"];
const SEEING_CARD_MAX_ARCSEC = 60;
const MISSING_IMAGE_PSF = "Measure the PSF of this image in the Resolution section.";
const MISSING_TYPED_PSF = "Type a PSF FWHM in the Resolution section.";
const MISSING_IMAGE_WCS = "This image has no celestial WCS, so its PSF is known in pixels only: type the PSF FWHM in arcsec.";
const MISSING_HEADER_CARD = "This file has no SEEING, PSF_FWHM or PSFFWHM card: choose another PSF source.";

function isFiniteValue(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isPositive(value: unknown): value is number {
  return isFiniteValue(value) && value > 0;
}

function positiveOrNull(value: number | null | undefined): number | null {
  return isPositive(value) ? value : null;
}

function memberOr<T>(value: unknown, isValid: (candidate: unknown) => boolean, fallback: T): T {
  return isValid(value) ? (value as T) : fallback;
}

export function sanitizeResolutionSettings(raw: unknown): ResolutionSettings {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return { ...DEFAULT_RESOLUTION_SETTINGS };
  const stored = raw as Record<string, unknown>;
  const defaults = DEFAULT_RESOLUTION_SETTINGS;
  const [h0Min, h0Max] = RESOLUTION_LIMITS.h0;
  const settings: ResolutionSettings = {
    redshift: memberOr(
      stored.redshift,
      (value) => value === null || (isPositive(value) && value <= RESOLUTION_LIMITS.redshiftMax),
      defaults.redshift,
    ),
    h0: memberOr(stored.h0, (value) => isFiniteValue(value) && value >= h0Min && value <= h0Max, defaults.h0),
    omegaM: memberOr(stored.omegaM, (value) => isPositive(value) && value < 1, defaults.omegaM),
    psfSource: memberOr(stored.psfSource, (value) => value === "image" || value === "typed" || value === "header", defaults.psfSource),
    psfFwhmArcsec: memberOr(stored.psfFwhmArcsec, (value) => value === null || isPositive(value), defaults.psfFwhmArcsec),
    eps: memberOr(stored.eps, (value) => isFiniteValue(value) && value >= 0 && value < 1, defaults.eps),
    qResolved: memberOr(stored.qResolved, isPositive, defaults.qResolved),
    minPixelsPerFwhm: memberOr(stored.minPixelsPerFwhm, isPositive, defaults.minPixelsPerFwhm),
    scaleOfInterestKpc: memberOr(stored.scaleOfInterestKpc, isPositive, defaults.scaleOfInterestKpc),
  };
  if (settings.omegaM + PLANCK18_NEUTRINO_AND_RADIATION_DENSITY * (defaults.h0 / settings.h0) ** 2 >= 1) {
    settings.omegaM = defaults.omegaM;
  }
  return settings;
}

export interface PsfChoice {
  fwhmArcsec: number | null;
  halfLightRadiusArcsec: number | null;
  reference: string | null;
  missing: string | null;
}

function missingPsf(missing: string): PsfChoice {
  return { fwhmArcsec: null, halfLightRadiusArcsec: null, reference: null, missing };
}

function starCount(nStars: number): string {
  return `${nStars} star${nStars === 1 ? "" : "s"}`;
}

export function psfChoice(
  settings: ResolutionSettings,
  imagePsf: ImagePsfFwhm | null,
  headerCards?: readonly SeeingCard[],
): PsfChoice {
  if (settings.psfSource === "typed") {
    return settings.psfFwhmArcsec === null
      ? missingPsf(MISSING_TYPED_PSF)
      : { fwhmArcsec: settings.psfFwhmArcsec, halfLightRadiusArcsec: null, reference: "typed value", missing: null };
  }
  if (settings.psfSource === "image") {
    if (imagePsf === null) return missingPsf(MISSING_IMAGE_PSF);
    if (imagePsf.fwhm_arcsec === null) return missingPsf(MISSING_IMAGE_WCS);
    return {
      fwhmArcsec: imagePsf.fwhm_arcsec,
      halfLightRadiusArcsec: imagePsf.half_light_radius_arcsec,
      reference: `${starCount(imagePsf.n_stars)} of this image (${imagePsf.model})`,
      missing: null,
    };
  }
  const card = imagePsf !== null && imagePsf.header_values.length > 0 ? imagePsf.header_values[0] : headerCards?.[0];
  if (card === undefined) {
    return missingPsf(imagePsf === null && headerCards === undefined ? MISSING_IMAGE_PSF : MISSING_HEADER_CARD);
  }
  return { fwhmArcsec: card.value_arcsec, halfLightRadiusArcsec: null, reference: `header card ${card.key}`, missing: null };
}

function unquotedCardText(raw: string): string {
  const trimmed = raw.trim();
  const unquoted = trimmed.length >= 2 && trimmed.startsWith("'") && trimmed.endsWith("'") ? trimmed.slice(1, -1) : trimmed;
  return unquoted.trim();
}

export function seeingCardsFromHeader(header: Readonly<Record<string, string>>): SeeingCard[] {
  const cards: SeeingCard[] = [];
  for (const key of SEEING_CARD_KEYS) {
    const raw = header[key];
    if (typeof raw !== "string") continue;
    const text = unquotedCardText(raw);
    const value = Number(text);
    if (text !== "" && isPositive(value) && value <= SEEING_CARD_MAX_ARCSEC) cards.push({ key, value_arcsec: value });
  }
  return cards;
}

export interface ResolutionSource {
  fwhmArcsec?: number | null;
  halfLightRadiusArcsec?: number | null;
  snr?: number | null;
}

export function resolutionRequest(
  settings: ResolutionSettings,
  psf: PsfChoice,
  pixelScaleArcsec: number | null | undefined,
  source: ResolutionSource,
): ResolutionRequest {
  return {
    psf_fwhm_arcsec: psf.fwhmArcsec,
    psf_reference: psf.reference,
    pixel_scale_arcsec: positiveOrNull(pixelScaleArcsec),
    source_fwhm_arcsec: positiveOrNull(source.fwhmArcsec),
    source_half_light_radius_arcsec: positiveOrNull(source.halfLightRadiusArcsec),
    psf_half_light_radius_arcsec: psf.halfLightRadiusArcsec,
    snr: finiteOrNull(source.snr),
    redshift: settings.redshift,
    psf_fwhm_rel_err: settings.eps,
    resolved_elements: settings.qResolved,
    min_pixels_per_fwhm: settings.minPixelsPerFwhm,
    scale_of_interest_kpc: settings.scaleOfInterestKpc,
    h0_km_s_mpc: settings.h0,
    omega_m: settings.omegaM,
  };
}

export function significant(value: number | null | undefined, digits = 3): string {
  return isFiniteNumber(value) ? String(Number(value.toPrecision(digits))) : "--";
}

export function scaleText(check: ResolutionCheck): string | null {
  if (check.kpc_per_arcsec === null) return null;
  const perPixel = check.kpc_per_pixel === null ? "" : ` (${significant(check.kpc_per_pixel)} kpc/px)`;
  return `${significant(check.kpc_per_arcsec)} kpc/arcsec at z = ${check.redshift}${perPixel}`;
}

function arcsecAndKpc(arcsec: number, kpc: number | null): string {
  return `${significant(arcsec)} arcsec${kpc === null ? "" : ` = ${significant(kpc)} kpc`}`;
}

const SIZE_BASIS_TEXT: Record<NonNullable<ResolutionCheck["size_basis"]>, string> = {
  fwhm: " (from the FWHM)",
  half_light_radius: " (from half-light radii)",
};

export function resolutionRows(check: ResolutionCheck): [string, string][] {
  const rows: [string, string][] = [];
  const hasPsf = check.psf_fwhm_arcsec !== null;
  const hasSourceSize = check.size_basis !== null;
  if (check.psf_fwhm_arcsec !== null) rows.push(["PSF FWHM", arcsecAndKpc(check.psf_fwhm_arcsec, check.psf_fwhm_kpc)]);
  if (check.psf_reference !== null) rows.push(["PSF from", check.psf_reference]);
  if (check.pixels_per_fwhm !== null) {
    const below = check.nyquist_sampled === false ? `, below the limit of ${check.min_pixels_per_fwhm}` : "";
    rows.push(["Sampling", `${check.pixels_per_fwhm.toFixed(2)} px per FWHM${below}`]);
  } else if (hasPsf) {
    rows.push(["Sampling", "not checked (no pixel scale)"]);
  }
  if (check.intrinsic_size_arcsec !== null) {
    const basis = check.size_basis === null ? "" : SIZE_BASIS_TEXT[check.size_basis];
    rows.push(["Intrinsic size", `${arcsecAndKpc(check.intrinsic_size_arcsec, check.intrinsic_size_kpc)}${basis}`]);
  }
  if (check.size_floor_arcsec !== null) {
    rows.push([
      "Size floor",
      `${arcsecAndKpc(check.size_floor_arcsec, check.size_floor_kpc)} (${significant(check.floor_fraction)} x PSF FWHM)`,
    ]);
  } else if (hasPsf && hasSourceSize) {
    rows.push(["Size floor", "no size can be measured at this signal-to-noise"]);
  }
  if (check.resolution_elements !== null) {
    rows.push(["Resolution elements", `${check.resolution_elements.toFixed(2)} (resolved from ${check.resolved_elements})`]);
  }
  if (check.snr !== null) rows.push(["Signal-to-noise", check.snr.toFixed(1)]);
  else if (hasSourceSize) rows.push(["Signal-to-noise", "not given"]);
  const scale = scaleText(check);
  if (scale !== null) rows.push(["Scale", scale]);
  if (check.scale_of_interest_in_psf !== null) {
    rows.push([
      "Scale of interest",
      `${check.scale_of_interest_kpc} kpc is ${significant(check.scale_of_interest_in_psf)} x the PSF FWHM`,
    ]);
  }
  if (check.kpc_per_arcsec !== null) rows.push(["Cosmology", check.cosmology]);
  return rows;
}

export function imagePsfText(psf: ImagePsfFwhm): string {
  const arcsec = psf.fwhm_arcsec === null ? "" : ` = ${significant(psf.fwhm_arcsec)} arcsec`;
  const scatter = psf.fwhm_px_scatter === null ? "" : ` ± ${psf.fwhm_px_scatter.toFixed(2)} px`;
  return `${psf.fwhm_px.toFixed(2)} px${arcsec}${scatter}, ${starCount(psf.n_stars)}`;
}

function median(sorted: readonly number[]): number {
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

function ascending(values: number[]): number[] {
  return values.sort((a, b) => a - b);
}

export function robustWidth(
  values: readonly (number | null | undefined)[],
): { median: number; scatter: number | null; n: number } | null {
  const finite = ascending(values.filter(isFiniteNumber));
  if (finite.length === 0) return null;
  const centre = median(finite);
  const scatter = finite.length >= 3 ? MAD_TO_SIGMA * median(ascending(finite.map((value) => Math.abs(value - centre)))) : null;
  return { median: centre, scatter, n: finite.length };
}

export interface RegionBudget {
  filter: string;
  psfFwhmArcsec: number;
  pixscaleArcsec: number;
  psfAreaPx: number;
  pixelsPerRegion: number;
  maxRegions: number;
  enough: boolean;
}

function asciiLowerCase(text: string): string {
  return text.replace(/[A-Z]/g, (letter) => letter.toLowerCase());
}

export function saguiRegionBudget(input: SaguiInputInfo, psfTarget: string, nRegions: number): RegionBudget | null {
  const target = asciiLowerCase(psfTarget.trim());
  const band = input.bands.find((candidate) => asciiLowerCase(candidate.filter) === target);
  if (band === undefined || !Number.isInteger(nRegions) || nRegions < 1) return null;
  const { psf_fwhm_arcsec: psfFwhmArcsec, pixscale_arcsec: pixscaleArcsec, psf_area_px: psfAreaPx } = band;
  if (!isPositive(psfFwhmArcsec) || !isPositive(pixscaleArcsec) || !isPositive(psfAreaPx)) return null;
  const pixels = input.width * input.height;
  const pixelsPerRegion = pixels / nRegions;
  return {
    filter: band.filter,
    psfFwhmArcsec,
    pixscaleArcsec,
    psfAreaPx,
    pixelsPerRegion,
    maxRegions: Math.floor(pixels / psfAreaPx),
    enough: pixelsPerRegion >= psfAreaPx,
  };
}
