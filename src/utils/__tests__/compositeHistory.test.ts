import { describe, expect, it } from "vitest";
import { applyCompositeOp, compositeHistoryLines, EMPTY_COMPOSITE_HISTORY, exportFitsContents, type CompositeOp } from "../wizard";

const DECONV = ["Deconvolution RL: 20 iterations, PSF gaussian sigma 2 px size 15"];
const BACKGROUND = ["Background polynomial: grid 8, degree 3, sigma 2.5, 3 iterations, subtract"];

function apply(...ops: CompositeOp[]) {
  return ops.reduce(applyCompositeOp, EMPTY_COMPOSITE_HISTORY);
}

describe("Blend weights in the composite HISTORY", () => {
  it("writes one line per channel with every weight of the matrix that produced the colours", () => {
    const lines = compositeHistoryLines(apply({
      kind: "blend",
      preset: "custom",
      weights: [
        { channel: "ha F656N", r: 0.3, g: 0.8, b: 0.2 },
        { channel: "oiii F502N", r: 0, g: 0.15, b: 0.85 },
      ],
    }));
    expect(lines).toEqual([
      "Blend: custom",
      "Blend ha F656N: R=0.300 G=0.800 B=0.200",
      "Blend oiii F502N: R=0.000 G=0.150 B=0.850",
    ]);
  });
});

describe("Processing tab steps on the linear composite in the HISTORY", () => {
  const blended = apply({ kind: "blend", preset: "sho" });

  it("records the chain steps with their parameters after the wizard lines", () => {
    const h = applyCompositeOp(blended, { kind: "chain", lines: [...BACKGROUND, ...DECONV], restarted: false });
    expect(compositeHistoryLines(h)).toEqual(["Blend: sho", ...BACKGROUND, ...DECONV]);
  });

  it("replaces the chain lines when a chain step runs again from an earlier input", () => {
    const h = apply({ kind: "blend", preset: "sho" }, { kind: "chain", lines: DECONV, restarted: false }, { kind: "chain", lines: BACKGROUND, restarted: false });
    expect(compositeHistoryLines(h)).toEqual(["Blend: sho", ...BACKGROUND]);
  });

  it("keeps the lines of a chain whose result became the new chain base", () => {
    const h = apply({ kind: "blend", preset: "sho" }, { kind: "chain", lines: DECONV, restarted: false }, { kind: "chain", lines: BACKGROUND, restarted: true });
    expect(compositeHistoryLines(h)).toEqual(["Blend: sho", ...DECONV, ...BACKGROUND]);
  });

  it("drops the chain lines on a chain reset that restored the composite, and keeps them when nothing was restored", () => {
    const h = applyCompositeOp(blended, { kind: "chain", lines: DECONV, restarted: false });
    expect(compositeHistoryLines(applyCompositeOp(h, { kind: "chainReset", restored: true }))).toEqual(["Blend: sho"]);
    expect(compositeHistoryLines(applyCompositeOp(h, { kind: "chainReset", restored: false }))).toEqual(["Blend: sho", ...DECONV]);
  });

  it("keeps the chain lines in order before a wizard step that ran on top of them", () => {
    const h = apply({ kind: "blend", preset: "sho" }, { kind: "chain", lines: DECONV, restarted: false }, { kind: "starRemoval", sigma: 4, growth: 3 });
    expect(compositeHistoryLines(h)).toEqual(["Blend: sho", ...DECONV, "Star removal: 4.0 sigma, growth 3.00x FWHM"]);
  });

  it("forgets the chain lines when a new Blend replaces the composite", () => {
    const h = apply({ kind: "blend", preset: "sho" }, { kind: "chain", lines: DECONV, restarted: false }, { kind: "blend", preset: "hoo" });
    expect(compositeHistoryLines(h)).toEqual(["Blend: hoo"]);
  });
});

describe("Export banner", () => {
  it("says what the FITS holds, from the same lines the HISTORY gets", () => {
    const h = apply(
      { kind: "blend", preset: "sho" },
      { kind: "colorBalance", mode: "manual", r: 1.2, g: 1, b: 0.9, scnr: null },
      { kind: "chain", lines: DECONV, restarted: false },
    );
    expect(exportFitsContents(h)).toBe(
      `FITS holds the linear composite, written to HISTORY as: Blend: sho; White balance: manual R=1.200 G=1.000 B=0.900; ${DECONV[0]}.`,
    );
  });
});
