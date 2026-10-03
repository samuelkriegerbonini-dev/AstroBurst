import type { DisplaySettings } from "../shared/types/display";
import type { DisplayHint } from "../shared/types/preview";
import { reconcileDisplayPatch } from "./displayTransfer";

export interface HintedDisplay {
  display: DisplaySettings;
  snapshot: DisplaySettings | null;
}

export interface HintSource {
  displayHint?: DisplayHint;
}

export function initialHintedDisplay(saved: DisplaySettings): HintedDisplay {
  return { display: saved, snapshot: null };
}

export function displayForRecordChange(state: HintedDisplay, prev: HintSource | null, next: HintSource | null): HintedDisplay {
  if (prev === next) return state;
  const hint = next?.displayHint;
  if (hint) return { display: reconcileDisplayPatch(state.display, hint), snapshot: state.snapshot ?? state.display };
  return state.snapshot ? { display: state.snapshot, snapshot: null } : state;
}

export function patchHintedDisplay(state: HintedDisplay, patch: Partial<DisplaySettings>): HintedDisplay {
  return { display: reconcileDisplayPatch(state.display, patch), snapshot: state.snapshot };
}

export function persistableDisplay(state: HintedDisplay): DisplaySettings | null {
  return state.snapshot ? null : state.display;
}
