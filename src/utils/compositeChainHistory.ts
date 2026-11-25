import type { CompositeChain } from "../shared/types/compositeChain";
import type { DbeConfig, DbeMode } from "../shared/types/dbe";
import { CHAIN_ORDER } from "./processingChain";

export const HISTORY_CARD_WIDTH = 70;

export function historyLines(label: string, params: readonly string[]): string[] {
  const cont = `${label} (cont.):`;
  const lines: string[] = [];
  let line = `${label}:`;
  let started = false;
  for (const param of params) {
    const sep = started ? ", " : " ";
    if (line.length + sep.length + param.length <= HISTORY_CARD_WIDTH) {
      line += sep + param;
      started = true;
      continue;
    }
    if (started) {
      lines.push(line);
      line = cont;
    }
    let rest = param;
    while (line.length + 1 + rest.length > HISTORY_CARD_WIDTH) {
      const take = HISTORY_CARD_WIDTH - line.length - 1;
      lines.push(`${line} ${rest.slice(0, take)}`);
      rest = rest.slice(take);
      line = cont;
    }
    line += ` ${rest}`;
    started = true;
  }
  lines.push(started ? line : label);
  return lines;
}

const masked = (mask: unknown) => (mask ? ["masked"] : []);

export function backgroundHistory(input: {
  model: string;
  gridSize: number;
  polyDegree: number;
  sigmaClip: number;
  iterations: number;
  mode: DbeMode;
  dbe: DbeConfig | null;
}): string[] {
  if (input.dbe) {
    const d = input.dbe;
    return historyLines("Background DBE", [
      `sample radius ${d.sampleRadius}`,
      `tolerance ${d.tolerance}`,
      `smoothing ${d.smoothing}`,
      `grid ${d.autoGrid ?? "auto"}`,
      `max samples ${d.maxSamples}`,
      d.rejectStars ? "stars rejected" : "stars kept",
      input.mode,
    ]);
  }
  return historyLines(`Background ${input.model}`, [
    `grid ${input.gridSize}`,
    `degree ${input.polyDegree}`,
    `sigma ${input.sigmaClip}`,
    `${input.iterations} iter`,
    input.mode,
  ]);
}

export function waveletHistory(input: {
  numScales: number;
  thresholds: readonly number[];
  linear: boolean;
  layerBias: readonly number[];
  mask: unknown;
}): string[] {
  const bias = input.layerBias.slice(0, input.numScales);
  return historyLines("Wavelet denoise", [
    `${input.numScales} scales`,
    `thresholds ${input.thresholds.slice(0, input.numScales).join("/")} sigma`,
    input.linear ? "soft" : "hard",
    ...(bias.some((b) => b !== 0) ? [`bias ${bias.join("/")}`] : []),
    ...masked(input.mask),
  ]);
}

export function deconvHistory(input: {
  iterations: number;
  psf: string;
  psfSigma: number;
  psfSize: number;
  regularization: number;
  deringing: boolean;
  deringThreshold: number;
  mask: unknown;
}): string[] {
  const psf = input.psf === "gaussian" ? `PSF gaussian sigma ${input.psfSigma} px size ${input.psfSize}` : `PSF ${input.psf} size ${input.psfSize}`;
  return historyLines("Deconvolution RL", [
    `${input.iterations} iterations`,
    psf,
    `regularization ${input.regularization}`,
    input.deringing ? `deringing ${input.deringThreshold}` : "no deringing",
    ...masked(input.mask),
  ]);
}

export function pixelMathHistory(input: { expression: string; truncate: boolean; rescale: boolean; mask: unknown }): string[] {
  return historyLines("PixelMath", [
    input.expression.trim(),
    ...(input.truncate ? ["truncate"] : []),
    ...(input.rescale ? ["rescale"] : []),
    ...masked(input.mask),
  ]);
}

export function linearChainHistory(chain: CompositeChain): string[] {
  return CHAIN_ORDER.flatMap((step) => {
    const entry = chain.steps[step];
    if (!entry || entry.displayed !== "linear") return [];
    return entry.history && entry.history.length > 0 ? [...entry.history] : [entry.label];
  });
}
