import { describe, expect, it } from "vitest";
import {
  errPlaneLabel,
  errorBandExtent,
  errorBandKind,
  errorBandLegend,
  errorBandRuns,
  noiseFactorLabel,
  noiseFactorStaleNote,
  regionErrorLine,
  regionNotesShown,
  regionViewErrors,
} from "../regionErrors";
import type { ContinuumWindows, NoiseFactor, RegionSpectrum } from "../../shared/types/cube";

const FACTOR: NoiseFactor = {
  factor: 1.31,
  factor_se: 0.05,
  n_channels: 12,
  windows: [
    [0, 5],
    [34, 39],
  ],
  warning: null,
};

function region(extra: Partial<RegionSpectrum> = {}): RegionSpectrum {
  return {
    sum: [10, 20, 30],
    mean: [1, 2, 3],
    npix: 10,
    n_bg: 0,
    bg_per_pixel: null,
    wavelengths: null,
    unit: "um",
    bg_subtracted: false,
    flux_jy: [1e-4, 2e-4, 3e-4],
    err: [0.4, 0.5, 0.6],
    err_scaled: [0.524, 0.655, 0.786],
    mean_err: [0.04, 0.0625, 0.06],
    used_weight: [10, 8, 10],
    err_source: "ERR HDU 2",
    bg_err_per_pixel: null,
    flux_jy_err: [4e-6, 5e-6, 6e-6],
    n_dq_excluded: null,
    noise_factor: FACTOR,
    notes: [],
    elapsed_ms: 1,
    ...extra,
  };
}

describe("regionViewErrors", () => {
  it("gives the sum view the propagated aperture-sum error and the backend's scaled error", () => {
    const r = region();
    expect(regionViewErrors(r, "sum")).toEqual({ err: r.err, scaled: r.err_scaled });
  });

  it("gives the mean view the backend's mean error, never the sum error or the sum error over npix", () => {
    const r = region();
    const errors = regionViewErrors(r, "mean");
    expect(errors.err).toBe(r.mean_err);
    expect(errors.err).not.toEqual(r.err);
    expect(errors.err).not.toEqual(r.err!.map((e) => e / r.npix));
    expect(errors.scaled).toEqual(r.mean_err!.map((e) => e * FACTOR.factor));
  });

  it("gives the Jy view the error of the Jy flux, scaled by the same factor", () => {
    const r = region();
    const errors = regionViewErrors(r, "jy");
    expect(errors.err).toBe(r.flux_jy_err);
    expect(errors.scaled).toEqual(r.flux_jy_err!.map((e) => e * FACTOR.factor));
  });

  it("gives no scaled error in any view when the backend sent no scaled error", () => {
    const r = region({ err_scaled: null, noise_factor: null });
    for (const view of ["sum", "mean", "jy"] as const) expect(regionViewErrors(r, view).scaled).toBeNull();
    expect(regionViewErrors(r, "mean").err).toBe(r.mean_err);
  });

  it("gives no error at all for a cube without ERR", () => {
    const r = region({ err: null, err_scaled: null, mean_err: null, flux_jy_err: null, err_source: "none", noise_factor: null });
    for (const view of ["sum", "mean", "jy"] as const) expect(regionViewErrors(r, view)).toEqual({ err: null, scaled: null });
  });
});

describe("noiseFactorLabel", () => {
  it("prefixes the ERR source to the factor, its standard error and the line-free channel count", () => {
    expect(noiseFactorLabel("ERR HDU 2", FACTOR)).toBe("ERR HDU 2 x 1.31 +- 0.05 (12 line-free channels)");
  });

  it("rounds the factor and its standard error to two decimals as the backend warning does", () => {
    expect(noiseFactorLabel("ERR HDU 4", { ...FACTOR, factor: 0.98765, factor_se: 0.0123, n_channels: 240 })).toBe(
      "ERR HDU 4 x 0.99 +- 0.01 (240 line-free channels)",
    );
  });
});

describe("noiseFactorStaleNote", () => {
  it("says nothing while the panel's continuum windows are the ones the factor was measured on", () => {
    expect(
      noiseFactorStaleNote(FACTOR, [
        [0, 5],
        [34, 39],
      ]),
    ).toBeNull();
  });

  it("names the windows of the factor once the panel's windows changed", () => {
    expect(
      noiseFactorStaleNote(FACTOR, [
        [0, 6],
        [34, 39],
      ]),
    ).toBe("noise factor measured on continuum 0–5 and 34–39; the windows changed since: press Region spectrum to measure it again");
  });

  it("asks for windows first when none would be sent, because Region spectrum without windows drops the factor", () => {
    expect(noiseFactorStaleNote(FACTOR, null)).toBe(
      "noise factor measured on continuum 0–5 and 34–39; no valid continuum windows are set: set them, then press Region spectrum to measure it again (without windows it drops the factor)",
    );
  });

  it("names a single window once", () => {
    const single: NoiseFactor = {
      ...FACTOR,
      windows: [
        [0, 5],
        [0, 5],
      ],
    };
    expect(
      noiseFactorStaleNote(single, [
        [1, 5],
        [1, 5],
      ]),
    ).toBe("noise factor measured on continuum 0–5; the windows changed since: press Region spectrum to measure it again");
  });
});

describe("regionErrorLine", () => {
  const SENT: ContinuumWindows = [
    [0, 2],
    [0, 2],
  ];

  it("prints the measured factor", () => {
    expect(regionErrorLine("ERR HDU 2", FACTOR, FACTOR.windows)).toEqual({
      text: "ERR HDU 2 x 1.31 +- 0.05 (12 line-free channels)",
      factor: "measured",
    });
  });

  it("says the factor was not measured on the windows sent when the backend refused them", () => {
    expect(regionErrorLine("ERR HDU 2", null, SENT)).toEqual({
      text: "error band: ERR HDU 2; noise factor not measured on continuum 0–2 (see notes)",
      factor: "not_measured",
    });
  });

  it("names only the error source when no windows were sent", () => {
    expect(regionErrorLine("ERR HDU 2", null, null)).toEqual({ text: "error band: ERR HDU 2", factor: "none" });
  });
});

describe("errorBandKind and errorBandLegend", () => {
  it("names the bands drawn and explains the fill and the dashed outline only when both are drawn", () => {
    const r = region();
    expect(errorBandKind(regionViewErrors(r, "sum"))).toBe("propagated+scaled");
    expect(errorBandKind(regionViewErrors(region({ err_scaled: null, noise_factor: null }), "sum"))).toBe("propagated");
    expect(errorBandKind({ err: null, scaled: null })).toBe("none");
    expect(errorBandKind(null)).toBe("none");
    expect(errorBandLegend("propagated+scaled")).toBe("fill: propagated error; dashed: error x noise factor");
    expect(errorBandLegend("propagated")).toBeNull();
    expect(errorBandLegend("none")).toBeNull();
  });
});

describe("errorBandExtent", () => {
  it("spans the widest band edge drawn, the scaled one included", () => {
    expect(errorBandExtent(null, [0.1, 0.2], { err: [0.5, 0.5], scaled: [0.65, 0.65] })).toEqual([0.1 - 0.65, 0.2 + 0.65]);
    expect(errorBandExtent(null, [0.1, 0.2], { err: [0.5, 0.5], scaled: null })).toEqual([0.1 - 0.5, 0.2 + 0.5]);
  });

  it("skips the channels the band skips and gives nothing without a band", () => {
    expect(errorBandExtent([0, NaN, 2], [1, 100, 1], { err: [0.1, 50, 0.2], scaled: null })).toEqual([0.8, 1.2]);
    expect(errorBandExtent(null, [1, 2], { err: null, scaled: null })).toBeNull();
    expect(errorBandExtent(null, [1, 2], null)).toBeNull();
    expect(errorBandExtent(null, [NaN], { err: [0.1], scaled: null })).toBeNull();
  });
});

describe("regionNotesShown", () => {
  it("leaves the noise-factor warning out of the notes because the panel shows it on its own", () => {
    const warning = "the empirical noise is 2.00 +- 0.10 x the propagated error over 40 line-free channels, outside [0.8, 1.25]";
    const notes = ["error propagated from the ERR cube (HDU 2)", warning];
    expect(regionNotesShown(region({ notes, noise_factor: { ...FACTOR, warning } }))).toEqual(["error propagated from the ERR cube (HDU 2)"]);
    expect(regionNotesShown(region({ notes }))).toEqual(notes);
  });
});

describe("errPlaneLabel", () => {
  it("says an ERR plane was written, from which ERR cube, and the map-level noise factor when there is one", () => {
    expect(errPlaneLabel(true, "ERR HDU 2", null)).toBe("ERR written (ERR HDU 2)");
    expect(errPlaneLabel(true, "ERR HDU 2", 1.3127)).toBe("ERR written (ERR HDU 2), noise factor 1.31 in ABNOISEF");
  });

  it("says no ERR plane was written and names the error source", () => {
    expect(errPlaneLabel(false, "none", null)).toBe("no ERR written (error source: none)");
  });
});

describe("errorBandRuns", () => {
  it("brackets each finite channel by its error on the plotted axis", () => {
    expect(errorBandRuns([2, 2.1, 2.2], [10, 20, 30], [1, 2, 3])).toEqual([
      { x: [2, 2.1, 2.2], lower: [9, 18, 27], upper: [11, 22, 33] },
    ]);
  });

  it("uses the channel index without an axis", () => {
    expect(errorBandRuns(null, [10, 20], [1, 2])).toEqual([{ x: [0, 1], lower: [9, 18], upper: [11, 22] }]);
  });

  it("splits the band where the flux, the error or the axis is not finite, or the error is negative", () => {
    const runs = errorBandRuns([0, 1, 2, NaN, 4, 5, 6, 7], [1, 1, NaN, 1, 1, 1, 1, 1], [0.1, 0.1, 0.1, 0.1, 0.1, NaN, -0.1, 0.1]);
    expect(runs.map((run) => run.x)).toEqual([[0, 1], [4], [7]]);
  });

  it("draws no band without an error or when the error length differs from the spectrum", () => {
    expect(errorBandRuns(null, [1, 2], null)).toEqual([]);
    expect(errorBandRuns(null, [1, 2], [0.1])).toEqual([]);
  });
});
