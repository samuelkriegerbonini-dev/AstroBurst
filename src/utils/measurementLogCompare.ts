import type { CompareReport, CompareRequest } from "../shared/types/catalogCompare";
import { cutText, rejectedText } from "./catalogCompare";
import { dqHandlingOf, type MeasurementLogDraft, type MeasurementProvenance } from "./measurementLog";
import { finiteOrNull } from "./values";

export const COMPARE_LOG_NOTE =
  "ratio_median and err_ratio_median are measured over catalogue (dimensionless); ratio_offset_mag in mag; implied_zero_point_ab in AB mag of one image unit; radii in px and arcsec; pixel_scale_arcsec in arcsec per px; summary only: the rows are in the CSV of the Compare section";

function joinedOrNull(items: readonly string[]): string | null {
  return items.length === 0 ? null : items.join("; ");
}

export function catalogCompareEntry(
  prov: MeasurementProvenance,
  report: CompareReport,
  opts: { request: CompareRequest; presetEdited: boolean },
): MeasurementLogDraft {
  const m = opts.request.mapping;
  const band = report.band;
  const aperture = report.aperture;
  const errorKind = m.flux_err !== null ? "flux_err" : m.flux_ivar !== null ? "flux_ivar" : null;
  return {
    kind: "catalog_compare",
    file: prov.file ?? band.file_name,
    image: prov.image,
    dq: dqHandlingOf(opts.request.exclude_dq === true, band.dq_source !== null),
    unit: null,
    source: JSON.stringify({ kind: "catalogue", table: report.table.file_name }),
    params: {
      table: report.table.file_name,
      table_format: report.table.format,
      preset: report.preset,
      preset_edited: opts.presetEdited,
      catalogue_band: report.catalogue_band,
      image_filter: band.filter === null ? null : `${band.filter.dataset} ${band.filter.filter}`,
      ra_column: m.ra,
      dec_column: m.dec,
      id_column: m.id,
      flux_column: m.flux,
      flux_unit: m.flux_unit,
      zero_point: finiteOrNull(m.zero_point),
      error_column: m.flux_err ?? m.flux_ivar,
      error_kind: errorKind,
      class_column: m.class,
      missing_value: finiteOrNull(m.missing_value),
      radius_px: finiteOrNull(aperture.radius_px),
      radius_arcsec: finiteOrNull(aperture.radius_arcsec),
      annulus_inner_px: finiteOrNull(aperture.annulus_px?.[0]),
      annulus_outer_px: finiteOrNull(aperture.annulus_px?.[1]),
      pixel_scale_arcsec: finiteOrNull(aperture.pixel_scale_arcsec),
      cuts: joinedOrNull(opts.request.cuts.map(cutText)),
      min_snr: finiteOrNull(opts.request.min_snr),
      max_snr: finiteOrNull(opts.request.max_snr),
      min_neighbour_arcsec: finiteOrNull(opts.request.min_neighbour_arcsec),
      calibration: report.calibration_origin,
    },
    values: {
      ratio_median: finiteOrNull(report.ratio_median),
      ratio_median_err: finiteOrNull(report.ratio_median_err),
      ratio_nmad: finiteOrNull(report.ratio_nmad),
      n_ratio: finiteOrNull(report.n_ratio),
      ratio_offset_mag: finiteOrNull(report.ratio_offset_mag),
      err_ratio_median: finiteOrNull(report.err_ratio_median),
      err_ratio_nmad: finiteOrNull(report.err_ratio_nmad),
      n_err: finiteOrNull(report.n_err),
      implied_zero_point_ab: finiteOrNull(report.implied_zero_point_ab),
      implied_zero_point_nmad: finiteOrNull(report.implied_zero_point_nmad),
      n_zero_point: finiteOrNull(report.n_zero_point),
      n_table: finiteOrNull(report.n_table),
      n_on_image: finiteOrNull(report.n_on_image),
      n_after_cuts: finiteOrNull(report.n_after_cuts),
      n_measured: finiteOrNull(report.n_measured),
      n_used: finiteOrNull(report.n_used),
      rejected: joinedOrNull(report.rejected.map(rejectedText)),
    },
    notes: [...report.warnings, COMPARE_LOG_NOTE],
  };
}
