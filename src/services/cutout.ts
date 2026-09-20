import { typedInvoke } from "../infrastructure/tauri";
import type { RegionShape } from "../shared/types/regions";
import type { CutoutExportResult, ExportCutoutOptions, ManualCutoutInput } from "../shared/types/cutout";

export type { CutoutExportResult, CutoutRect, CutoutSizeUnit, ExportCutoutOptions, ManualCutoutInput } from "../shared/types/cutout";

export type BoxRegion = Extract<RegionShape, { shape: "box" }>;

const SOURCE_EXTENSION = /\.(fits?|fts)(\.(fz|gz))?$|\.(asdf|zip|gz)$/i;

export function manualCutoutBox(input: ManualCutoutInput): RegionShape | null {
  const { centreX, centreY, width, height, unit, pixelScaleArcsec } = input;
  if (![centreX, centreY, width, height].every(Number.isFinite) || width <= 0 || height <= 0) return null;
  if (unit === "px") {
    return { shape: "box", x: centreX, y: centreY, width, height, angle: 0 };
  }
  if (pixelScaleArcsec == null || !Number.isFinite(pixelScaleArcsec) || pixelScaleArcsec <= 0) return null;
  return {
    shape: "box",
    x: centreX,
    y: centreY,
    width: width / pixelScaleArcsec,
    height: height / pixelScaleArcsec,
    angle: 0,
  };
}

export function selectedBoxRegion(
  regions: ReadonlyArray<{ id: string; shape: RegionShape }>,
  selectedId: string | null,
): BoxRegion | null {
  if (!selectedId) return null;
  const shape = regions.find((r) => r.id === selectedId)?.shape;
  return shape && shape.shape === "box" ? shape : null;
}

export function describeCutout(result: CutoutExportResult): string {
  const { rect, fraction_on_image, hdus } = result;
  const coverage = `${Math.round(fraction_on_image * 100)}% on image`;
  return `${rect.width}x${rect.height} px at (${rect.x0}, ${rect.y0}), ${coverage}, HDUs ${hdus.join("+")}`;
}

export function cutoutDefaultFileName(sourcePath: string): string {
  const stem = sourcePath
    .split("#")[0]
    .split(/[/\\]/)
    .pop()
    ?.replace(SOURCE_EXTENSION, "") || "image";
  return `${stem}_cutout.fits`;
}

export function exportCutout(
  path: string,
  region: RegionShape,
  options: ExportCutoutOptions = {},
): Promise<CutoutExportResult> {
  return typedInvoke<CutoutExportResult>("export_cutout_cmd", {
    path,
    outputPath: options.outputPath ?? null,
    outputDir: options.outputDir ?? null,
    region,
    includeErr: options.includeErr ?? true,
    includeDq: options.includeDq ?? true,
    sky: options.sky ?? false,
  });
}
