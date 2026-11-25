import { describe, expect, it } from "vitest";
import {
  backgroundHistory,
  deconvHistory,
  HISTORY_CARD_WIDTH,
  historyLines,
  linearChainHistory,
  pixelMathHistory,
  waveletHistory,
} from "../compositeChainHistory";
import { EMPTY_COMPOSITE_CHAIN } from "../compositeChain";
import type { CompositeChain } from "../../shared/types/compositeChain";

describe("chain step HISTORY lines", () => {
  it("names each linear step with its parameters", () => {
    expect(backgroundHistory({ model: "polynomial", gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract", dbe: null })).toEqual([
      "Background polynomial: grid 8, degree 3, sigma 2.5, 3 iter, subtract",
    ]);
    expect(waveletHistory({ numScales: 4, thresholds: [3, 2.5, 2, 1.5, 1], linear: true, layerBias: [0, 0, 0, 0, 0], mask: null })).toEqual([
      "Wavelet denoise: 4 scales, thresholds 3/2.5/2/1.5 sigma, soft",
    ]);
    expect(deconvHistory({ iterations: 20, psf: "gaussian", psfSigma: 2, psfSize: 15, regularization: 0.001, deringing: true, deringThreshold: 0.1, mask: "star_mask" })).toEqual([
      "Deconvolution RL: 20 iterations, PSF gaussian sigma 2 px size 15",
      "Deconvolution RL (cont.): regularization 0.001, deringing 0.1, masked",
    ]);
  });

  it("splits a long PixelMath expression over cards that fit the 70 character HISTORY value", () => {
    const expression = "iif($T > 0.5, $T * 0.8 + $Ha * 0.2, $T * 0.95 + $Ha * 0.05) - 0.001 * $OIII";
    const lines = pixelMathHistory({ expression, truncate: true, rescale: false, mask: null });
    for (const line of lines) expect(line.length).toBeLessThanOrEqual(HISTORY_CARD_WIDTH);
    const body = lines.map((line, i) => line.replace(i === 0 ? "PixelMath: " : "PixelMath (cont.): ", "")).join("");
    expect(body).toBe(`${expression}, truncate`);
  });

  it("writes the bare label when there are no parameters", () => {
    expect(historyLines("Background", [])).toEqual(["Background"]);
  });

  it("lists only the steps that left the composite linear, in chain order", () => {
    const entry = (label: string, displayed: "linear" | "stretched", history?: string[]) => ({ previewUrl: `asset://${label}.png`, label, displayed, history });
    const chain: CompositeChain = {
      ...EMPTY_COMPOSITE_CHAIN,
      steps: {
        stretch: entry("Stretch", "stretched", ["Stretch: arcsinh"]),
        deconv: entry("Deconvolved", "linear", ["Deconvolution RL: 20 iterations"]),
        background: entry("Background Removed", "linear"),
      },
    };
    expect(linearChainHistory(chain)).toEqual(["Background Removed", "Deconvolution RL: 20 iterations"]);
  });
});
