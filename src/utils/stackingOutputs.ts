import type { ProcessedResult } from "../shared/types/preview";
import type { DrizzleRgbResult, PipelineChannelStats, PipelineMasterOutput, PipelineResult, PipelineRgbPalette, StackResult } from "../shared/types/stacking";
import { samePath } from "./processingChain";

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? "";
}

export function frameStem(path: string): string {
  const stem = baseName(path)
    .replace(/\.(gz|fz|bz2)$/i, "")
    .replace(/\.[^.]*$/, "")
    .replace(/[^A-Za-z0-9._-]+/g, "_")
    .replace(/^\.+/, "");
  return stem || "frame";
}

function pad(value: number, width = 2): string {
  return String(value).padStart(width, "0");
}

function runTimestamp(at: Date): string {
  const date = `${at.getFullYear()}${pad(at.getMonth() + 1)}${pad(at.getDate())}`;
  const time = `${pad(at.getHours())}${pad(at.getMinutes())}${pad(at.getSeconds())}-${pad(at.getMilliseconds(), 3)}`;
  return `${date}-${time}`;
}

export function stackOutputName(firstFramePath: string, frameCount: number, at: Date): string {
  return `${frameStem(firstFramePath)}_stack${frameCount}_${runTimestamp(at)}`;
}

export function pipelineOutputName(firstFramePath: string, frameCount: number, at: Date): string {
  return `${frameStem(firstFramePath)}_pipeline${frameCount}_${runTimestamp(at)}`;
}

export function drizzleOutputName(firstFramePath: string, frameCount: number, at: Date): string {
  return `${frameStem(firstFramePath)}_drizzle${frameCount}_${runTimestamp(at)}`;
}

export function framesLabel(prefix: string, frameCount: number): string {
  return `${prefix} · ${frameCount} frame${frameCount === 1 ? "" : "s"}`;
}

export const PIPELINE_RGB_CHOICE = "RGB";
const LUMINANCE_LANE = "L";

interface CompositePalette {
  planes: readonly [string, string, string];
  luminance: boolean;
}

const PIPELINE_PALETTES: Record<PipelineRgbPalette, CompositePalette> = {
  RGB: { planes: ["R", "G", "B"], luminance: true },
  SHO: { planes: ["SII", "Ha", "OIII"], luminance: false },
  HOO: { planes: ["Ha", "OIII", "OIII"], luminance: false },
};

export interface StackViewOutput {
  output: Omit<ProcessedResult, "label">;
  label: string;
}

export function stackedLights(channel: PipelineChannelStats): number {
  return channel.lights_input - (channel.excluded_frames?.length ?? 0);
}

function channelLights(result: PipelineResult, label: string): number {
  const channel = result.stats.channels.find((c) => c.label === label);
  return channel ? stackedLights(channel) : 0;
}

function masterNamed(masters: readonly PipelineMasterOutput[], label: string): PipelineMasterOutput | undefined {
  return masters.find((m) => m.label.toUpperCase() === label.toUpperCase());
}

function sameDims(a: PipelineMasterOutput, b: PipelineMasterOutput): boolean {
  return a.dimensions[0] === b.dimensions[0] && a.dimensions[1] === b.dimensions[1];
}

function compositeLanes(palette: CompositePalette, planes: readonly PipelineMasterOutput[] | null, luminance: PipelineMasterOutput | undefined): Set<string> {
  const lanes = new Set(palette.planes.map((label) => label.toUpperCase()));
  if (palette.luminance && planes && luminance && planes.every((plane) => sameDims(plane, luminance))) lanes.add(LUMINANCE_LANE);
  return lanes;
}

function composedDims(result: PipelineResult, planes: readonly PipelineMasterOutput[] | null): [number, number] | null {
  if (!planes) return toDims(result.rgb_dimensions);
  return toDims([Math.min(...planes.map((m) => m.dimensions[0])), Math.min(...planes.map((m) => m.dimensions[1]))]);
}

function compositeViewOutput(result: PipelineResult, masters: readonly PipelineMasterOutput[]): StackViewOutput | null {
  const name = result.rgb_palette ?? PIPELINE_RGB_CHOICE;
  const palette = PIPELINE_PALETTES[name];
  const found = palette.planes.map((label) => masterNamed(masters, label));
  const reference = found.find((m) => m !== undefined);
  if (!result.rgbPreviewUrl || !reference) return null;
  const planes = found.every((m): m is PipelineMasterOutput => m !== undefined) ? found : null;
  const lanes = compositeLanes(palette, planes, masterNamed(masters, LUMINANCE_LANE));
  const lights = result.stats.channels.filter((c) => lanes.has(c.label.toUpperCase())).reduce((sum, c) => sum + stackedLights(c), 0);
  return {
    output: { fitsPath: null, previewUrl: result.rgbPreviewUrl, dimensions: composedDims(result, planes), kind: "stacking", inputPath: reference.input_path },
    label: framesLabel(`Pipeline ${name}`, lights),
  };
}

export function pipelineViewOutput(result: PipelineResult, choice: string): StackViewOutput | null {
  const masters = result.masters ?? [];
  if (choice === PIPELINE_RGB_CHOICE) return compositeViewOutput(result, masters);
  const master = masters.find((m) => m.label === choice);
  if (!master) return null;
  return {
    output: {
      fitsPath: master.fits_path,
      previewUrl: master.previewUrl ?? null,
      dimensions: toDims(master.dimensions),
      kind: "stacking",
      inputPath: master.input_path,
    },
    label: framesLabel(`Pipeline ${choice}`, channelLights(result, choice)),
  };
}

export function stackViewOutput(result: StackResult, inputs: readonly string[], targetPath: string, prefix: string): StackViewOutput {
  return {
    output: { fitsPath: result.fits_path ?? null, previewUrl: result.previewUrl ?? null, dimensions: toDims(result.dimensions), kind: "stacking", inputPath: inputs[0] ?? targetPath },
    label: framesLabel(prefix, result.frame_count ?? inputs.length),
  };
}

export function drizzleViewOutput(result: DrizzleRgbResult, inputs: readonly string[], targetPath: string): StackViewOutput {
  const frames = result.frame_count_r + result.frame_count_g + result.frame_count_b;
  return {
    output: { fitsPath: null, previewUrl: result.previewUrl ?? null, dimensions: toDims(result.output_dims), kind: "stacking", inputPath: inputs[0] ?? targetPath },
    label: framesLabel("Drizzle RGB", frames || inputs.length),
  };
}

export function pipelineInitialChoice(result: PipelineResult): string | null {
  if (result.rgb_preview) return PIPELINE_RGB_CHOICE;
  return result.channel_previews[0]?.label ?? null;
}

export interface WizardParkInput {
  targetKey: string;
  currentKey: string | null;
  wizardCompositeOnScreen: boolean;
  wizardCompositeReady: boolean;
}

export function parksWizardComposite(input: WizardParkInput): boolean {
  return input.targetKey === input.currentKey && input.wizardCompositeOnScreen && input.wizardCompositeReady;
}

export function sourceLabel(prefix: string, sourcePath: string, currentPath: string): string {
  return samePath(sourcePath, currentPath) ? prefix : `${prefix} · ${baseName(sourcePath)}`;
}

export interface OutputRecipient {
  key: string;
  path: string;
}

export function resultsForRecipients(
  target: OutputRecipient,
  alsoShowing: readonly OutputRecipient[],
  output: Omit<ProcessedResult, "label">,
  labelFor: (recipientPath: string) => string,
): { key: string; result: ProcessedResult }[] {
  const recipients = [target, ...alsoShowing.filter((r) => r.key !== target.key)];
  return recipients.map((r) => ({ key: r.key, result: { ...output, label: labelFor(r.path) } }));
}

export function displaysFileGrid(processed: Pick<ProcessedResult, "kind" | "inputPath"> | null, filePath: string): boolean {
  return !processed || processed.kind !== "stacking" || samePath(processed.inputPath, filePath);
}

export function otherGridHint(label: string | null): string {
  return `The preview shows ${label ?? "a result"}, built on another frame's pixel grid. Points drawn on it stay in that grid after a reset: reset the preview, then place the Points on this frame before measuring.`;
}

export function toDims(dims: readonly number[] | null | undefined): [number, number] | null {
  return dims && dims.length === 2 && dims[0] > 0 && dims[1] > 0 ? [dims[0], dims[1]] : null;
}

function withoutQuery(url: string): string {
  const cut = url.search(/[?#]/);
  return cut >= 0 ? url.slice(0, cut) : url;
}

export function showsOutput(
  processed: ProcessedResult | null | undefined,
  output: Pick<ProcessedResult, "fitsPath" | "previewUrl">,
): boolean {
  if (!processed) return false;
  if (processed.fitsPath !== null || output.fitsPath !== null) {
    return processed.fitsPath !== null && output.fitsPath !== null && samePath(processed.fitsPath, output.fitsPath);
  }
  return (
    processed.previewUrl !== null &&
    output.previewUrl !== null &&
    samePath(withoutQuery(processed.previewUrl), withoutQuery(output.previewUrl))
  );
}
