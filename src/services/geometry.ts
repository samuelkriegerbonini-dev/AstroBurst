import { typedInvoke } from "../infrastructure/tauri";
import type { GeometryOverrides, ObservationGeometryResult } from "../shared/types/geometry";

export function getObservationGeometry(path: string, overrides: GeometryOverrides = {}): Promise<ObservationGeometryResult> {
  return typedInvoke<ObservationGeometryResult>("observation_geometry_cmd", {
    path,
    targetRa: overrides.targetRa ?? null,
    targetDec: overrides.targetDec ?? null,
    siteLat: overrides.siteLat ?? null,
    siteLon: overrides.siteLon ?? null,
    siteHeight: overrides.siteHeight ?? null,
  });
}
