import { formatLat, formatLon } from "./coordFormat";
import { deepZoomAvailable } from "./analysisSections";
import { viewportClickRoute, type ViewportClickRoute } from "./regionClick";
import type { WavelengthProbe } from "../shared/types/analysis";

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

export type PreviewViewer = "empty" | "cpu" | "preview-tab";

export function previewViewer(input: GpuDisplayInput): PreviewViewer {
  if (!input.hasFile) return "empty";
  return input.rgbView || gpuDisplayOnScreen(input) ? "preview-tab" : "cpu";
}

export const NEEDS_GPU_TITLE = "needs GPU rendering";
export const PNG_ONLY_TITLE = "PNG-only result; use Revert to original in the preview header to get the display controls back";
export const GPU_WAITING_TITLE = "waiting for the GPU image";
export const GPU_LOAD_FAILED_TITLE = "GPU image load failed";

export function cpuViewerDisplayTitle({ useGpu, previewOnly, loadFailed }: {
  useGpu: boolean;
  previewOnly: boolean;
  loadFailed: boolean;
}): string {
  if (!useGpu) return NEEDS_GPU_TITLE;
  if (previewOnly) return PNG_ONLY_TITLE;
  return loadFailed ? GPU_LOAD_FAILED_TITLE : GPU_WAITING_TITLE;
}

export interface GpuToggleInput {
  probing: boolean;
  loading: boolean;
  available: boolean | null;
  supported: boolean;
  useGpu: boolean;
  loadError: string | null;
  reason: string | null;
  pngOnlyMono: boolean;
  cpuViewerShown: boolean;
}

export type GpuToggleState = "probing" | "loading" | "unavailable" | "failed" | "cpu" | "png-only" | "gpu";
export type GpuToggleTone = "failed" | "gpu" | "muted" | "off";

export interface GpuToggleView {
  state: GpuToggleState;
  label: string;
  title: string;
  tone: GpuToggleTone;
}

export const GPU_PNG_ONLY_TOGGLE_TITLE = "GPU on: this PNG-only result has no FITS pixels, so the CPU viewer shows it";
export const GPU_WAITING_TOGGLE_TITLE = "GPU on: waiting for the GPU image; the CPU viewer shows it meanwhile";
export const GPU_PROBING_TOGGLE_TITLE = "Checking for a GPU";
export const GPU_PROBING_CPU_VIEWER_TOGGLE_TITLE = "Checking for a GPU; the CPU viewer shows the image meanwhile";

function gpuToggleState({ probing, loading, available, useGpu, loadError, pngOnlyMono }: GpuToggleInput): GpuToggleState {
  if (probing) return "probing";
  if (loading) return "loading";
  if (available === false) return "unavailable";
  if (loadError) return "failed";
  if (!useGpu) return "cpu";
  return pngOnlyMono ? "png-only" : "gpu";
}

const GPU_TOGGLE_LABELS: Record<GpuToggleState, string> = {
  probing: "...",
  loading: "...",
  unavailable: "CPU",
  failed: "GPU failed",
  cpu: "CPU",
  "png-only": "GPU",
  gpu: "GPU",
};

function gpuOnTitle(state: GpuToggleState, cpuViewerShown: boolean): string {
  if (state === "png-only") return GPU_PNG_ONLY_TOGGLE_TITLE;
  return cpuViewerShown ? GPU_WAITING_TOGGLE_TITLE : "Rendering on GPU (WebGPU)";
}

function gpuToggleTitle(input: GpuToggleInput, state: GpuToggleState): string {
  const { available, supported, useGpu, loadError, reason, cpuViewerShown } = input;
  if (loadError) return `GPU image load failed: ${loadError} — showing the CPU viewer; click to switch to CPU`;
  if (state === "probing") return cpuViewerShown ? GPU_PROBING_CPU_VIEWER_TOGGLE_TITLE : GPU_PROBING_TOGGLE_TITLE;
  if (available === false && supported) return `${reason ?? "GPU unavailable"} — click to retry`;
  return reason ?? (useGpu ? gpuOnTitle(state, cpuViewerShown) : "Rendering on CPU — click to use GPU");
}

export function gpuToggleView(input: GpuToggleInput): GpuToggleView {
  const { useGpu, loadError } = input;
  const state = gpuToggleState(input);
  const tone: GpuToggleTone = loadError ? "failed" : state === "png-only" ? "muted" : useGpu ? "gpu" : "off";
  return { state, label: GPU_TOGGLE_LABELS[state], title: gpuToggleTitle(input, state), tone };
}

export function gpuAfterProbe(ok: boolean, savedPreference: boolean | null, current: boolean): boolean {
  if (!ok) return false;
  return savedPreference === null ? true : current;
}

export function histogramOnPath<T>(histData: T | null, histDataPath: string | null, path: string | null): T | null {
  return histDataPath !== null && histDataPath === path ? histData : null;
}

export const CUBE_SPECTRUM_HINT = "Click to extract spectrum";
export const CANVAS_HINT_CLASS =
  "absolute bottom-2 right-2 z-[6] bg-black/60 text-[10px] text-purple-300 px-2 py-1 rounded pointer-events-none whitespace-nowrap";

export function sameGrid(a: [number, number] | null, b: [number, number] | null): boolean {
  return !!a && !!b && a[0] === b[0] && a[1] === b[1];
}

export function cubeSpectrumHintShown({ isSpectralCube, fileDims, displayedDims }: {
  isSpectralCube: boolean;
  fileDims: [number, number] | null;
  displayedDims: [number, number] | null;
}): boolean {
  return isSpectralCube && sameGrid(fileDims, displayedDims);
}

export const CLICK_SLOP_PX = 4;
export const COMPARE_DIVIDER_GRAB_PX = 12;

export interface CompareDividerInput {
  compareMode: boolean;
  hasComparison: boolean;
  viewerError: boolean;
  offsetX: number;
  width: number;
  comparePos: number;
}

export function compareDividerGrabbed({ compareMode, hasComparison, viewerError, offsetX, width, comparePos }: CompareDividerInput): boolean {
  if (!compareMode || !hasComparison || viewerError) return false;
  return Math.abs(offsetX - (width * comparePos) / 100) < COMPARE_DIVIDER_GRAB_PX;
}

export interface ScreenPoint {
  x: number;
  y: number;
}

export interface ViewerClickInput {
  press: ScreenPoint | null;
  release: ScreenPoint;
  cursorMode: "pan" | "crosshair";
  onImage: boolean;
  hasPixelHandler: boolean;
  hasCanvasHandler: boolean;
}

export function viewerClickRoute({ press, release, cursorMode, onImage, hasPixelHandler, hasCanvasHandler }: ViewerClickInput): ViewportClickRoute {
  if (!press || Math.hypot(release.x - press.x, release.y - press.y) > CLICK_SLOP_PX) return "none";
  return viewportClickRoute(cursorMode, onImage, hasPixelHandler, hasCanvasHandler);
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

export type FileSwitchCompositeAction = "park" | "reset";

export function fileSwitchCompositeAction({ livePreviewUrl, previousFileRgbUrl, wizardCompositeReady }: {
  livePreviewUrl: string | null;
  previousFileRgbUrl: string | null;
  wizardCompositeReady: boolean;
}): FileSwitchCompositeAction {
  const wizardCompositeLive = livePreviewUrl !== null && livePreviewUrl !== previousFileRgbUrl;
  return wizardCompositeLive && wizardCompositeReady ? "park" : "reset";
}

export function reseedsRgbFileView({ sameFile, isRgb, previousPreviewUrl, nextPreviewUrl, livePreviewUrl }: {
  sameFile: boolean;
  isRgb: boolean;
  previousPreviewUrl: string | null;
  nextPreviewUrl: string | null;
  livePreviewUrl: string | null;
}): boolean {
  if (!sameFile || !isRgb || previousPreviewUrl === null || nextPreviewUrl === null) return false;
  return nextPreviewUrl !== previousPreviewUrl && livePreviewUrl === previousPreviewUrl;
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
  wavelength: WavelengthProbe | null;
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
  wavelength: string | null;
}

export function formatWavelength(w: WavelengthProbe): string {
  return `λ ${w.value.toFixed(w.value < 10 ? 4 : 3)}${w.unit ? ` ${w.unit}` : ""}`;
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
    wavelength: valueHere?.wavelength ? formatWavelength(valueHere.wavelength) : null,
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
    ? `${texture} For full-resolution pixels open Image > Deep Zoom.`
    : texture;
}
