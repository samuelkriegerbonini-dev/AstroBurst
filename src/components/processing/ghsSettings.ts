import type { GhsSettings } from "../../services/processing";

export function withSymmetryPoint(settings: GhsSettings, symmetryPoint: number): GhsSettings {
  return {
    ...settings,
    symmetryPoint,
    shadowProtect: Math.min(settings.shadowProtect, symmetryPoint),
    highlightProtect: Math.max(settings.highlightProtect, symmetryPoint),
  };
}

export function withShadowProtect(settings: GhsSettings, shadowProtect: number): GhsSettings {
  return { ...settings, shadowProtect: Math.min(shadowProtect, settings.symmetryPoint) };
}

export function withHighlightProtect(settings: GhsSettings, highlightProtect: number): GhsSettings {
  return { ...settings, highlightProtect: Math.max(highlightProtect, settings.symmetryPoint) };
}
