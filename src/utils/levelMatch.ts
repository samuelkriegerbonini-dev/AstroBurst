import type { BlendWeight, FilterDetectionRef, FrequencyBin, LevelMatchEntry, SpccInput, WizardState } from "./wizard";
import { effectiveBinFiles, fileFilterBand, narrowbandFileLabel, NARROWBAND_IDS, resolveChannelPath, spccInputs } from "./wizard";
import type { ChannelSource } from "./channelMapping";
import { WB_APPLY_MAX, WB_APPLY_MIN, wbFactorsOutOfRange } from "./whiteBalanceRange";

export const LEVEL_PERCENTILES: readonly [number, number] = [50, 99.5];

export const WIZARD_CACHE_PREFIX = "__wizard_ch_";

export interface BackendBlendWeight {
  channelIdx: number;
  r: number;
  g: number;
  b: number;
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
  level: number;
}

export type LevelScalesOutcome = { scales: Record<string, number> } | { error: string };

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
  levelsByPath: Readonly<Record<string, number>>,
  labelOf: (binId: string) => string,
): ChannelLevel[] {
  return usedChannelIndices(request).map((i) => {
    const binId = request.channelOrder[i];
    return { binId, label: labelOf(binId), level: levelsByPath[request.paths[i]] ?? Number.NaN };
  });
}

export function channelLevel(limits: { vmin: number; vmax: number }): number {
  return limits.vmax - limits.vmin;
}

export function levelScales(levels: readonly ChannelLevel[]): LevelScalesOutcome {
  const bad = levels.find((l) => !Number.isFinite(l.level) || l.level <= 0);
  if (bad) {
    return {
      error: `Match levels needs signal above the sky in every channel: ${bad.label} has p99.5 - p50 = ${bad.level}. Turn Match levels off or check that channel.`,
    };
  }
  const max = Math.max(...levels.map((l) => l.level));
  const scales: Record<string, number> = {};
  for (const l of levels) scales[l.binId] = max / l.level;
  return { scales };
}

export function scaleBlendWeights(
  weights: readonly BackendBlendWeight[],
  channelOrder: readonly string[],
  scales: Readonly<Record<string, number>> | null,
): BackendBlendWeight[] {
  if (scales === null) return weights.map((w) => ({ ...w }));
  return weights.map((w) => {
    const k = scales[channelOrder[w.channelIdx]] ?? 1;
    return { channelIdx: w.channelIdx, r: w.r * k, g: w.g * k, b: w.b * k };
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
  scales: Readonly<Record<string, number>>,
): LevelMatchEntry[] {
  return state.bins.flatMap((bin) => {
    const scale = scales[bin.id];
    return scale === undefined ? [] : [{ channel: levelMatchChannelLabel(state, bin.id, files), scale }];
  });
}

export function levelMatchSummary(entries: readonly LevelMatchEntry[]): string {
  return `Level match: ${entries.map((e) => `${e.channel} x${e.scale.toPrecision(4)}`).join(", ")}`;
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
  scales: Readonly<Record<string, number>> | null,
): { factors: RgbFactors; rescale: number } {
  const divide = (key: "r" | "g" | "b") => factors[key] / (scales?.[inputs[key]?.binId ?? ""] ?? 1);
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
  scales: Readonly<Record<string, number>> | null,
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
