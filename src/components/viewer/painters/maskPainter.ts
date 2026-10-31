import type { OverlayLayer, OverlayPainter } from "../../../utils/overlayStore";
import { edgeToCentre } from "../../../utils/pixelMapping";
import type { Pt } from "../../../utils/regionGeometry";
import type { MaskPreview, MaskStyle } from "../../../shared/types/masks";

export const MASK_LAYER_ID = "mask";
export const MASK_LAYER_KIND = "mask";

const CELL_FULL = 255;

export type MaskSurface = (rgba: Uint8ClampedArray<ArrayBuffer>, width: number, height: number) => CanvasImageSource | null;

export type MaskAffine = [number, number, number, number, number, number];

export interface MaskGrid {
  width: number;
  height: number;
}

export interface MaskRgba {
  rgba: Uint8ClampedArray<ArrayBuffer>;
  markedCells: number;
}

export interface MaskOverlayLayer extends OverlayLayer {
  maskId: string;
  markedCells: number;
}

export interface MaskLayerInput {
  maskId: string;
  preview: MaskPreview;
  dims: readonly [number, number];
  style: MaskStyle;
}

export function maskRgba(cells: Uint8Array, { opacity, tint, displayInvert }: MaskStyle): MaskRgba {
  const alphaOf = new Uint8ClampedArray(CELL_FULL + 1);
  for (let cell = 0; cell <= CELL_FULL; cell++) alphaOf[cell] = Math.round((displayInvert ? CELL_FULL - cell : cell) * opacity);
  const unmarked = displayInvert ? CELL_FULL : 0;
  const [r, g, b] = tint;
  const rgba = new Uint8ClampedArray(cells.length * 4);
  let markedCells = 0;
  for (let i = 0; i < cells.length; i++) {
    const cell = cells[i];
    const o = i * 4;
    rgba[o] = r;
    rgba[o + 1] = g;
    rgba[o + 2] = b;
    rgba[o + 3] = alphaOf[cell];
    if (cell !== unmarked) markedCells++;
  }
  return { rgba, markedCells };
}

export function maskAffine(toScreen: (p: Pt) => Pt, [cols, rows]: readonly [number, number], grid: MaskGrid): MaskAffine {
  const origin = toScreen({ x: edgeToCentre(0), y: edgeToCentre(0) });
  const right = toScreen({ x: edgeToCentre(cols), y: edgeToCentre(0) });
  const down = toScreen({ x: edgeToCentre(0), y: edgeToCentre(rows) });
  return [
    (right.x - origin.x) / grid.width,
    (right.y - origin.y) / grid.width,
    (down.x - origin.x) / grid.height,
    (down.y - origin.y) / grid.height,
    origin.x,
    origin.y,
  ];
}

export function createMaskPainter(image: CanvasImageSource, dims: readonly [number, number], grid: MaskGrid): OverlayPainter {
  return ({ ctx, toScreen }) => {
    const affine = maskAffine(toScreen, dims, grid);
    if (!affine.every(Number.isFinite)) return;
    ctx.save();
    ctx.imageSmoothingEnabled = false;
    ctx.transform(...affine);
    ctx.drawImage(image, 0, 0);
    ctx.restore();
  };
}

function canvasSurface(rgba: Uint8ClampedArray<ArrayBuffer>, width: number, height: number): CanvasImageSource | null {
  if (typeof document === "undefined") return null;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.putImageData(new ImageData(rgba, width, height), 0, 0);
  return canvas;
}

export function maskLayer({ maskId, preview, dims, style }: MaskLayerInput, surface: MaskSurface = canvasSurface): MaskOverlayLayer | null {
  if (preview.width === 0 || preview.height === 0) return null;
  const { rgba, markedCells } = maskRgba(preview.cells, style);
  const image = surface(rgba, preview.width, preview.height);
  if (!image) return null;
  return {
    id: MASK_LAYER_ID,
    kind: MASK_LAYER_KIND,
    visible: true,
    paint: createMaskPainter(image, dims, { width: preview.width, height: preview.height }),
    maskId,
    markedCells,
  };
}

export function isMaskLayer(layer: OverlayLayer): layer is MaskOverlayLayer {
  const candidate = layer as Partial<MaskOverlayLayer>;
  return layer.id === MASK_LAYER_ID && typeof candidate.maskId === "string" && typeof candidate.markedCells === "number";
}
