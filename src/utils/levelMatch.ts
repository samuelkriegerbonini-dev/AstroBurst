import type { BlendWeight, FilterDetectionRef, FrequencyBin, LevelMatchEntry, LevelScale, SpccInput, WizardState } from "./wizard";
import { effectiveBinFiles, fileFilterBand, levelMatchFactors, narrowbandFileLabel, NARROWBAND_IDS, resolveChannelPath, spccInputs } from "./wizard";
import type { ChannelSource } from "./channelMapping";
import { WB_APPLY_MAX, WB_APPLY_MIN, wbFactorsOutOfRange } from "./whiteBalanceRange";
import type { BlendResult } from "../shared/types/compose";
import type { StfParams } from "../shared/types";

export const WIZARD_CACHE_PREFIX = "__wizard_ch_";

export interface BackendBlendWeight {
  channelIdx: number;
  r: number;
  g: number;
  b: number;
  offset?: number;
}

export interface BlendRequest {
  channelOrder: string[];
  paths: string[];
  weights: BackendBlendWeight[];
}

export interface LevelTarget {
  binId: string;
  path: string;
}

export interface ChannelLevel {
  binId: string;
  label: string;
  median: number;
  mad: number;
}

export interface MeasuredLevel {
  median: number;
  mad: number;
}

export type LevelScales = Record<string, LevelScale>;

export type LevelScalesOutcome = { scales: LevelScales } | { error: string };

export interface BlendCompositeStf {
  r: StfParams;
  g: StfParams;
  b: StfParams;
  linked: boolean;
}

export const LEVEL_SCALE_LIMIT = 1e4;

export const LEVEL_MATCH_HELP =
  "Matches each channel's background (median) and diffuse-signal spread (MAD) before blending, so a bright narrowband filter does not swamp the others. On by default when a narrowband filter is loaded.";

export const SPCC_BG_NOTE =
  "With Match levels on, the three channels share one background level before SPCC; SPCC then scales the whole composite, so the exported background is neutral only if the SPCC factors are equal. Run Background before Blend for a neutral background after SPCC.";

type LevelFile = ChannelSource & { path: string };

export function blendRequest(
  state: WizardState,
  filledBins: readonly FrequencyBin[],
  activeWeights: readonly BlendWeight[],
): BlendRequest {
  const channelOrder: string[] = [];
  const paths: string[] = [];
  for (const bin of filledBins) {
    const p = resolveChannelPath(state, bin.id);
    if (p) {
      channelOrder.push(bin.id);
      paths.push(p);
    }
  }
  const weights = activeWeights
    .filter((w) => w.r > 0 || w.g > 0 || w.b > 0)
    .flatMap((w) => {
      const idx = channelOrder.indexOf(w.channelId);
      return idx === -1 ? [] : [{ channelIdx: idx, r: w.r, g: w.g, b: w.b }];
    });
  return { channelOrder, paths, weights };
}

function usedChannelIndices(request: BlendRequest): number[] {
  const used = new Set(request.weights.map((w) => w.channelIdx));
  return request.channelOrder.flatMap((_, i) => (used.has(i) ? [i] : []));
}

export function levelTargets(request: BlendRequest): LevelTarget[] {
  const seen = new Set<string>();
  const targets: LevelTarget[] = [];
  for (const i of usedChannelIndices(request)) {
    const path = request.paths[i];
    if (seen.has(path)) continue;
    seen.add(path);
    targets.push({ binId: request.channelOrder[i], path });
  }
  return targets;
}

export function channelLevels(
  request: BlendRequest,
  levelsByPath: Readonly<Record<string, MeasuredLevel>>,
  labelOf: (binId: string) => string,
): ChannelLevel[] {
  return usedChannelIndices(request).map((i) => {
    const binId = request.channelOrder[i];
    const measured = levelsByPath[request.paths[i]];
    return { binId, label: labelOf(binId), median: measured?.median ?? Number.NaN, mad: measured?.mad ?? Number.NaN };
  });
}

export function levelScales(levels: readonly ChannelLevel[]): LevelScalesOutcome {
  const bad = levels.find((l) => !Number.isFinite(l.mad) || l.mad <= 0);
  if (bad) {
    return {
      error: `Match levels needs a measurable noise level in every channel: ${bad.label} has MAD = ${bad.mad} (constant, empty or quantised data). Turn Match levels off or check that channel.`,
    };
  }
  const scales: LevelScales = {};
  if (levels.length === 0) return { scales };
  const reference = levels.reduce((best, l) => (l.mad > best.mad ? l : best));
  for (const l of levels) {
    const k = reference.mad / l.mad;
    if (k > LEVEL_SCALE_LIMIT) {
      return {
        error: `Match levels would scale ${l.label} by x${Math.round(k).toLocaleString("en-US")}; the channels differ by more than 10,000x. Turn Match levels off or check the channel assignment.`,
      };
    }
    const offset = reference.median / k - l.median;
    scales[l.binId] = { k, offset, z: k * offset };
  }
  return { scales };
}

export function scaleBlendWeights(
  weights: readonly BackendBlendWeight[],
  channelOrder: readonly string[],
  scales: Readonly<LevelScales> | null,
): BackendBlendWeight[] {
  if (scales === null) return weights.map((w) => ({ ...w }));
  return weights.map((w) => {
    const s = scales[channelOrder[w.channelIdx]];
    if (s === undefined) return { ...w };
    return { channelIdx: w.channelIdx, r: w.r * s.k, g: w.g * s.k, b: w.b * s.k, offset: s.offset };
  });
}

export function levelMatchDefault(
  state: WizardState,
  files: readonly LevelFile[],
  detections: readonly FilterDetectionRef[] = [],
): boolean {
  const filled = state.bins.filter((b) => b.files.length > 0);
  if (filled.some((b) => NARROWBAND_IDS.has(b.id))) return true;
  return filled.some((bin) =>
    bin.files.some((path) =>
      narrowbandFileLabel(files.find((f) => f.path === path), detections.find((d) => d.path === path), path) !== null,
    ),
  );
}

export function levelMatchEnabled(
  state: WizardState,
  files: readonly LevelFile[],
  detections: readonly FilterDetectionRef[] = [],
): boolean {
  return state.levelMatch ?? levelMatchDefault(state, files, detections);
}

export function levelMatchChannelLabel(state: WizardState, binId: string, files: readonly LevelFile[]): string {
  const bin = state.bins.find((b) => b.id === binId);
  const path = bin ? effectiveBinFiles(state, bin)[0] : undefined;
  const code = path ? fileFilterBand(files.find((f) => f.path === path), path)?.code : undefined;
  return code ? `${binId} ${code}` : binId;
}

export function levelMatchEntries(
  state: WizardState,
  files: readonly LevelFile[],
  scales: Readonly<LevelScales>,
): LevelMatchEntry[] {
  return state.bins.flatMap((bin) => {
    const s = scales[bin.id];
    return s === undefined ? [] : [{ channel: levelMatchChannelLabel(state, bin.id, files), scale: s.k, z: s.z }];
  });
}

export function levelMatchSummary(entries: readonly LevelMatchEntry[]): string {
  return `Level match: ${entries.map((e) => `${e.channel} ${levelMatchFactors(e)}`).join(", ")}`;
}

export function spccBackgroundNote(state: WizardState, matchOn: boolean): string | null {
  return matchOn && state.wbMode === "spcc" ? SPCC_BG_NOTE : null;
}

export function blendCompositeStf(res: BlendResult): BlendCompositeStf | undefined {
  const { stf_r: r, stf_g: g, stf_b: b } = res;
  return r && g && b ? { r, g, b, linked: res.stf_linked ?? true } : undefined;
}

export function levelMeasureError(label: string, path: string, message: string): string {
  return path.startsWith(WIZARD_CACHE_PREFIX)
    ? `${label} is no longer in memory; run Align again (or the Crop or BG step that produced it).`
    : `Measuring levels for ${label} failed: ${message}`;
}

type RgbFactors = { r: number; g: number; b: number };

function spccLevelFit(
  factors: RgbFactors,
  inputs: Readonly<Record<"r" | "g" | "b", SpccInput | null>>,
  scales: Readonly<LevelScales> | null,
): { factors: RgbFactors; rescale: number } {
  const divide = (key: "r" | "g" | "b") => factors[key] / (scales?.[inputs[key]?.binId ?? ""]?.k ?? 1);
  const divided = { r: divide("r"), g: divide("g"), b: divide("b") };
  const values = [divided.r, divided.g, divided.b];
  if (wbFactorsOutOfRange(divided.r, divided.g, divided.b).length === 0) return { factors: divided, rescale: 1 };
  if (!values.every((v) => Number.isFinite(v) && v > 0)) return { factors: divided, rescale: 1 };
  const rescale = 1 / Math.sqrt(Math.min(...values) * Math.max(...values));
  return { factors: { r: divided.r * rescale, g: divided.g * rescale, b: divided.b * rescale }, rescale };
}

export function spccFactorsForLevels(
  factors: RgbFactors,
  inputs: Readonly<Record<"r" | "g" | "b", SpccInput | null>>,
  scales: Readonly<LevelScales> | null,
): RgbFactors {
  return spccLevelFit(factors, inputs, scales).factors;
}

export const SPCC_LEVEL_NOTE = "SPCC factors are divided by the Blend level-match scales.";

export function spccLevelNote(state: WizardState): string | null {
  if (state.wbMode !== "spcc" || state.spccFactors === null || state.blendLevelScales === null) return null;
  const { rescale } = spccLevelFit(state.spccFactors, spccInputs(state), state.blendLevelScales);
  if (rescale === 1) return SPCC_LEVEL_NOTE;
  return `SPCC factors are divided by the Blend level-match scales, then all three are scaled x${rescale.toPrecision(4)} to stay within [${WB_APPLY_MIN}, ${WB_APPLY_MAX}]; the colour ratios are unchanged.`;
}

export function withSpccWb(state: WizardState): WizardState {
  if (state.wbMode !== "spcc" || state.spccFactors === null) return state;
  const f = spccFactorsForLevels(state.spccFactors, spccInputs(state), state.blendLevelScales);
  if (f.r === state.wbR && f.g === state.wbG && f.b === state.wbB) return state;
  return { ...state, wbR: f.r, wbG: f.g, wbB: f.b };
}
