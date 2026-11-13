import type { LightCurveDoc, LightCurveQuantity, LightCurveTimeScale } from "../shared/types/lightCurve";
import { lightCurveDocFromTable } from "./lightCurveDoc";
import type { LightCurveTableColumn, LightCurveTableInput } from "./lightCurveDoc";
import { isFiniteNumber } from "./values";

export const JD_MJD_BOUNDARY = 2400000.5;

export interface TableTimeGuess {
  column: string;
  scale: LightCurveTimeScale;
  reason: string | null;
}

export interface TableValueGuess {
  column: string;
  quantity: LightCurveQuantity;
  errColumn: string | null;
  time: TableTimeGuess;
}

export interface LightCurveTablePlan {
  numericColumns: string[];
  times: TableTimeGuess[];
  time: TableTimeGuess | null;
  values: TableValueGuess[];
  notes: string[];
}

export const NO_TIME_COLUMN_NOTE =
  "no time column found (looked for BJD_TDB, BMJD_TDB, MJD_UTC, MJD, jd_utc, hjd_utc, jd_mid, index): choose the time column and its scale";
export const NO_VALUE_COLUMN_NOTE =
  "no flux or magnitude column found (looked for diff_mag, rel_flux, whitelight_flux, flux, mag, target_flux): choose the value column";
export const BMJD_REASON = "values below 2400000.5: read as BMJD (TDB) = BJD_TDB - 2400000.5";

interface TimeFamily {
  names: readonly string[];
  prefix: string | null;
  scale: LightCurveTimeScale;
  preference: number;
}

const TIME_FAMILIES: readonly TimeFamily[] = [
  { names: ["bjd_tdb", "bjd"], prefix: "bjd_tdb_", scale: "bjd_tdb", preference: 0 },
  { names: ["bmjd_tdb"], prefix: null, scale: "bmjd_tdb", preference: 0 },
  { names: ["mjd", "mjd_utc"], prefix: "mjd_utc_", scale: "mjd_utc", preference: 1 },
  { names: ["jd_utc"], prefix: null, scale: "jd_utc", preference: 2 },
  { names: ["hjd_utc", "hjd"], prefix: null, scale: "hjd_utc", preference: 3 },
  { names: ["jd_mid", "jd"], prefix: null, scale: "jd_header", preference: 4 },
  { names: ["index", "frame"], prefix: null, scale: "index", preference: 5 },
];

const WHITE_LIGHT_PREFIX = "whitelight_flux";
const WHITE_LIGHT_TIME_STEMS: readonly string[] = ["bjd_tdb", "mjd_utc", "mjd"];

interface ValueFamily {
  name: string;
  quantity: LightCurveQuantity;
  errNames: readonly string[];
  preference: number;
}

const VALUE_FAMILIES: readonly ValueFamily[] = [
  { name: "diff_mag", quantity: "diff_mag", errNames: ["diff_err"], preference: 0 },
  { name: "rel_flux", quantity: "rel_flux", errNames: ["rel_flux_err"], preference: 0 },
  { name: WHITE_LIGHT_PREFIX, quantity: "flux", errNames: [], preference: 1 },
  { name: "flux", quantity: "flux", errNames: ["flux_err", "err"], preference: 2 },
  { name: "mag", quantity: "diff_mag", errNames: ["mag_err", "err"], preference: 3 },
  { name: "target_flux", quantity: "flux", errNames: ["target_err"], preference: 4 },
];

function timeFamilyOf(name: string): TimeFamily | undefined {
  const lower = name.toLowerCase();
  return TIME_FAMILIES.find((family) => family.names.includes(lower) || (family.prefix !== null && lower.startsWith(family.prefix)));
}

function valueFamilyOf(name: string): ValueFamily | undefined {
  const lower = name.toLowerCase();
  return VALUE_FAMILIES.find((family) => (family.name === WHITE_LIGHT_PREFIX ? lower.startsWith(family.name) : lower === family.name));
}

function smallestFinite(values: readonly (number | null)[]): number | null {
  return values.reduce<number | null>(
    (smallest, value) => (isFiniteNumber(value) && (smallest === null || value < smallest) ? value : smallest),
    null,
  );
}

export function guessTimeScale(name: string, values: readonly (number | null)[]): TableTimeGuess | null {
  const family = timeFamilyOf(name);
  const smallest = smallestFinite(values);
  if (!family || smallest === null) return null;
  return family.scale === "bjd_tdb" && smallest < JD_MJD_BOUNDARY
    ? { column: name, scale: "bmjd_tdb", reason: BMJD_REASON }
    : { column: name, scale: family.scale, reason: null };
}

function mostPreferred<T>(items: readonly T[], preference: (item: T) => number): T[] {
  const best = items.reduce((lowest, item) => Math.min(lowest, preference(item)), Number.POSITIVE_INFINITY);
  return items.filter((item) => preference(item) === best);
}

function whiteLightTime(valueName: string, times: readonly TableTimeGuess[]): TableTimeGuess | undefined {
  const suffix = valueName.toLowerCase().slice(WHITE_LIGHT_PREFIX.length).replace(/^_order_\d+/, "");
  return WHITE_LIGHT_TIME_STEMS.map((stem) => times.find((guess) => guess.column.toLowerCase() === stem + suffix)).find(
    (guess) => guess !== undefined,
  );
}

function errColumnOf(family: ValueFamily, numeric: readonly LightCurveTableColumn[]): string | null {
  return (
    family.errNames
      .map((errName) => numeric.find((column) => column.name.toLowerCase() === errName))
      .find((column) => column !== undefined)?.name ?? null
  );
}

export function planLightCurveTable(table: LightCurveTableInput): LightCurveTablePlan {
  const numeric = table.columns.filter((column) => column.values !== null);
  const times = numeric.flatMap((column) => guessTimeScale(column.name, column.values ?? []) ?? []);
  const time = mostPreferred(times, (guess) => timeFamilyOf(guess.column)?.preference ?? Number.POSITIVE_INFINITY)[0] ?? null;
  const numericColumns = numeric.map((column) => column.name);
  if (time === null) return { numericColumns, times, time, values: [], notes: [NO_TIME_COLUMN_NOTE] };
  const candidates = numeric.flatMap((column) => {
    const family = valueFamilyOf(column.name);
    return family && smallestFinite(column.values ?? []) !== null ? [{ column: column.name, family }] : [];
  });
  const values = mostPreferred(candidates, (candidate) => candidate.family.preference).map(({ column, family }) => ({
    column,
    quantity: family.quantity,
    errColumn: errColumnOf(family, numeric),
    time: (family.name === WHITE_LIGHT_PREFIX ? whiteLightTime(column, times) : undefined) ?? time,
  }));
  return { numericColumns, times, time, values, notes: values.length === 0 ? [NO_VALUE_COLUMN_NOTE] : [] };
}

export function lightCurveDocsFromTable(
  table: LightCurveTableInput,
  path: string,
  plan: LightCurveTablePlan = planLightCurveTable(table),
): LightCurveDoc[] {
  return plan.values.flatMap(({ column, quantity, errColumn, time }) => {
    const doc = lightCurveDocFromTable(table, {
      path,
      timeColumn: time.column,
      timeScale: time.scale,
      valueColumn: column,
      quantity,
      errColumn,
    });
    if (doc.time.length === 0) return [];
    return time.reason === null ? [doc] : [{ ...doc, notes: [...doc.notes, `time column ${time.column}: ${time.reason}`] }];
  });
}
