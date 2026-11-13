import type { MastFilterValues, MastMissionId } from "../../../shared/types/mast";

export interface Suggestion {
  id: string;
  label: string;
  mission: MastMissionId;
  text: string;
  filters: MastFilterValues;
  preset?: string;
}

export const SUGGESTIONS: readonly Suggestion[] = [
  { id: "m16-webb", label: "M16, Webb combined", mission: "jwst", text: "M16", filters: { level: "combined" } },
  { id: "p2739-webb", label: "Programme 2739, Webb", mission: "jwst", text: "2739", filters: { level: "combined" } },
  { id: "m16-hubble", label: "M16, Hubble WFC3 F502N", mission: "hst", text: "M16", filters: { instrument: "WFC3", filter: "F502N" } },
  { id: "p1366-tso", label: "Programme 1366, Webb time series (WASP-39 b)", mission: "jwst", text: "1366", filters: {}, preset: "tso" },
  { id: "p2072-x1d", label: "Programme 2072, Webb 1D spectra (SN 2021aefx)", mission: "jwst", text: "2072", filters: {}, preset: "spectra_1d" },
  { id: "p1266-ifu", label: "Programme 1266, Webb IFU cubes (Galactic Centre)", mission: "jwst", text: "1266", filters: {}, preset: "ifu_cubes" },
];

export const FIRST_RUN_HINTS: readonly { keys: string; desc: string }[] = [
  { keys: "/", desc: "search" },
  { keys: "arrows", desc: "move" },
  { keys: "Space", desc: "large view" },
  { keys: "V", desc: "gallery or table" },
  { keys: "?", desc: "shortcuts" },
];
