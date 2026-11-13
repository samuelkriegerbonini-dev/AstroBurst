import type { ImageDepth } from "../shared/types/depth";
import type { ResolutionCheck } from "../shared/types/resolution";
import { summedPixelUnit } from "./csvProvenance";
import { finiteOrNull, type MeasurementLogDraft, type MeasurementProvenance } from "./measurementLog";
import { VERDICT_LABEL, type ResolutionContext } from "./resolutionText";

const MICROJANSKY_PER_JANSKY = 1e6;
const AB_UNIT = "AB mag";
const UNKNOWN_SUM_UNIT = "the image unit x pix";

export function imageDepthEntry(prov: MeasurementProvenance, depth: ImageDepth): MeasurementLogDraft {
  const sumUnit = summedPixelUnit(depth.bunit);
  const limitJy = finiteOrNull(depth.limit_jy);
  return {
    kind: "image_depth",
    file: prov.file,
    image: prov.image,
    dq: depth.dq_source !== null ? "excluded" : "off",
    unit: depth.limit_ab !== null ? AB_UNIT : sumUnit,
    source: depth.definition,
    params: {
      radius_arcsec: finiteOrNull(depth.radius_arcsec),
      radius_px: finiteOrNull(depth.radius_px),
      n_requested: finiteOrNull(depth.n_requested),
      n_sigma: finiteOrNull(depth.n_sigma),
      seed: finiteOrNull(depth.seed),
      detect_nsigma: finiteOrNull(depth.detect_nsigma),
      psf_fwhm_arcsec: finiteOrNull(depth.psf_fwhm_arcsec),
      calibration: depth.calibration_origin,
      error_layer: depth.err_origin,
      mask_layer: depth.dq_source,
    },
    values: {
      limit_ab: finiteOrNull(depth.limit_ab),
      limit_ujy: limitJy === null ? null : limitJy * MICROJANSKY_PER_JANSKY,
      limit_native: finiteOrNull(depth.limit_native),
      sigma_empty_native: finiteOrNull(depth.sigma_empty_native),
      sigma_clipped_native: finiteOrNull(depth.sigma_clipped_native),
      sigma_formal_native: finiteOrNull(depth.sigma_formal_native),
      f_corr: finiteOrNull(depth.f_corr),
      sigma_empty_rel_err: finiteOrNull(depth.sigma_empty_rel_err),
      n_used: finiteOrNull(depth.n_used),
      n_placed: finiteOrNull(depth.n_placed),
      masked_fraction: finiteOrNull(depth.masked_fraction),
      psf_limit_formal_ab: finiteOrNull(depth.psf_limit_formal_ab),
      psf_limit_formal_native: finiteOrNull(depth.psf_limit_formal_native),
      sigma_pix_median_native: finiteOrNull(depth.sigma_pix_median_native),
      sigma_pix_spread: finiteOrNull(depth.sigma_pix_spread),
      pixel_scale_arcsec: finiteOrNull(depth.pixel_scale_arcsec),
    },
    notes: [
      `limit_native, the sigma values and psf_limit_formal_native are sums in ${sumUnit ?? UNKNOWN_SUM_UNIT}; sigma_pix_median_native is per pixel; limit_ujy is uJy`,
      ...depth.notes,
      ...depth.warnings,
    ],
  };
}

export function resolutionCheckEntry(
  prov: MeasurementProvenance,
  check: ResolutionCheck,
  context: ResolutionContext,
  source: string,
): MeasurementLogDraft {
  const kpcPerArcsec = finiteOrNull(check.kpc_per_arcsec);
  return {
    kind: "resolution_check",
    file: prov.file,
    image: prov.image,
    dq: "off",
    unit: null,
    source,
    params: {
      context,
      psf_fwhm_arcsec: finiteOrNull(check.psf_fwhm_arcsec),
      psf_reference: check.psf_reference,
      snr: finiteOrNull(check.snr),
      redshift: finiteOrNull(check.redshift),
      psf_fwhm_rel_err: finiteOrNull(check.psf_fwhm_rel_err),
      resolved_elements: finiteOrNull(check.resolved_elements),
      min_pixels_per_fwhm: finiteOrNull(check.min_pixels_per_fwhm),
      scale_of_interest_kpc: finiteOrNull(check.scale_of_interest_kpc),
      h0_km_s_mpc: finiteOrNull(check.h0_km_s_mpc),
      omega_m: finiteOrNull(check.omega_m),
    },
    values: {
      verdict: check.verdict === null ? null : VERDICT_LABEL[check.verdict],
      size_basis: check.size_basis,
      intrinsic_size_arcsec: finiteOrNull(check.intrinsic_size_arcsec),
      size_floor_arcsec: finiteOrNull(check.size_floor_arcsec),
      floor_fraction: finiteOrNull(check.floor_fraction),
      resolution_elements: finiteOrNull(check.resolution_elements),
      pixels_per_fwhm: finiteOrNull(check.pixels_per_fwhm),
      nyquist_sampled: check.nyquist_sampled,
      kpc_per_arcsec: kpcPerArcsec,
      psf_fwhm_kpc: finiteOrNull(check.psf_fwhm_kpc),
      intrinsic_size_kpc: finiteOrNull(check.intrinsic_size_kpc),
      size_floor_kpc: finiteOrNull(check.size_floor_kpc),
      scale_of_interest_in_psf: finiteOrNull(check.scale_of_interest_in_psf),
    },
    notes: [
      kpcPerArcsec !== null ? `sizes in arcsec and proper kpc; ${check.cosmology}` : "sizes in arcsec",
      ...check.notes,
      ...check.warnings,
    ],
  };
}
