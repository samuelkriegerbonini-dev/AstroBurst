import { withPreview } from "../infrastructure/tauri";
import type { PvDiagramResult, PvRunParams } from "../shared/types/pv";

export type PvDiagramOptions = Omit<PvRunParams, "filePath" | "regionId" | "correction">;

export function computePvDiagram(
  path: string,
  outputDir: string | undefined,
  options: PvDiagramOptions,
): Promise<PvDiagramResult> {
  const { line, stepPx, widthPx, z0, z1, mode, restUm, convention, velocityShiftKms } = options;
  return withPreview<PvDiagramResult>("pv_diagram_cmd", outputDir, {
    path,
    line,
    stepPx,
    widthPx,
    z0,
    z1,
    mode,
    restUm: restUm ?? null,
    convention,
    velocityShiftKms: velocityShiftKms ?? null,
  });
}
