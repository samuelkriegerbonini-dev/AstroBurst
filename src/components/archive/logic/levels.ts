import type { MastLevel } from "../../../shared/types/mast";

export interface LevelChoice {
  value: MastLevel;
  label: string;
  title: string;
}

export const LEVEL_CHOICES: readonly LevelChoice[] = [
  { value: "combined", label: "Combined L3", title: "Level 3: combined mosaics, cubes and spectra of several exposures, including source-based products (2bs, 3)" },
  { value: "exposures", label: "Exposures L1-2", title: "Levels 1b to 2c: single exposures, from uncalibrated ramps to calibrated frames" },
  { value: "all", label: "All levels", title: "Every level, including the source-based sets that carry only 2bs" },
];

export function levelOf(value: unknown): MastLevel | null {
  return LEVEL_CHOICES.find((choice) => choice.value === value)?.value ?? null;
}

export function narrowsLevel(level: MastLevel | null | undefined): level is Exclude<MastLevel, "all"> {
  return level === "combined" || level === "exposures";
}

export function levelChipLabel(level: MastLevel | null | undefined): string | null {
  return narrowsLevel(level) ? (LEVEL_CHOICES.find((choice) => choice.value === level)?.label ?? null) : null;
}
