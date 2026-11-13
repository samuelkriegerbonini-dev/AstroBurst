import type { ForcedBand } from "../../shared/types/forcedPhotometry";
import { formatSignificant } from "../../utils/formatSignificant";

export const MISSING = "--";

const n = (value: number | null | undefined): string =>
  typeof value === "number" && Number.isFinite(value) ? formatSignificant(value, 4) : MISSING;

const counted = (count: number, word: string): string => `${count} ${word}${count === 1 ? "" : "s"}`;

function filterText(band: ForcedBand): string {
  const filter = band.filter;
  if (filter === null) return "not identified: the band stays off the SED";
  return `${filter.dataset} ${filter.filter}, pivot ${n(filter.pivot_um)} um (${filter.source})`;
}

function apertureText(band: ForcedBand): string {
  const annulus = band.annulus_px;
  const sky = annulus === null ? ", no local sky" : `, sky annulus ${n(annulus[0])} to ${n(annulus[1])} px`;
  return `${n(band.aperture_radius_px)} px radius${sky}`;
}

function calibrationText(band: ForcedBand): string {
  const unit = band.bunit ?? "image unit";
  const origin = band.calibration_origin;
  if (band.jy_per_native_unit !== null) return `${origin ?? MISSING}: ${n(band.jy_per_native_unit * 1e6)} uJy per ${unit}`;
  if (origin !== null) return `${origin}: no Jy and no AB magnitude`;
  return `none: fluxes stay in ${unit}`;
}

function errorLayerText(band: ForcedBand): string {
  const origin = band.err_origin;
  if (origin === null) return band.annulus_px === null ? "none: the fluxes have no error" : "none: errors come from the sky annulus";
  const assumed = origin.assumed ? " (assumed)" : "";
  const noData = origin.n_no_data > 0 ? `, ${counted(origin.n_no_data, "pixel")} without data` : "";
  return `${origin.role.replace(/_/g, " ")} from ${origin.source}${assumed}${noData}`;
}

function flagsText(band: ForcedBand): string {
  if (band.dq_source === null) return "none";
  const named = (band.flag_names ?? []).length;
  return named > 0 ? `${band.dq_source}, ${counted(named, "named bit")}` : band.dq_source;
}

function correctionText(band: ForcedBand): string {
  const c = band.aperture_correction;
  if (c === null) return "none";
  return `x ${n(c.factor)} to ${n(c.reference_radius_px)} px (scatter ${n(c.scatter)}, ${c.n_stars} of ${c.n_candidates} sources)`;
}

export function bandCardRows(band: ForcedBand, nSources: number): [string, string][] {
  const failed = band.n_failed > 0 ? `, ${band.n_failed} failed` : "";
  return [
    ["File", band.file_name],
    ["Filter", filterText(band)],
    ["Pixel scale", `${n(band.pixel_scale_arcsec)} arcsec/px`],
    ["Aperture", apertureText(band)],
    ["Calibration", calibrationText(band)],
    ["Error layer", errorLayerText(band)],
    ["Flags", flagsText(band)],
    ["Aperture correction", correctionText(band)],
    ["Noise factor", band.f_corr === null ? "not given: limits use the formal error" : `f_corr ${n(band.f_corr)}`],
    ["Measured", `${band.n_measured} of ${nSources} positions${failed}`],
  ];
}
