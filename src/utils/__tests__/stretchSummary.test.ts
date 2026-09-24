import { describe, it, expect } from "vitest";
import { backgroundRunSummary, stretchRunSummary, toneRunSummary } from "../wizard";

describe("stretchRunSummary", () => {
  it("reports per-channel iterations and convergence for a composite masked stretch", () => {
    const text = stretchRunSummary({
      elapsed_ms: 812,
      channels: {
        r: { iterations_run: 6, converged: true },
        g: { iterations_run: 10, converged: false },
        b: { iterations_run: 7, converged: true },
      },
    });
    expect(text).toBe("812ms, iterations R 6 / G 10 / B 7, not converged (G)");
  });

  it("says converged when every channel converged", () => {
    const text = stretchRunSummary({
      elapsed_ms: 5,
      channels: { r: { iterations_run: 3, converged: true }, g: { iterations_run: 3, converged: true }, b: { iterations_run: 4, converged: true } },
    });
    expect(text).toContain("converged");
    expect(text).not.toContain("not converged");
  });

  it("keeps the single-channel top-level fields", () => {
    expect(stretchRunSummary({ elapsed_ms: 40, iterations_run: 9, converged: false })).toBe("40ms, 9 iterations, not converged");
    expect(stretchRunSummary({ elapsed_ms: 12, stretch_factor: 5 })).toBe("12ms, factor=5");
  });
});

describe("backgroundRunSummary", () => {
  it("leaves out the sample count and residual that a batch mode did not measure", () => {
    expect(backgroundRunSummary({ sample_count: null, rms_residual: null, elapsed_ms: 12 })).toBe("12ms");
    expect(backgroundRunSummary({ sample_count: 64, rms_residual: null, elapsed_ms: 5 })).toBe("64 samples, 5ms");
    expect(backgroundRunSummary({ sample_count: 64, rms_residual: 0.01234, elapsed_ms: 5 })).toBe("64 samples, RMS 0.0123, 5ms");
  });
});

describe("toneRunSummary", () => {
  it("says which local-contrast runs the backend re-applied on top of the new curves", () => {
    expect(toneRunSummary({ elapsed_ms: 30, curves_applied: true, contrast_reapplied: ["lhe", "hdr"] }))
      .toBe("30ms | curves applied | LHE + HDRMT re-applied on the new curves");
  });

  it("stays quiet when nothing was re-applied", () => {
    expect(toneRunSummary({ elapsed_ms: 30, curves_applied: true, contrast_reapplied: [] })).toBe("30ms | curves applied");
    expect(toneRunSummary({ elapsed_ms: 8, curves_applied: false })).toBe("8ms");
  });
});
