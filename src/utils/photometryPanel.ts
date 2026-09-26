import type { PhotometryMeasurement } from "../services/analysis";

export const GAIA_MATCH_PREF_KEY = "astroburst.photometry.gaiaMatch.v1";

export function loadGaiaMatchPreference(storage: Pick<Storage, "getItem"> = localStorage): boolean {
  try {
    return storage.getItem(GAIA_MATCH_PREF_KEY) === "true";
  } catch {
    return false;
  }
}

export function saveGaiaMatchPreference(value: boolean, storage: Pick<Storage, "setItem"> = localStorage): void {
  try {
    storage.setItem(GAIA_MATCH_PREF_KEY, value ? "true" : "false");
  } catch {
  }
}

export function gaiaFailureWarning(error: unknown): string {
  return `Gaia query failed: ${error instanceof Error ? error.message : String(error)}`;
}

export function withWarning(result: PhotometryMeasurement, warning: string): PhotometryMeasurement {
  return result.warnings.includes(warning) ? result : { ...result, warnings: [...result.warnings, warning] };
}

export function replaceNewest(
  history: PhotometryMeasurement[],
  previous: PhotometryMeasurement,
  next: PhotometryMeasurement,
): PhotometryMeasurement[] {
  return history[0] === previous ? [next, ...history.slice(1)] : history;
}

export interface ClickLogGate {
  begin(): number;
  isCurrent(seq: number): boolean;
  hold(seq: number, append: () => void): void;
  commit(seq: number, append: () => void): void;
  supersede(): void;
}

export function createClickLogGate(): ClickLogGate {
  let current = 0;
  let held: (() => void) | null = null;
  const flush = () => {
    const append = held;
    held = null;
    append?.();
  };
  return {
    begin() {
      flush();
      return ++current;
    },
    isCurrent: (seq) => seq === current,
    hold(seq, append) {
      if (seq === current) held = append;
    },
    commit(seq, append) {
      if (seq !== current) return;
      held = null;
      append();
    },
    supersede() {
      current++;
      flush();
    },
  };
}

export function detectedStarsLabel(shown: number, total: number | null | undefined): string {
  const capped = shown > 0 && typeof total === "number" && Number.isFinite(total) && total > shown;
  return capped ? `Detected stars (${shown} of ${total})` : `Detected stars (${shown})`;
}
