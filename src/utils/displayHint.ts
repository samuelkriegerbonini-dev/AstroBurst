import type { DisplayLook, DisplaySettings, StretchMode } from "../shared/types/display";
import type { DisplayHint } from "../shared/types/preview";
import { reconcileDisplayPatch } from "./displayTransfer";
import { withLook } from "./displayMemory";

export interface HintedDisplay {
  display: DisplaySettings;
  snapshot: DisplaySettings | null;
  stretchBeforeSymmetric?: StretchMode;
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
  const stretchBeforeSymmetric = patch.stretch === undefined ? state.stretchBeforeSymmetric : undefined;
  if (stretchBeforeSymmetric !== undefined && patch.symmetric === false) {
    return { display: reconcileDisplayPatch(state.display, { ...patch, stretch: stretchBeforeSymmetric }), snapshot: state.snapshot };
  }
  const display = reconcileDisplayPatch(state.display, patch);
  const switchedBySymmetric = patch.stretch === undefined && display.stretch !== state.display.stretch;
  return { display, snapshot: state.snapshot, stretchBeforeSymmetric: switchedBySymmetric ? state.display.stretch : stretchBeforeSymmetric };
}

export function persistableDisplay(state: HintedDisplay): DisplaySettings | null {
  return state.snapshot ? null : state.display;
}

export function fileDisplayState(state: HintedDisplay, look: DisplayLook, shown: HintSource | null): HintedDisplay {
  return displayForRecordChange({ display: withLook(state.snapshot ?? state.display, look), snapshot: null }, null, shown);
}
