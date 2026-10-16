import type { MastColumn, MastResolved, MastRow } from "../../../shared/types/mast";
import { coordinatesText } from "./copyBuilders";
import { dateText, isPublic } from "./cardLabel";

export interface MetaRow {
  label: string;
  value: string;
  copy: string | null;
  note: string | null;
}

export interface MetaSection {
  key: "identifiers" | "pointing" | "observation";
  title: string;
  rows: MetaRow[];
}

export interface MetaColumn {
  name: string;
  label: string;
  value: string;
}

export const FOOTPRINT_NOTE = "Footprint on the image arrives in v1.1";
export const SEP_NOTE = "unit and meaning differ by mission";

const S_REGION_FULL_MAX = 40;
const S_REGION_HEAD = 18;

function metaRow(label: string, value: string, copy: string | null = null, note: string | null = null): MetaRow {
  return { label, value, copy, note };
}

export function valueText(value: string | number | boolean | null | undefined): string {
  return value === null || value === undefined ? "" : String(value);
}

export function sRegionText(sRegion: string): string {
  if (sRegion.length <= S_REGION_FULL_MAX) return sRegion;
  return `${sRegion.slice(0, S_REGION_HEAD)}… (${sRegion.length.toLocaleString("en-US")} chars)`;
}

function dateTimeText(value: string): string {
  const m = /^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}(?::\d{2})?)/.exec(value.trim());
  return m ? `${m[1]} ${m[2]}` : value.trim();
}

function identifiers(row: MastRow): MetaRow[] {
  const rows = [metaRow("Dataset", row.datasetId, row.datasetId)];
  if (row.program) {
    const value = row.observation ? `${row.program} / ${row.observation}` : row.program;
    const progObs = row.values._prog_obs;
    rows.push(metaRow("Programme / Obs", value, typeof progObs === "string" && progObs ? progObs : value));
  }
  const target = row.target?.trim();
  if (target) {
    const at = row.raDeg !== null && row.decDeg !== null ? `  (targ ${coordinatesText(row.raDeg, row.decDeg)})` : "";
    rows.push(metaRow("Target", `${target}${at}`, target));
  }
  return rows;
}

function pointing(row: MastRow, resolved: MastResolved | null, searchPosition: { raDeg: number; decDeg: number } | null): MetaRow[] {
  const rows: MetaRow[] = [];
  if (searchPosition) {
    const coords = coordinatesText(searchPosition.raDeg, searchPosition.decDeg);
    const by = resolved ? ` (${resolved.resolver} "${resolved.canonicalName}")` : "";
    rows.push(metaRow("Search position", `${coords}${by}`, coords));
  }
  if (row.sepArcmin !== null && Number.isFinite(row.sepArcmin)) {
    rows.push(row.sepVerified ? metaRow("Sep", `${row.sepArcmin.toFixed(2)}'`) : metaRow("Sep (MAST)", row.sepArcmin.toFixed(3), null, SEP_NOTE));
  }
  if (row.sRegion) rows.push(metaRow("s_region", sRegionText(row.sRegion), row.sRegion, FOOTPRINT_NOTE));
  return rows;
}

function observation(row: MastRow): MetaRow[] {
  const rows: MetaRow[] = [];
  const add = (label: string, value: string | null | undefined) => {
    if (value) rows.push(metaRow(label, value));
  };
  add("Instrument", row.instrument?.trim());
  add("Exp type", row.expType);
  add("Filter (row)", row.filter);
  if (row.filters.length > 1) add("Filters in set", row.filters.join(" "));
  add("Date obs (UTC)", row.dateObs ? dateTimeText(row.dateObs) : null);
  add("Exposure", row.expTimeS !== null && Number.isFinite(row.expTimeS) ? `${Number(row.expTimeS.toFixed(2))} s` : null);
  add("Access", row.access);
  if (!isPublic(row)) add("Public from", dateText(row.releaseDate));
  return rows;
}

export function metadataSections(input: { row: MastRow; resolved: MastResolved | null; searchPosition: { raDeg: number; decDeg: number } | null }): MetaSection[] {
  const { row, resolved, searchPosition } = input;
  return [
    { key: "identifiers", title: "Identifiers", rows: identifiers(row) },
    { key: "pointing", title: "Pointing", rows: pointing(row, resolved, searchPosition) },
    { key: "observation", title: "Observation", rows: observation(row) },
  ];
}

export function allColumns(row: MastRow, columns: readonly MastColumn[], filter: string): MetaColumn[] {
  const q = filter.trim().toLowerCase();
  return columns
    .filter((c) => !q || c.name.toLowerCase().includes(q) || c.label.toLowerCase().includes(q))
    .map((c) => ({ name: c.name, label: c.label, value: valueText(row.values[c.name]) }));
}
