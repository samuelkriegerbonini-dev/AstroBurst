import { describe, it, expect } from "vitest";

import {
  blendMatrixError,
  blendWeightsCoverAllColumns,
  emptyBlendColumns,
  resolvePresetWeights,
  wavelengthAutoWeights,
  wavelengthAutoWeightsBalanced,
  weightsForFilledBins,
} from "../blendWeights";
import { BLEND_PRESETS, type FrequencyBin } from "../wizard";

const bin = (id: string, wavelength?: number): FrequencyBin => ({
  id,
  label: id,
  shortLabel: id,
  wavelength,
  color: "#fff",
  files: ["a.fits"],
});

describe("wavelengthAutoWeights", () => {
  it("keeps every output channel fed with exactly two filled bins", () => {
    const weights = wavelengthAutoWeights([bin("ha", 656), bin("oiii", 501)]);

    expect(emptyBlendColumns(weights)).toEqual([]);
    expect(weights).toEqual([
      { channelId: "oiii", r: 0, g: 0.5, b: 0.5 },
      { channelId: "ha", r: 1, g: 0, b: 0 },
    ]);
  });

  it("still spreads three or more bins across the spectral triangle", () => {
    const weights = wavelengthAutoWeights([bin("sii", 673), bin("ha", 656), bin("oiii", 501)]);

    expect(weights).toEqual([
      { channelId: "oiii", r: 0, g: 0, b: 1 },
      { channelId: "ha", r: 0, g: 1, b: 0 },
      { channelId: "sii", r: 1, g: 0, b: 0 },
    ]);
  });

  it("never leaves a column empty for bin counts the auto map produces", () => {
    for (let count = 2; count <= 8; count += 1) {
      const bins = Array.from({ length: count }, (_, i) => bin(`wl${i}`, 400 + i * 100));
      expect(emptyBlendColumns(wavelengthAutoWeights(bins))).toEqual([]);
      expect(emptyBlendColumns(wavelengthAutoWeightsBalanced(bins))).toEqual([]);
    }
  });
});

describe("wavelengthAutoWeightsBalanced", () => {
  it("normalises each column to a total of one", () => {
    const weights = wavelengthAutoWeightsBalanced([bin("a", 500), bin("b", 600), bin("c", 700)]);
    const total = (axis: "r" | "g" | "b") => weights.reduce((acc, w) => acc + w[axis], 0);

    expect(total("r")).toBeCloseTo(1, 5);
    expect(total("g")).toBeCloseTo(1, 5);
    expect(total("b")).toBeCloseTo(1, 5);
  });
});

describe("resolvePresetWeights", () => {
  it("falls through to the wavelength remap when only one preset channel is filled", () => {
    const bins = [bin("ha", 656), bin("wl900", 900), bin("wl4440", 4440)];

    const resolved = resolvePresetWeights(BLEND_PRESETS.sho, bins);

    expect(resolved).not.toBeNull();
    expect(resolved).toHaveLength(3);
    expect(emptyBlendColumns(resolved!)).toEqual([]);
  });

  it("uses the exact rows when they already cover R, G and B", () => {
    const bins = [bin("sii", 673), bin("ha", 656), bin("oiii", 501)];

    expect(resolvePresetWeights(BLEND_PRESETS.sho, bins)).toEqual(BLEND_PRESETS.sho.weights);
  });

  it("refuses a preset that cannot fill all three columns instead of returning a partial matrix", () => {
    const bins = [bin("sii", 673), bin("ha", 656)];

    expect(resolvePresetWeights(BLEND_PRESETS.sho, bins)).toBeNull();
  });
});

describe("weight matrix validation", () => {
  it("reports the wizard default as not covering the filled bins", () => {
    const bins = [bin("ha", 656), bin("wl900", 900), bin("wl4440", 4440)];

    const applicable = weightsForFilledBins(BLEND_PRESETS.sho.weights, bins);

    expect(applicable).toEqual([{ channelId: "ha", r: 0, g: 1, b: 0 }]);
    expect(blendWeightsCoverAllColumns(applicable)).toBe(false);
  });

  it("names the empty columns and the channels that could feed them", () => {
    const message = blendMatrixError([{ r: 0, g: 1, b: 0 }], ["Hα"]);

    expect(message).toContain("R and B");
    expect(message).toContain("Hα");
  });

  it("names a single empty column", () => {
    expect(blendMatrixError([{ r: 1, g: 0, b: 1 }], ["Hα", "OIII"])).toContain("G");
  });

  it("accepts a matrix that feeds every column", () => {
    expect(blendMatrixError([{ r: 1, g: 0.5, b: 0 }, { r: 0, g: 0.5, b: 1 }], ["a", "b"])).toBeNull();
  });

  it("rejects an empty matrix", () => {
    expect(blendMatrixError([], [])).toContain("No weights assigned");
  });
});
