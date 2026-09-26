import type { RightToolId } from "../hooks/useRightTool";
import { formatLat, formatLon } from "./coordFormat";
import { deepZoomAvailable } from "./analysisSections";

export const KEPT_RIGHT_TOOL: RightToolId = "analysis";

export interface RightToolSlotInput {
  rightTool: RightToolId | null;
  displayTool: RightToolId | null;
  columnMounted: boolean;
  fileKey: string | null;
  keptFileKey: string | null;
}

export interface RightToolSlots {
  kept: { visible: boolean; active: boolean } | null;
  transient: { id: RightToolId; active: boolean } | null;
}

export function keptToolFileKey(rightTool: RightToolId | null, fileKey: string | null, keptFileKey: string | null): string | null {
  if (rightTool === KEPT_RIGHT_TOOL) return fileKey;
  return keptFileKey === fileKey ? keptFileKey : null;
}

export function rightToolSlots({ rightTool, displayTool, columnMounted, fileKey, keptFileKey }: RightToolSlotInput): RightToolSlots {
  const showing = columnMounted ? displayTool : null;
  const keptMounted = rightTool === KEPT_RIGHT_TOOL || (fileKey !== null && keptFileKey === fileKey);
  return {
    kept: keptMounted ? { visible: showing === KEPT_RIGHT_TOOL, active: rightTool === KEPT_RIGHT_TOOL } : null,
    transient: showing !== null && showing !== KEPT_RIGHT_TOOL ? { id: showing, active: rightTool === showing } : null,
  };
}

export interface GpuDisplayInput {
  hasFile: boolean;
  rgbView: boolean;
  useGpu: boolean;
  hasRawPixels: boolean;
  previewOnly: boolean;
}

export function gpuDisplayOnScreen({ hasFile, rgbView, useGpu, hasRawPixels, previewOnly }: GpuDisplayInput): boolean {
  return hasFile && !rgbView && useGpu && hasRawPixels && !previewOnly;
}

export function gpuAfterProbe(ok: boolean, savedPreference: boolean | null, current: boolean): boolean {
  if (!ok) return false;
  return savedPreference === null ? true : current;
}

export type BackToFileAction = "rgb-file" | "display-only" | "clear";

export function backToFileAction({ isRgbFile, hasProcessed, wizardCompositeReady }: {
  isRgbFile: boolean;
  hasProcessed: boolean;
  wizardCompositeReady: boolean;
}): BackToFileAction {
  if (isRgbFile && !hasProcessed) return "rgb-file";
  return wizardCompositeReady ? "display-only" : "clear";
}

export function formatPixelValue(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return "—";
  const abs = Math.abs(v);
  if (abs !== 0 && (abs < 1e-3 || abs >= 1e6)) return v.toExponential(3);
  if (Number.isInteger(v)) return String(v);
  return v.toFixed(abs >= 100 ? 2 : 4);
}

export interface PixelValueAt {
  x: number;
  y: number;
  value: number | null;
  unit: string | null;
}

export interface SkyAt {
  x: number;
  y: number;
  radec: [number, number];
}

export interface StatusStripParts {
  position: string;
  value: string;
  sky: string | null;
}

export function statusStripParts(
  pixel: { x: number; y: number } | null,
  valueAt: PixelValueAt | null,
  skyAt: SkyAt | null,
): StatusStripParts | null {
  if (!pixel) return null;
  const valueHere = valueAt !== null && valueAt.x === pixel.x && valueAt.y === pixel.y ? valueAt : null;
  const skyHere = skyAt !== null && skyAt.x === pixel.x && skyAt.y === pixel.y ? skyAt : null;
  return {
    position: `x ${pixel.x}  y ${pixel.y}`,
    value: valueHere ? `${formatPixelValue(valueHere.value)}${valueHere.unit ? ` ${valueHere.unit}` : ""}` : "…",
    sky: skyHere
      ? `RA ${formatLon(skyHere.radec[0], { hours: true, format: "sexagesimal" })}  Dec ${formatLat(skyHere.radec[1], { format: "sexagesimal" })} ICRS`
      : null,
  };
}

export function viewerPublishesPixel({ composite, fileRgbView }: { composite: boolean; fileRgbView: boolean }): boolean {
  return !composite || fileRgbView;
}

export function statusStripIdleText(publishesPixel: boolean): string {
  return publishesPixel ? "Hover the image for x, y, value and RA/Dec" : "No pixel readout for the colour composite";
}

export interface PreviewTextureInput {
  renderW: number;
  renderH: number;
  fitsW: number;
  fitsH: number;
  deepZoomOffered: boolean;
}

export function previewTextureTitle({ renderW, renderH, fitsW, fitsH, deepZoomOffered }: PreviewTextureInput): string {
  const texture = `The viewer shows a downsampled preview texture (${renderW}×${renderH} of ${fitsW}×${fitsH} FITS pixels).`;
  return deepZoomOffered && deepZoomAvailable(fitsW, fitsH)
    ? `${texture} For full-resolution pixels open Analysis > Deep Zoom.`
    : texture;
}
