import type { ForcedPhotometryRequest, ForcedPhotometryResult } from "../shared/types/forcedPhotometry";
import { bandSlugs, isLimit, toUjy } from "./forcedPhotometry";
import { dqHandlingOf, type LogValue, type MeasurementLogDraft } from "./measurementLog";
import type { SedPoint } from "./sedAxis";

export const FORCED_LOG_UNIT_NOTE =
  "flux_ujy, flux_err_ujy, upper_limit_ujy in uJy: aperture fluxes, not corrected to total (multiply by aperture_correction); flux_err_ujy includes f_corr where a band has one; mag_ab in AB mag; n_ values are counts; aperture_radius_px in px; ujy_per_unit in uJy per image unit; aperture_correction and f_corr are factors";

const LIST_SEPARATOR = "; ";

export interface SedLogInput {
  file: string | null;
  image: string;
  label: string;
  unit: string | null;
  points: readonly SedPoint[];
  savedPath: string | null;
}

export function forcedPhotometryEntry(
  request: ForcedPhotometryRequest,
  result: ForcedPhotometryResult,
  selected: number | null,
): MeasurementLogDraft {
  const slugs = bandSlugs(result.bands);
  const source = selected !== null ? result.sources[selected] : undefined;
  const nPositions = result.sources.length;
  const aperture = request.aperture;
  const annulus = aperture.annulus_arcsec ?? aperture.annulus_px ?? null;
  const annulusUnit = aperture.annulus_arcsec ? "arcsec" : aperture.annulus_px ? "px" : null;
  const anyMasked = result.sources.some((s) => s.cells.some((c) => c !== null && c.n_masked > 0));
  const sourceValues: Record<string, LogValue> = {};
  if (source) {
    result.bands.forEach((_, i) => {
      const cell = source.cells[i];
      sourceValues[`flux_ujy_${slugs[i]}`] = toUjy(cell?.flux_jy);
      sourceValues[`flux_err_ujy_${slugs[i]}`] = toUjy(cell?.flux_err_corr_jy ?? cell?.flux_err_jy);
      sourceValues[`upper_limit_ujy_${slugs[i]}`] = toUjy(cell?.upper_limit_jy);
      sourceValues[`mag_ab_${slugs[i]}`] = cell?.mag_ab ?? null;
    });
  }
  const bandValues: Record<string, LogValue> = {};
  result.bands.forEach((band, i) => {
    bandValues[`n_measured_${slugs[i]}`] = band.n_measured;
    bandValues[`n_limits_${slugs[i]}`] = result.sources.filter((s) => isLimit(s.cells[i] ?? null)).length;
    bandValues[`n_failed_${slugs[i]}`] = band.n_failed;
    bandValues[`aperture_radius_px_${slugs[i]}`] = band.aperture_radius_px;
    bandValues[`aperture_correction_${slugs[i]}`] = band.aperture_correction?.factor ?? null;
    bandValues[`f_corr_${slugs[i]}`] = band.f_corr;
    bandValues[`ujy_per_unit_${slugs[i]}`] = toUjy(band.jy_per_native_unit);
  });
  return {
    kind: "forced_photometry",
    file: result.bands.map((band) => band.file_name).join(LIST_SEPARATOR),
    image: "original",
    dq: dqHandlingOf(request.options?.exclude_dq === true, anyMasked),
    unit: "uJy",
    source: source
      ? JSON.stringify({ kind: "sky_position", id: source.id, ra_deg: source.ra_deg, dec_deg: source.dec_deg, n_positions: nPositions })
      : JSON.stringify({ kind: "sky_positions", n_positions: nPositions }),
    params: {
      aperture_radius_arcsec: aperture.radius_arcsec ?? null,
      aperture_radius_px: aperture.radius_px ?? null,
      annulus_inner: annulus ? annulus[0] : null,
      annulus_outer: annulus ? annulus[1] : null,
      annulus_unit: annulusUnit,
      correction: aperture.correction,
      reference_radius_arcsec: result.aperture.reference_radius_arcsec ?? null,
      limit_sigma: result.limit_sigma,
      gain: request.options?.gain ?? null,
      n_bands: result.bands.length,
      bands: result.bands.map((band) => band.label).join(LIST_SEPARATOR),
    },
    values: { ...sourceValues, n_positions: nPositions, ...bandValues },
    notes: [
      FORCED_LOG_UNIT_NOTE,
      ...result.bands.flatMap((band) => (band.error !== null ? [`${band.label}: not measured: ${band.error}`] : [])),
      ...result.warnings,
    ],
  };
}

function filterColumnNames(points: readonly SedPoint[]): string[] {
  const used = new Set<string>();
  return points.map((point) => {
    const name = point.filter.replace(/[^A-Za-z0-9_]/g, "_");
    let candidate = name;
    for (let n = 2; used.has(candidate); n++) candidate = `${name}_${n}`;
    used.add(candidate);
    return candidate;
  });
}

export function sedEntry(input: SedLogInput): MeasurementLogDraft {
  const names = filterColumnNames(input.points);
  const values: Record<string, LogValue> = {};
  input.points.forEach((point, i) => {
    values[`flux_${names[i]}`] = point.value;
    values[`flux_err_${names[i]}`] = point.err;
    values[`limit_${names[i]}`] = point.limit === true;
  });
  return {
    kind: "sed",
    file: input.file,
    image: input.image,
    dq: "not_applied",
    unit: input.unit,
    source: JSON.stringify({ kind: "sed", label: input.label }),
    params: { n_points: input.points.length, saved_path: input.savedPath },
    values,
    notes: input.unit === null ? [] : [`flux and flux_err in ${input.unit}; limit true means the flux is an upper limit`],
  };
}
