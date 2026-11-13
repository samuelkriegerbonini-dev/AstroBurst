import type { LightCurveDoc } from "../shared/types/lightCurve";
import type { SeriesKey, TsoFile, TsoLightCurves, WhtltComparison, WhtltImport, WhtltSeries } from "../shared/types/tso";
import { formatSignificant } from "./formatSignificant";
import { bandText, lightCurveDocsFromTso } from "./tsoLightCurves";
import { isFiniteNumber } from "./values";

export const WHTLT_OVERLAY = "white light and whtlt";
export const APP_WHITE_DOC_ID = "white:app";

const DIFFERENCE_DIGITS = 3;

export interface WhtltRowText {
  column: string;
  series: string;
  rows: string;
  difference: string;
  verdict: string;
  limits: string | null;
}

function asciiLower(text: string): string {
  return text.replace(/[A-Z]/g, (letter) => letter.toLowerCase());
}

export function defaultWhtltPath(files: readonly Pick<TsoFile, "whtlt_path">[]): string | null {
  return files.find((file) => file.whtlt_path !== null)?.whtlt_path ?? null;
}

export function whtltSeriesFor(imported: WhtltImport, key: SeriesKey): WhtltSeries | null {
  const detector = asciiLower(key.detector);
  const matches = imported.series.filter(
    (w) => (w.detector === null || asciiLower(w.detector) === detector) && (w.sporder === null || w.sporder === key.sporder),
  );
  return matches.length === 1 ? matches[0] : null;
}

export function whtltDocs(imported: WhtltImport, curves: TsoLightCurves, paths: readonly string[]): LightCurveDoc[] {
  if (curves.time_scale === "index") return [];
  const w = whtltSeriesFor(imported, curves.series);
  if (w === null) return [];
  const utc = curves.time_scale === "mjd_utc";
  const times = utc ? w.mjd_utc : w.bmjd_tdb;
  const kept = times.flatMap((time, row) => (isFiniteNumber(time) ? [{ time, row }] : []));
  if (kept.length === 0) return [];
  const app: LightCurveDoc = {
    ...lightCurveDocsFromTso(curves, paths)[0],
    id: APP_WHITE_DOC_ID,
    label: "white light (app)",
    overlay: WHTLT_OVERLAY,
  };
  const archived: LightCurveDoc = {
    id: `whtlt:${w.column}`,
    label: `archived whtlt (${w.column})`,
    quantity: "flux",
    unit: w.flux_unit,
    timeScale: curves.time_scale,
    time: kept.map((point) => point.time),
    value: kept.map((point) => w.flux[point.row]),
    err: kept.map(() => null),
    pointRef: kept.map((point) => point.row),
    flags: kept.map(() => []),
    source: { kind: "table", path: imported.path, time_column: utc ? "MJD_UTC" : "BJD_TDB", value_column: w.column },
    notes: [`pipeline white light read from ${imported.file_name}`],
    overlay: WHTLT_OVERLAY,
  };
  return [app, archived];
}

function verdictText(row: WhtltComparison): string {
  if (row.series === null || row.n_joined === 0) return "not compared";
  return row.within_1e4 ? "matches (within 1e-4)" : "differs";
}

export function comparisonRowText(row: WhtltComparison): WhtltRowText {
  return {
    column: row.column,
    series: row.label ?? "no single series",
    rows: `${row.n_joined} of ${row.n_whtlt}`,
    difference: row.max_rel_diff === null ? "--" : formatSignificant(row.max_rel_diff, DIFFERENCE_DIGITS),
    verdict: verdictText(row),
    limits: row.limits_um === null || row.limits_origin === null ? null : `${bandText(row.limits_um)} (${row.limits_origin})`,
  };
}
