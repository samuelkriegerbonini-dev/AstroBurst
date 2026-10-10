import { describe, it, expect } from "vitest";

import {
  blendMatrixError,
  blendWeightsCoverAllColumns,
  emptyBlendColumns,
  resolvePresetWeights,
  unequalColumnTotalsNote,
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

  it("keeps the luminance bin out of the R and G weights of an LRGB set", () => {
    const weights = wavelengthAutoWeights([bin("l"), bin("r"), bin("g"), bin("b")]);

    expect(weights.find((w) => w.channelId === "l")).toBeUndefined();
    expect(weights).toEqual([
      { channelId: "b", r: 0, g: 0, b: 1 },
      { channelId: "g", r: 0, g: 1, b: 0 },
      { channelId: "r", r: 1, g: 0, b: 0 },
    ]);
  });

  it("keeps the luminance bin out of the balanced spread as well", () => {
    const weights = wavelengthAutoWeightsBalanced([bin("l"), bin("ha", 656), bin("oiii", 501)]);

    expect(weights.map((w) => w.channelId)).toEqual(["oiii", "ha"]);
    expect(emptyBlendColumns(weights)).toEqual([]);
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

describe("unequalColumnTotalsNote", () => {
  const bicolorNote =
    "R, G and B get unequal total weights (R 1.00 · G 0.50 · B 0.50), so their backgrounds differ even after Match levels. Balanced (λ) gives each colour the same total weight.";

  it("names the R 1 / G 0.5 / B 0.5 totals of the two-filter Auto (λ) spread", () => {
    const weights = wavelengthAutoWeights([bin("ha", 656), bin("oiii", 501)]);

    expect(unequalColumnTotalsNote(weights)).toBe(bicolorNote);
  });

  it("stays silent for Balanced (λ) on the same two filters", () => {
    expect(unequalColumnTotalsNote(wavelengthAutoWeightsBalanced([bin("ha", 656), bin("oiii", 501)]))).toBeNull();
  });

  it("stays silent for the three-filter Auto (λ) spread, whose totals are 1 / 1 / 1", () => {
    expect(unequalColumnTotalsNote(wavelengthAutoWeights([bin("sii", 673), bin("ha", 656), bin("oiii", 501)]))).toBeNull();
  });

  it("stays silent for the four-filter Auto (λ) spread, whose rounded totals 1.33 / 1.34 / 1.33 sit within 5 %", () => {
    const bins = [bin("a", 450), bin("b", 550), bin("c", 650), bin("d", 750)];

    expect(unequalColumnTotalsNote(wavelengthAutoWeights(bins))).toBeNull();
  });

  it("flags the green-heavy five-filter Auto (λ) spread and clears it with Balanced (λ)", () => {
    const bins = [bin("a", 450), bin("b", 500), bin("c", 550), bin("d", 600), bin("e", 650)];

    expect(unequalColumnTotalsNote(wavelengthAutoWeights(bins))).toContain("(R 1.50 · G 2.00 · B 1.50)");
    expect(unequalColumnTotalsNote(wavelengthAutoWeightsBalanced(bins))).toBeNull();
  });

  it("stays silent for a diagonal identity matrix", () => {
    const identity = [{ r: 1, g: 0, b: 0 }, { r: 0, g: 1, b: 0 }, { r: 0, g: 0, b: 1 }];

    expect(unequalColumnTotalsNote(identity)).toBeNull();
  });

  it("ignores an all-zero column, which blendMatrixError reports instead", () => {
    expect(unequalColumnTotalsNote([{ r: 1, g: 1, b: 0 }])).toBeNull();
    expect(unequalColumnTotalsNote([{ r: 0.6, g: 0, b: 0 }, { r: 0, g: 0, b: 0.6 }])).toBeNull();
  });

  it("treats totals within 5 % of the largest as equal and anything wider as unequal", () => {
    expect(unequalColumnTotalsNote([{ r: 1, g: 0.95, b: 1 }])).toBeNull();
    expect(unequalColumnTotalsNote([{ r: 1, g: 0.94, b: 1 }])).toContain("(R 1.00 · G 0.94 · B 1.00)");
  });

  it("stays silent when there is nothing to compare", () => {
    expect(unequalColumnTotalsNote([])).toBeNull();
    expect(unequalColumnTotalsNote([{ r: 0, g: 0, b: 0 }])).toBeNull();
    expect(unequalColumnTotalsNote([{ r: 1, g: 0, b: 0 }])).toBeNull();
  });
});
