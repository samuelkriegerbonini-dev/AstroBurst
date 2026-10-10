import { sameGrid } from "./previewShell";

export interface CompositeViewInput {
  compositePreviewUrl: string | null;
  fileIsRgb: boolean;
  filePreviewUrl: string | null;
  hasProcessed: boolean;
}

export function isFileRgbView(input: Omit<CompositeViewInput, "hasProcessed">): boolean {
  return !!input.compositePreviewUrl && input.fileIsRgb && input.compositePreviewUrl === input.filePreviewUrl;
}

export function isCompositeOnScreen(input: CompositeViewInput): boolean {
  if (!input.compositePreviewUrl) return false;
  return !(isFileRgbView(input) && input.hasProcessed);
}

export type MeasurementTone = "processed" | "original" | "composite";

export interface MeasurementSource {
  text: string;
  title: string;
  tone: MeasurementTone;
}

export interface RgbMeasureInput {
  composite: boolean;
  fileRgbView: boolean;
  filePath: string | null;
}

export function rgbMeasurePath(input: RgbMeasureInput): string | null {
  return input.composite && input.fileRgbView ? input.filePath : null;
}

export interface DetectedStarsInput {
  compositeOnScreen: boolean;
  measuresFilePlanes: boolean;
}

export function detectedStarsOnMeasuredImage(input: DetectedStarsInput): boolean {
  return !input.compositeOnScreen || input.measuresFilePlanes;
}

export interface StarDetectionScopeInput {
  path: string | null;
  composite: boolean;
  rgbPath: string | null;
  compositeVersion: number;
}

export function starDetectionScope(input: StarDetectionScopeInput): string {
  if (!input.composite) return `image:${input.path ?? ""}`;
  if (input.rgbPath !== null) return `rgb-file:${input.rgbPath}`;
  return `wizard-composite:${input.compositeVersion}:${input.path ?? ""}`;
}

export interface MeasureKeyInput {
  path: string | null;
  composite: boolean;
  processedFitsPath: string | null;
  processedVersion: number;
}

export function analysisMeasureKey(input: MeasureKeyInput): string | null {
  if (!input.path) return null;
  if (input.composite || input.processedFitsPath !== input.path) return input.path;
  return `${input.path}@${input.processedVersion}`;
}

export function statisticsRunKey(input: {
  filePath: string | null;
  composite: boolean;
  rgbPath: string | null;
  measureKey: string | null;
}): string {
  return `${input.measureKey ?? input.filePath ?? ""}|${input.composite ? 1 : 0}|${input.rgbPath ?? ""}`;
}

export function regionStatsRunKey(filePath: string | null, measureKey: string | null): string {
  return measureKey ?? filePath ?? "";
}

export interface WcsProbeInput {
  path: string | null;
  dimensions: [number, number] | null;
  previewOnly: boolean;
  fileDims: [number, number] | null;
}

export interface WcsProbe {
  path: string;
  dimensions: [number, number];
}

export function wcsProbeTarget({ path, dimensions, previewOnly, fileDims }: WcsProbeInput): WcsProbe | null {
  if (!path || !dimensions) return null;
  if (previewOnly && !sameGrid(fileDims, dimensions)) return null;
  return { path, dimensions };
}

export interface MeasurementSourceInput {
  measuresComposite: boolean;
  compositeOnScreen: boolean;
  measuresFilePlanes: boolean;
  fileName: string | null;
  processedLabel: string | null;
  previewOnly: boolean;
}

export function describeMeasurementSource(input: MeasurementSourceInput): MeasurementSource | null {
  const name = input.fileName ?? "the selected file";
  if (input.compositeOnScreen) {
    if (!input.measuresComposite) {
      return { text: "selected file", tone: "original", title: `Measured on ${name}, not on the RGB view on screen` };
    }
    if (input.measuresFilePlanes) {
      return { text: "RGB file", tone: "original", title: `Measured on the R, G and B planes of ${name}` };
    }
    return {
      text: "composite",
      tone: "composite",
      title:
        "Measured on the linear planes of the RGB composite (the wizard Blend and the linear steps applied after it), not on the stretch, curves or local contrast shown on screen",
    };
  }
  if (!input.processedLabel) return null;
  if (input.previewOnly) {
    return {
      text: "original",
      tone: "original",
      title: `Measured on the original file: ${input.processedLabel} is a PNG-only result with no data to measure`,
    };
  }
  return {
    text: input.processedLabel,
    tone: "processed",
    title: `Measured on the processed result (${input.processedLabel}), not on the original file`,
  };
}

export const STF_LOCK_STRETCH = "STF applies only to the mtf stretch";
export const STF_LOCK_PNG = "PNG-only result; use Revert to original in the preview header to get the display controls back";
export const STF_LOCK_RGB = "RGB view on screen: this mono histogram does not drive it; use the RGB channel STF";

export interface StfLockInput {
  stretch: string;
  compositeOnScreen: boolean;
  previewOnly: boolean;
}

export function histogramStfLock(input: StfLockInput): string | null {
  if (input.compositeOnScreen) return STF_LOCK_RGB;
  if (input.previewOnly) return STF_LOCK_PNG;
  if (input.stretch !== "mtf") return STF_LOCK_STRETCH;
  return null;
}

export interface CpuStfInput {
  hasRawPixels: boolean;
  rawPixelsLoading: boolean;
  locked: boolean;
}

export function cpuStfRenderAllowed(input: CpuStfInput): boolean {
  return !input.hasRawPixels && !input.rawPixelsLoading && !input.locked;
}

export type RgbStfPanelMode = "hidden" | "live" | "baked";

export interface RgbStfPanelInput {
  compositeOnScreen: boolean;
  hasRgbRawPixels: boolean;
  displayReferred: boolean;
}

export function rgbStfPanelMode(input: RgbStfPanelInput): RgbStfPanelMode {
  if (!input.compositeOnScreen || !input.hasRgbRawPixels) return "hidden";
  return input.displayReferred ? "baked" : "live";
}
