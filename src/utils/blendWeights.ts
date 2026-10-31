import { effectiveBinFiles, fileFilterBand, type BlendWeight, type FrequencyBin, type WizardState } from "./wizard";
import { COLOR_BIN_IDS, REUSABLE_BIN_IDS, type ChannelSource } from "./channelMapping";

export const CANONICAL_WAVELENGTH: Record<string, number> = {
  sii: 673, ha: 656, nii: 658, oiii: 501,
  r: 620, g: 530, b: 470, l: 550,
};

const MIN_COLUMN_WEIGHT = 1e-6;

const round2 = (x: number) => Math.round(x * 100) / 100;

export type BlendColumn = "R" | "G" | "B";

export interface ColorAxes {
  r: number;
  g: number;
  b: number;
}

export function binWavelength(bin: FrequencyBin): number {
  if (bin.wavelength) return bin.wavelength;
  return CANONICAL_WAVELENGTH[bin.id] ?? 550;
}

export function blendColumnTotals(rows: ColorAxes[]): ColorAxes {
  return rows.reduce(
    (acc, w) => ({ r: acc.r + w.r, g: acc.g + w.g, b: acc.b + w.b }),
    { r: 0, g: 0, b: 0 },
  );
}

export function emptyBlendColumns(rows: ColorAxes[]): BlendColumn[] {
  const totals = blendColumnTotals(rows);
  const columns: [BlendColumn, number][] = [["R", totals.r], ["G", totals.g], ["B", totals.b]];
  return columns.filter(([, total]) => Math.abs(total) < MIN_COLUMN_WEIGHT).map(([name]) => name);
}

export function blendWeightsCoverAllColumns(rows: ColorAxes[]): boolean {
  return rows.length > 0 && emptyBlendColumns(rows).length === 0;
}

export function blendMatrixError(rows: ColorAxes[], channelLabels: string[]): string | null {
  if (rows.length === 0) {
    return "No weights assigned. Give R, G and B at least one input channel in the weight matrix.";
  }
  const empty = emptyBlendColumns(rows);
  if (empty.length === 0) return null;
  const columns = empty.join(" and ");
  const feeders = channelLabels.length > 0 ? channelLabels.join(", ") : "the assigned channels";
  return `Blend matrix leaves ${columns} empty: every ${columns} weight is zero, which renders a black plane instead of a composite. Assign a non-zero ${columns} weight to one of ${feeders}, or press Auto (λ).`;
}

const COLUMN_TOTAL_TOLERANCE = 0.05;

export function unequalColumnTotalsNote(rows: ColorAxes[]): string | null {
  const totals = blendColumnTotals(rows);
  const fed = [totals.r, totals.g, totals.b].filter((total) => Math.abs(total) >= MIN_COLUMN_WEIGHT);
  if (fed.length < 2) return null;
  const largest = Math.max(...fed);
  if (Math.min(...fed) >= largest * (1 - COLUMN_TOTAL_TOLERANCE)) return null;
  const shown = `R ${totals.r.toFixed(2)} · G ${totals.g.toFixed(2)} · B ${totals.b.toFixed(2)}`;
  return `R, G and B get unequal total weights (${shown}), so their backgrounds differ even after Match levels. Balanced (λ) gives each colour the same total weight.`;
}

export function weightsForFilledBins(weights: BlendWeight[], filledBins: FrequencyBin[]): BlendWeight[] {
  return weights.filter((w) => filledBins.some((b) => b.id === w.channelId));
}

function triangleAxes(position: number): ColorAxes {
  return {
    r: Math.max(0, 2 * position - 1),
    g: 1 - Math.abs(2 * position - 1),
    b: Math.max(0, 1 - 2 * position),
  };
}

function bicolorAxes(index: number): ColorAxes {
  return index === 0 ? { r: 0, g: 0.5, b: 0.5 } : { r: 1, g: 0, b: 0 };
}

function spectralAxes(index: number, count: number): ColorAxes {
  if (count < 2) return { r: 1, g: 1, b: 1 };
  if (count === 2) return bicolorAxes(index);
  return triangleAxes(index / (count - 1));
}

export function spectralBins(filledBins: FrequencyBin[]): FrequencyBin[] {
  return filledBins.filter((b) => !REUSABLE_BIN_IDS.has(b.id));
}

function spectralRows(filledBins: FrequencyBin[]): (ColorAxes & { channelId: string })[] {
  const sorted = spectralBins(filledBins).sort((a, b) => binWavelength(a) - binWavelength(b));
  return sorted.map((bin, i) => ({ channelId: bin.id, ...spectralAxes(i, sorted.length) }));
}

export function wavelengthAutoWeights(filledBins: FrequencyBin[]): BlendWeight[] {
  return spectralRows(filledBins).map((w) => ({
    channelId: w.channelId,
    r: round2(w.r),
    g: round2(w.g),
    b: round2(w.b),
  }));
}

export function wavelengthAutoWeightsBalanced(filledBins: FrequencyBin[]): BlendWeight[] {
  const raw = spectralRows(filledBins);
  const totals = blendColumnTotals(raw);
  const factor = (total: number) => (total > MIN_COLUMN_WEIGHT ? 1 / total : 1);
  const fr = factor(totals.r);
  const fg = factor(totals.g);
  const fb = factor(totals.b);
  return raw.map((w) => ({
    channelId: w.channelId,
    r: round2(w.r * fr),
    g: round2(w.g * fg),
    b: round2(w.b * fb),
  }));
}

function remapPresetByWavelength(presetWeights: BlendWeight[], filledBins: FrequencyBin[]): BlendWeight[] {
  const sortedPreset = [...presetWeights]
    .map((w) => ({ ...w, wl: CANONICAL_WAVELENGTH[w.channelId] ?? 550 }))
    .sort((a, b) => b.wl - a.wl);
  const sortedBins = [...filledBins].sort((a, b) => binWavelength(b) - binWavelength(a));
  return sortedPreset
    .slice(0, sortedBins.length)
    .map((pw, i) => ({ channelId: sortedBins[i].id, r: pw.r, g: pw.g, b: pw.b }));
}

interface PresetAttempt {
  rows: BlendWeight[];
  resolved: boolean;
}

function presetAttempt(preset: { weights: BlendWeight[] }, filledBins: FrequencyBin[]): PresetAttempt {
  const exact = weightsForFilledBins(preset.weights, filledBins);
  if (blendWeightsCoverAllColumns(exact)) return { rows: exact, resolved: true };

  const spectral = spectralBins(filledBins);
  if (spectral.length < 2) return { rows: exact, resolved: false };

  const remapped = remapPresetByWavelength(preset.weights, spectral);
  return { rows: remapped, resolved: remapped.length >= 2 && blendWeightsCoverAllColumns(remapped) };
}

export function resolvePresetWeights(
  preset: { weights: BlendWeight[] },
  filledBins: FrequencyBin[],
): BlendWeight[] | null {
  const attempt = presetAttempt(preset, filledBins);
  return attempt.resolved ? attempt.rows : null;
}

export type PresetPlan =
  | { available: true; weights: BlendWeight[]; mapping: string; title: string }
  | { available: false; reason: string };

const AXIS_COLUMNS: readonly (readonly [keyof ColorAxes, BlendColumn])[] = [["r", "R"], ["g", "G"], ["b", "B"]];

function joinWithAnd(names: readonly string[]): string {
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

function fedAxes(row: ColorAxes): (readonly [keyof ColorAxes, BlendColumn])[] {
  return AXIS_COLUMNS.filter(([axis]) => row[axis] > 0);
}

function binLabel(filledBins: FrequencyBin[], channelId: string): string {
  return filledBins.find((b) => b.id === channelId)?.shortLabel ?? channelId;
}

export type PresetBinNames = Readonly<Record<string, string>>;

const COLOR_BINS: ReadonlySet<string> = new Set(COLOR_BIN_IDS);

function sharedFilterCode(state: WizardState, bin: FrequencyBin, files: ReadonlyMap<string, ChannelSource>): string | null {
  const codes = new Set(effectiveBinFiles(state, bin).map((path) => fileFilterBand(files.get(path), path)?.code ?? null));
  if (codes.size !== 1) return null;
  const [code] = codes;
  return code;
}

export function presetBinNames(state: WizardState, files: readonly (ChannelSource & { path: string })[]): PresetBinNames {
  const byPath = new Map(files.map((file) => [file.path, file]));
  return Object.fromEntries(
    state.bins
      .filter((bin) => bin.files.length > 0)
      .map((bin) => [bin.id, (COLOR_BINS.has(bin.id) ? sharedFilterCode(state, bin, byPath) : null) ?? bin.shortLabel]),
  );
}

function presetMapping(rows: BlendWeight[], filledBins: FrequencyBin[], nameOf: (channelId: string) => string): string {
  const fed = rows.filter((row) => fedAxes(row).length > 0);
  const used = new Set(fed.map((row) => row.channelId));
  const applied = fed.map((row) => `${nameOf(row.channelId)}→${fedAxes(row).map(([, column]) => column).join("+")}`);
  const unused = spectralBins(filledBins).filter((b) => !used.has(b.id)).map((b) => `${nameOf(b.id)} unused`);
  return [...applied, ...unused].join(", ");
}

function presetWeightsTitle(rows: BlendWeight[], nameOf: (channelId: string) => string): string {
  const terms = rows
    .filter((row) => fedAxes(row).length > 0)
    .map((row) => `${nameOf(row.channelId)} ${fedAxes(row).map(([axis, column]) => `${row[axis].toFixed(2)} ${column}`).join(" + ")}`);
  return `Fixed weights: ${terms.join("; ")}`;
}

function presetBlockReason(filledBins: FrequencyBin[], rows: BlendWeight[], nameOf: (channelId: string) => string): string {
  const names = spectralBins(filledBins)
    .sort((a, b) => binWavelength(b) - binWavelength(a))
    .map((b) => nameOf(b.id));
  const gap = `${joinWithAnd(names)} cannot fill ${joinWithAnd(emptyBlendColumns(rows))}`;
  if (names.length === 1) return `needs a second channel: ${gap}`;
  if (names.length === 2) return `needs a third channel: ${gap}`;
  return gap;
}

export function presetPlan(
  preset: { weights: BlendWeight[] },
  filledBins: FrequencyBin[],
  binNames: PresetBinNames = {},
): PresetPlan {
  const nameOf = (channelId: string) => binNames[channelId] ?? binLabel(filledBins, channelId);
  const attempt = presetAttempt(preset, filledBins);
  if (!attempt.resolved) return { available: false, reason: presetBlockReason(filledBins, attempt.rows, nameOf) };
  return {
    available: true,
    weights: attempt.rows,
    mapping: presetMapping(attempt.rows, filledBins, nameOf),
    title: presetWeightsTitle(attempt.rows, nameOf),
  };
}
