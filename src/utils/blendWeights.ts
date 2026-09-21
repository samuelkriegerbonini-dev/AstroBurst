import type { BlendWeight, FrequencyBin } from "./wizard";

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

function spectralRows(filledBins: FrequencyBin[]): (ColorAxes & { channelId: string })[] {
  const sorted = [...filledBins].sort((a, b) => binWavelength(a) - binWavelength(b));
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

export function resolvePresetWeights(
  preset: { weights: BlendWeight[] },
  filledBins: FrequencyBin[],
): BlendWeight[] | null {
  const exact = weightsForFilledBins(preset.weights, filledBins);
  if (blendWeightsCoverAllColumns(exact)) return exact;

  if (filledBins.length < 2) return null;

  const remapped = remapPresetByWavelength(preset.weights, filledBins);
  if (remapped.length < 2 || !blendWeightsCoverAllColumns(remapped)) return null;
  return remapped;
}
