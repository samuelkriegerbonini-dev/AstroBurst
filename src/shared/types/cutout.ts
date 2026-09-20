export interface CutoutRect {
  x0: number;
  y0: number;
  width: number;
  height: number;
}

export interface CutoutExportResult {
  output_path: string;
  rect: CutoutRect;
  fraction_on_image: number;
  hdus: string[];
  ltv1: number;
  ltv2: number;
  rotated_box_used_bounds: boolean;
  warnings: string[];
  elapsed_ms: number;
}

export type CutoutSizeUnit = "px" | "arcsec";

export interface ManualCutoutInput {
  centreX: number;
  centreY: number;
  width: number;
  height: number;
  unit: CutoutSizeUnit;
  pixelScaleArcsec: number | null;
}

export interface ExportCutoutOptions {
  outputPath?: string;
  outputDir?: string;
  includeErr?: boolean;
  includeDq?: boolean;
  sky?: boolean;
}
