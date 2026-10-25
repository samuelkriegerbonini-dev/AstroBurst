import type { MastMissionId } from "../../../shared/types/mast";

export const CATEGORY_COLUMN = "_category";

export const MISSION_LABELS: Record<MastMissionId, string> = {
  jwst: "JWST",
  hst: "HST",
  roman: "Roman",
  iue: "IUE",
  classy: "CLASSY",
  ullyses: "ULLYSES",
  roman_cgi: "Roman CGI",
  roman_spectra: "Roman Spectra",
  tess: "TESS",
  multi_mission: "All missions",
};

export function missionLabelOf(id: MastMissionId): string {
  return MISSION_LABELS[id] ?? id;
}
