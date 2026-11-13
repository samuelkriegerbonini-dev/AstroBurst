import { typedInvoke } from "../infrastructure/tauri";
import type { PlaneInfo } from "../shared/types/fits.types";
import type { CompanionRole, CompanionSuggestions, ErrorLayerRole } from "../shared/types/layers";

export function setCompanion(
  path: string,
  role: CompanionRole,
  source: string,
  layerRole: ErrorLayerRole | null = null,
): Promise<PlaneInfo> {
  return typedInvoke<PlaneInfo>("set_companion_cmd", { path, role, source, layerRole });
}

export function clearCompanion(path: string, role: CompanionRole, disable = false): Promise<PlaneInfo> {
  return typedInvoke<PlaneInfo>("clear_companion_cmd", { path, role, disable });
}

export function setFlagExclusion(path: string, mask: number | null): Promise<PlaneInfo> {
  return typedInvoke<PlaneInfo>("set_flag_exclusion_cmd", { path, mask });
}

export function suggestCompanions(path: string): Promise<CompanionSuggestions> {
  return typedInvoke<CompanionSuggestions>("suggest_companions_cmd", { path });
}

export function setSdssFrameError(path: string, gain: number, darkVariance: number): Promise<PlaneInfo> {
  return typedInvoke<PlaneInfo>("set_sdss_frame_error_cmd", { path, gain, darkVariance });
}
