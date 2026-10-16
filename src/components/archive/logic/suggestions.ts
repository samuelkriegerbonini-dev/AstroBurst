import type { MastFilterValues, MastMissionId } from "../../../shared/types/mast";

export interface Suggestion {
  id: string;
  label: string;
  mission: MastMissionId;
  text: string;
  filters: MastFilterValues;
}

export const SUGGESTIONS: readonly Suggestion[] = [
  { id: "m16-webb", label: "M16, Webb combined", mission: "jwst", text: "M16", filters: { level: "combined" } },
  { id: "p2739-webb", label: "Programme 2739, Webb", mission: "jwst", text: "2739", filters: { level: "combined" } },
  { id: "m16-hubble", label: "M16, Hubble WFC3 F502N", mission: "hst", text: "M16", filters: { instrument: "WFC3", filter: "F502N" } },
];

export const FIRST_RUN_HINTS: readonly { keys: string; desc: string }[] = [
  { keys: "/", desc: "search" },
  { keys: "arrows", desc: "move" },
  { keys: "Space", desc: "large view" },
  { keys: "V", desc: "gallery or table" },
  { keys: "?", desc: "shortcuts" },
];
