import type { LightCurveViewState } from "../shared/types/lightCurve";
import type { TsoFile, TsoLightCurves } from "../shared/types/tso";
import { PROVENANCE_FILE_SEPARATOR } from "./csvProvenance";
import { formatSignificant } from "./formatSignificant";
import { QUICK_LOOK_LABEL } from "./lightCurveAnalysis";
import { finiteOrNull, medianOf, type DqHandling, type MeasurementLogDraft } from "./measurementLog";
import { tsoTimeName, type DepthRow, type DepthSpectrum } from "./tsoLightCurves";

type TsoLogFile = Pick<TsoFile, "file_name" | "sha256" | "level">;

const RATIO_DIGITS = 3;
const WHITE_BAND = "white";
const DEPTH_RULE_NOTE = "depth D = 1 - F_in / F_out; depth_err is the larger of the scatter and the formal estimate";

function dqOf(result: TsoLightCurves): DqHandling {
  return result.mode === "masked" ? "excluded" : "not_applied";
}

function fileNames(files: readonly TsoLogFile[]): string[] {
  return files.map((f) => f.file_name);
}

function sha256List(files: readonly TsoLogFile[]): string {
  return files.map((f) => f.sha256).join(PROVENANCE_FILE_SEPARATOR);
}

function seriesSource(result: TsoLightCurves, files: readonly TsoLogFile[]) {
  return { kind: "x1dints", files: fileNames(files), detector: result.series.detector, sporder: result.series.sporder };
}

export function tsoLightCurveEntry(result: TsoLightCurves, files: readonly TsoLogFile[], savedPath: string | null): MeasurementLogDraft {
  const unit = result.flux_unit === "unknown" ? null : result.flux_unit;
  const nUsed = result.white.n_used;
  const n = result.time.length;
  const timeNote =
    result.time_scale === "index"
      ? "time_first and time_last are integration numbers"
      : `time_first and time_last in days, ${tsoTimeName(result.time_scale)}`;
  return {
    kind: "tso_light_curve",
    file: files[0]?.file_name ?? null,
    image: `${n} integrations`,
    dq: dqOf(result),
    unit,
    source: JSON.stringify(seriesSource(result, files)),
    params: {
      series: result.label,
      mode: result.mode,
      time_scale: tsoTimeName(result.time_scale),
      band_lo_um: result.white.band_um[0],
      band_hi_um: result.white.band_um[1],
      limits_origin: result.white.limits_origin,
      n_bins: result.bins.length,
      bins: result.bins.length === 0 ? null : result.bins.map((bin) => `${bin.band_um[0]}-${bin.band_um[1]}`).join(PROVENANCE_FILE_SEPARATOR),
      n_files: files.length,
      source_level: [...new Set(files.map((f) => f.level))].join(PROVENANCE_FILE_SEPARATOR),
      set_id: result.set_id,
      source_sha256: sha256List(files),
      n_rejected_elements: result.rejected_elements.length,
      saved_path: savedPath,
    },
    values: {
      n_integrations: n,
      dropped_nan_time: result.dropped.nan_time,
      dropped_repeated_time: result.dropped.repeated_time,
      white_flux_median: medianOf(result.white.flux),
      white_flux_err_median: medianOf(result.white.flux_err),
      white_n_used_min: nUsed.length === 0 ? null : nUsed.reduce((a, b) => Math.min(a, b)),
      white_n_used_max: nUsed.length === 0 ? null : nUsed.reduce((a, b) => Math.max(a, b)),
      time_first: finiteOrNull(result.time[0]),
      time_last: finiteOrNull(result.time[n - 1]),
    },
    notes: [
      `white_flux_median and white_flux_err_median are sums over the band in ${unit ?? "the flux unit of the file"}; ${timeNote}`,
      ...result.warnings,
      ...result.notes,
    ],
  };
}

function depthEntry(
  band: string,
  row: DepthRow,
  spectrum: DepthSpectrum,
  result: TsoLightCurves,
  view: Pick<LightCurveViewState, "baselineKind">,
  files: readonly TsoLogFile[],
  savedPath: string | null,
): MeasurementLogDraft {
  const windowNote =
    result.time_scale === "index" ? "window in integration numbers" : `window in days, ${tsoTimeName(result.time_scale)}`;
  const lossNote =
    row.pixelLoss && row.pixelRatio !== null
      ? [
          `mean NPIXELS inside the window is ${formatSignificant(row.pixelRatio, RATIO_DIGITS)} of the baseline: the extraction lost pixels during the event, so this depth is biased high`,
        ]
      : [];
  return {
    kind: "tso_depth_spectrum",
    file: files[0]?.file_name ?? null,
    image: `${row.nIn} in, ${row.nOut} out`,
    dq: dqOf(result),
    unit: "fraction",
    source: JSON.stringify({ ...seriesSource(result, files), band_um: row.band }),
    params: {
      series: result.label,
      mode: result.mode,
      band,
      band_lo_um: row.band[0],
      band_hi_um: row.band[1],
      window_start: spectrum.window?.start ?? null,
      window_end: spectrum.window?.end ?? null,
      time_scale: tsoTimeName(result.time_scale),
      baseline_margin_days: spectrum.marginDays,
      baseline_kind: view.baselineKind,
      set_id: result.set_id,
      source_sha256: sha256List(files),
      saved_path: savedPath,
    },
    values: {
      depth: row.depth,
      depth_err: row.err,
      depth_err_scatter: row.errScatter,
      depth_err_formal: row.errFormal,
      n_in: row.nIn,
      n_out: row.nOut,
      npixels_ratio: row.pixelRatio,
    },
    notes: [QUICK_LOOK_LABEL, DEPTH_RULE_NOTE, `band edges in um; ${windowNote}`, ...lossNote],
  };
}

export function tsoDepthSpectrumEntries(
  spectrum: DepthSpectrum,
  result: TsoLightCurves,
  view: Pick<LightCurveViewState, "baselineKind">,
  files: readonly TsoLogFile[],
  savedPath: string | null,
): MeasurementLogDraft[] {
  if (spectrum.window === null) return [];
  const bands: [string, DepthRow][] = [
    ...(spectrum.white === null ? [] : [[WHITE_BAND, spectrum.white] as [string, DepthRow]]),
    ...spectrum.rows.map((row): [string, DepthRow] => [row.label, row]),
  ];
  return bands
    .filter(([, row]) => row.depth !== null)
    .map(([band, row]) => depthEntry(band, row, spectrum, result, view, files, savedPath));
}
