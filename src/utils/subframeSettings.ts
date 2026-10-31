export interface SubframeThresholds {
  maxFwhm: number;
  maxEccentricity: number;
  minSnr: number;
  minStars: number;
}

export interface SubframeScoreWeights {
  fwhmWeight: number;
  eccentricityWeight: number;
  snrWeight: number;
  noiseWeight: number;
}

export type SubframeSettings = SubframeThresholds & SubframeScoreWeights;

export const DEFAULT_SUBFRAME_THRESHOLDS: SubframeThresholds = {
  maxFwhm: 8.0,
  maxEccentricity: 0.7,
  minSnr: 5.0,
  minStars: 5,
};

export const DEFAULT_SUBFRAME_SETTINGS: SubframeSettings = {
  ...DEFAULT_SUBFRAME_THRESHOLDS,
  fwhmWeight: 1.0,
  eccentricityWeight: 0.5,
  snrWeight: 1.0,
  noiseWeight: 0.3,
};

export const SUBFRAME_STALE_NOTE = "Settings changed since the analysis: Analyze again";
const SUBFRAME_FRAMES_STALE_NOTE = "Frames changed since the analysis: Analyze again";
const SUBFRAME_FRAMES_AND_SETTINGS_STALE_NOTE = "Frames and settings changed since the analysis: Analyze again";

export function subframeFilesChanged(analysed: readonly string[], current: readonly string[]): boolean {
  const before = new Set(analysed);
  const now = new Set(current);
  return before.size !== now.size || [...before].some((path) => !now.has(path));
}

export function subframeStaleNote(settingsChanged: boolean, filesChanged: boolean): string | null {
  if (filesChanged) return settingsChanged ? SUBFRAME_FRAMES_AND_SETTINGS_STALE_NOTE : SUBFRAME_FRAMES_STALE_NOTE;
  return settingsChanged ? SUBFRAME_STALE_NOTE : null;
}

const SETTING_KEYS: readonly (keyof SubframeSettings)[] = [
  "maxFwhm",
  "maxEccentricity",
  "minSnr",
  "minStars",
  "fwhmWeight",
  "eccentricityWeight",
  "snrWeight",
  "noiseWeight",
];

export function subframeSettingsChanged(analysed: Partial<SubframeSettings>, current: Partial<SubframeSettings>): boolean {
  return SETTING_KEYS.some((key) => analysed[key] !== current[key]);
}

interface AnalysedSubframes {
  analysedWith?: SubframeThresholds;
}

export function analysedThresholdsOf(
  bins: readonly { id: string }[],
  results: Readonly<Record<string, AnalysedSubframes | undefined>>,
): SubframeThresholds | null {
  for (const bin of bins) {
    const analysedWith = results[bin.id]?.analysedWith;
    if (analysedWith) return analysedWith;
  }
  return null;
}

export function subframeResultStale(result: AnalysedSubframes, current: SubframeThresholds): boolean {
  return result.analysedWith !== undefined && subframeSettingsChanged(result.analysedWith, current);
}
