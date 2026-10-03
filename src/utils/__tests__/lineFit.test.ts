import { describe, it, expect } from "vitest";
import {
  COMPONENT_COLORS,
  DEFAULT_LINE_FIT_SNR,
  INTEGER_PLANES,
  LINE_FIT_LABEL_PREFIX,
  VELOCITY_PLANES,
  displayHintFor,
  displayedLineFitPlane,
  inspectKey,
  isVelocityPlane,
  lineFitConfig,
  lineFitCubeResult,
  lineFitFrameNote,
  lineFitInspectPlot,
  lineFitInspectSummary,
  lineFitOverlayPolylines,
  lineFitResultLabel,
  lineFitSpaxelNotes,
  lineFitSpaxelRows,
  lineFitSummary,
  parseResolvingPower,
  planeLabel,
  planeUnit,
  sigmaCaption,
  spanAxisValue,
  type LineFitConfigInput,
  type LineFitRun,
} from "../lineFit";
import { channelAxisValue, axisValueToPixel, defaultContinuumWindows, type PlotMapping } from "../spectrumRange";
import type { LineFitComponentFit, LineFitConfig, LineFitPlane, LineFitResult, LineFitSpaxel } from "../../shared/types/cube";

function configInput(overrides: Partial<LineFitConfigInput> = {}): LineFitConfigInput {
  return {
    range: { z0: 20, z1: 30 },
    windows: null,
    channelCount: 60,
    restUm: 1.875613,
    convention: "optical",
    snrText: "3",
    emissionOnly: true,
    useErr: true,
    useDq: true,
    resolvingPowerText: "",
    components: "one",
    ...overrides,
  };
}

function config(overrides: Partial<LineFitConfig> = {}): LineFitConfig {
  return {
    z0: 530,
    z1: 560,
    rest_um: 1.875613,
    convention: "optical",
    continuum: [
      [500, 520],
      [568, 588],
    ],
    snr_threshold: 3,
    emission_only: true,
    use_err: true,
    use_dq: true,
    resolving_power: null,
    components: "one",
    ...overrides,
  };
}

function files(plane: string, withPreview = true) {
  return {
    png_path: `C:/out/a_linefit_${plane}_530-560.png`,
    fits_path: `C:/out/a_linefit_${plane}_530-560.fits`,
    ...(withPreview ? { previewUrl: `asset://a_linefit_${plane}.png` } : {}),
  };
}

function result(overrides: Partial<LineFitResult> = {}): LineFitResult {
  const order: LineFitPlane[] = ["flux", "velocity", "sigma_obs", "flux_err", "v_err", "sigma_err", "chi2_red", "snr", "mask", "ncomp"];
  return {
    planes: Object.fromEntries(order.map((p) => [p, files(p)])),
    plane_order: order,
    units: { flux: "MJy/sr km/s", velocity: "km/s", sigma: "km/s" },
    sigma_label: "observed (instrumental width not removed)",
    weighting: "err",
    err_hdu: 2,
    dq_hdu: 3,
    z0: 530,
    z1: 560,
    n_channels: 31,
    rest_um: 1.875613,
    convention: "optical",
    components: "one",
    continuum_windows: [
      [500, 520],
      [568, 588],
    ],
    resolving_power: null,
    snr_threshold: 3,
    n_fit: 1310,
    n_masked: 64,
    n_const_continuum: 2,
    n_two_components: 0,
    median_chi2_red: 1.9,
    notes: ["n1", "n2"],
    dimensions: [53, 55],
    elapsed_ms: 1800,
    ...overrides,
  };
}

function run(overrides: Partial<LineFitResult> = {}, lineLabel: string | null = "Paα"): LineFitRun {
  return { key: "k1", filePath: "C:/d/a_s3d.fits#hdu=1", config: config(), result: result(overrides), lineLabel };
}

function component(overrides: Partial<LineFitComponentFit> = {}): LineFitComponentFit {
  return {
    amplitude: 2.5,
    centre: 1.876,
    sigma: 0.0006,
    amplitude_err: 0.1,
    centre_err: 0.00001,
    sigma_err: 0.00002,
    velocity_kms: 61.2,
    v_err_kms: 1.6,
    sigma_kms: 96.0,
    sigma_err_kms: 3.2,
    sigma_corr_kms: null,
    flux: 601.5,
    flux_err: 25.0,
    snr: 24.06,
    ...overrides,
  };
}

function spaxel(overrides: Partial<LineFitSpaxel> = {}): LineFitSpaxel {
  return {
    x: 18,
    y: 32,
    z0: 1,
    z1: 1,
    continuum_windows: [
      [0, 0],
      [2, 2],
    ],
    span: [0, 2],
    axis: [1, 2, 3],
    axis_unit: "um",
    flux: [1, 3, 1],
    err: null,
    channels: [1],
    dropped_dq: [7, 8],
    dropped_err: [],
    continuum: { intercept: 1, slope: 0, x_ref: 2, sigma: 0.05, linear: true, channels: 2 },
    weighting: "err",
    single: component(),
    chi2: 30.5,
    dof: 28,
    chi2_red: 1.089,
    converged: true,
    iterations: 9,
    components: [],
    ncomp: 1,
    delta_bic: null,
    mask: 1,
    model: {
      channel: [0, 0.5, 1, 1.5, 2],
      continuum: [1, 1, 1, 1, 1],
      total: [1, 2, 3, 2, 1],
      components: [[0, 1, 2, 1, 0]],
    },
    notes: [],
    ...overrides,
  };
}

const MAPPING: PlotMapping = { width: 100, padLeft: 10, padRight: 10, xMin: 1, xMax: 3, xValues: [1, 2, 3], n: 3 };
const FRAME = { top: 0, height: 100, yMin: 0, yMax: 10 };

describe("lineFitConfig", () => {
  it("fills the continuum from defaultContinuumWindows when no windows are brushed and maps an empty R to null", () => {
    const cfg = lineFitConfig(configInput());
    expect(cfg).toEqual({
      z0: 20,
      z1: 30,
      rest_um: 1.875613,
      convention: "optical",
      continuum: defaultContinuumWindows({ z0: 20, z1: 30 }, 60),
      snr_threshold: 3,
      emission_only: true,
      use_err: true,
      use_dq: true,
      resolving_power: null,
      components: "one",
    });
    expect(cfg).toHaveProperty("continuum", [
      [15, 19],
      [31, 35],
    ]);
  });

  it("keeps brushed windows, the components mode, the flags and a valid R", () => {
    const cfg = lineFitConfig(
      configInput({
        windows: [
          [2, 8],
          [40, 50],
        ],
        resolvingPowerText: " 2700 ",
        components: "auto",
        useErr: false,
        useDq: false,
        emissionOnly: false,
        snrText: "0",
        restUm: null,
      }),
    );
    expect(cfg).toMatchObject({
      continuum: [
        [2, 8],
        [40, 50],
      ],
      resolving_power: 2700,
      components: "auto",
      use_err: false,
      use_dq: false,
      emission_only: false,
      snr_threshold: 0,
      rest_um: null,
    });
  });

  it("rejects a negative or non-numeric S/N, an R of abc or 0, and windows outside the cube", () => {
    expect(lineFitConfig(configInput({ snrText: "-1" }))).toHaveProperty("error");
    expect(lineFitConfig(configInput({ snrText: "x" }))).toHaveProperty("error");
    expect(lineFitConfig(configInput({ snrText: "" }))).toHaveProperty("error");
    expect(lineFitConfig(configInput({ resolvingPowerText: "abc" }))).toHaveProperty("error");
    expect(lineFitConfig(configInput({ resolvingPowerText: "0" }))).toHaveProperty("error");
    expect(
      lineFitConfig(
        configInput({
          windows: [
            [2, 8],
            [55, 70],
          ],
        }),
      ),
    ).toHaveProperty("error");
  });

  it("parseResolvingPower maps empty text to null, a positive number to itself and anything else to an error", () => {
    expect(parseResolvingPower("")).toBeNull();
    expect(parseResolvingPower("   ")).toBeNull();
    expect(parseResolvingPower("2700")).toBe(2700);
    expect(parseResolvingPower("abc")).toHaveProperty("error");
    expect(parseResolvingPower("0")).toHaveProperty("error");
    expect(parseResolvingPower("-5")).toHaveProperty("error");
    expect(parseResolvingPower("Infinity")).toHaveProperty("error");
  });
});

describe("plane labels, units and display hints", () => {
  it("labels every base plane and suffixes the component planes", () => {
    expect(planeLabel("flux")).toBe("flux");
    expect(planeLabel("velocity")).toBe("V");
    expect(planeLabel("sigma_obs")).toBe("σ obs");
    expect(planeLabel("sigma_corr")).toBe("σ corr");
    expect(planeLabel("flux_err")).toBe("flux err");
    expect(planeLabel("v_err")).toBe("V err");
    expect(planeLabel("sigma_err")).toBe("σ err");
    expect(planeLabel("chi2_red")).toBe("χ²_red");
    expect(planeLabel("snr")).toBe("S/N");
    expect(planeLabel("mask")).toBe("mask");
    expect(planeLabel("ncomp")).toBe("ncomp");
    expect(planeLabel("c1_velocity")).toBe("V c1");
    expect(planeLabel("c2_flux_err")).toBe("flux err c2");
    expect(planeLabel("c2_sigma_obs")).toBe("σ obs c2");
  });

  it("planeUnit follows the D22 table", () => {
    const r = result();
    expect(planeUnit(r, "flux")).toBe("MJy/sr km/s");
    expect(planeUnit(r, "c1_flux_err")).toBe("MJy/sr km/s");
    expect(planeUnit(r, "c2_v_err")).toBe("km/s");
    expect(planeUnit(r, "sigma_corr")).toBe("km/s");
    expect(planeUnit(r, "c1_sigma_err")).toBe("km/s");
    expect(planeUnit(r, "ncomp")).toBeNull();
    expect(planeUnit(r, "mask")).toBeNull();
    expect(planeUnit(r, "chi2_red")).toBeNull();
    expect(planeUnit(r, "snr")).toBeNull();
  });

  it("isVelocityPlane is true for the three velocity planes only", () => {
    expect(VELOCITY_PLANES).toEqual(["velocity", "c1_velocity", "c2_velocity"]);
    expect(INTEGER_PLANES).toEqual(["mask", "ncomp"]);
    expect(isVelocityPlane("velocity")).toBe(true);
    expect(isVelocityPlane("c2_velocity")).toBe(true);
    expect(isVelocityPlane("v_err")).toBe(false);
    expect(isVelocityPlane("flux")).toBe(false);
  });

  it("displayHintFor gives a diverging map for velocities, gray minmax for integer planes and viridis otherwise", () => {
    const velocity = { colormap: "rdbu", invert: true, symmetric: true, centre: 0, stretch: "linear", limits: "percentile" };
    expect(displayHintFor("velocity")).toEqual(velocity);
    expect(displayHintFor("c2_velocity")).toEqual(velocity);
    expect(displayHintFor("c1_velocity")).toEqual(velocity);
    expect(displayHintFor("flux")).toEqual({ colormap: "viridis", invert: false, symmetric: false, stretch: "linear", limits: "percentile" });
    expect(displayHintFor("v_err")).toEqual({ colormap: "viridis", invert: false, symmetric: false, stretch: "linear", limits: "percentile" });
    expect(displayHintFor("mask")).toEqual({ colormap: "gray", invert: false, symmetric: false, stretch: "linear", limits: "minmax" });
    expect(displayHintFor("ncomp")).toEqual({ colormap: "gray", invert: false, symmetric: false, stretch: "linear", limits: "minmax" });
  });

  it("displayHintFor inverts RdBu on velocity planes so approaching gas is blue and sets invert on every hint", () => {
    for (const plane of VELOCITY_PLANES) expect(displayHintFor(plane).invert).toBe(true);
    for (const plane of ["flux", "sigma_obs", "chi2_red", "snr", "mask", "ncomp", "c1_flux"] as const) {
      expect(displayHintFor(plane).invert).toBe(false);
    }
  });
});

describe("line-fit results in the viewer", () => {
  it("lineFitResultLabel names the plane, the picked line or the channel range, and the fitted count", () => {
    expect(lineFitResultLabel(result(), "velocity", "Brα")).toBe("Line fit V · Brα · 1310 px");
    expect(lineFitResultLabel(result(), "flux", null)).toBe("Line fit flux · ch 530-560 · 1310 px");
    expect(lineFitResultLabel(result(), "velocity", null).startsWith(LINE_FIT_LABEL_PREFIX)).toBe(true);
  });

  it("lineFitCubeResult returns null without a previewUrl and otherwise carries the display hint", () => {
    const withPreview = lineFitCubeResult(run(), "velocity");
    expect(withPreview).toEqual({
      label: "Line fit V · Paα · 1310 px",
      previewUrl: "asset://a_linefit_velocity.png",
      fitsPath: "C:/out/a_linefit_velocity_530-560.fits",
      dimensions: [53, 55],
      displayHint: displayHintFor("velocity"),
    });
    const bare = run();
    bare.result.planes.flux = files("flux", false);
    expect(lineFitCubeResult(bare, "flux")).toBeNull();
    expect(lineFitCubeResult(run(), "c1_velocity")).toBeNull();
  });

  it("displayedLineFitPlane returns the plane whose fits_path is displayed and null otherwise", () => {
    const r = result();
    expect(displayedLineFitPlane({ fitsPath: "C:/out/a_linefit_flux_530-560.fits" }, r)).toBe("flux");
    expect(displayedLineFitPlane({ fitsPath: "C:/out/a_linefit_ncomp_530-560.fits" }, r)).toBe("ncomp");
    expect(displayedLineFitPlane({ fitsPath: "C:/out/a_moment_m1.fits" }, r)).toBeNull();
    expect(displayedLineFitPlane({ fitsPath: null }, r)).toBeNull();
    expect(displayedLineFitPlane(null, r)).toBeNull();
  });

  it("lineFitSummary lists the counts and adds the two-component count only when components is not one", () => {
    const one = lineFitSummary(result());
    expect(one).toBe("1310 fitted, 64 below S/N 3, 2 constant continuum, median χ²_red 1.900, 1800 ms");
    expect(one).not.toContain("two-component");
    const auto = lineFitSummary(result({ components: "auto", n_two_components: 351 }));
    expect(auto).toContain(", 351 two-component");
    expect(lineFitSummary(result({ components: "two", n_two_components: 0 }))).toContain("0 two-component");
    expect(lineFitSummary(result({ median_chi2_red: null }))).toContain("median χ²_red n/a");
  });

  it("sigmaCaption and lineFitFrameNote say what the sigma and the velocity frame are", () => {
    expect(sigmaCaption(result())).toBe("σ observed (instrumental width not removed)");
    expect(sigmaCaption(result({ sigma_label: "LSF-corrected (R = 2700)" }))).toBe("σ LSF-corrected (R = 2700)");
    expect(lineFitFrameNote("BARYCENT")).toBe("line-fit V in the header frame BARYCENT");
    expect(lineFitFrameNote(null)).toBe("line-fit V in the header frame (no SPECSYS)");
    expect(lineFitFrameNote(null)).toContain("(no SPECSYS)");
  });

  it("inspectKey changes with the file, the pixel and the run", () => {
    const a = run();
    const b = { ...run(), key: "k2" };
    const base = inspectKey("C:/d/a.fits", 1, 2, a);
    expect(inspectKey("C:/d/a.fits", 1, 2, a)).toBe(base);
    expect(inspectKey("C:/d/b.fits", 1, 2, a)).not.toBe(base);
    expect(inspectKey("C:/d/a.fits", 2, 1, a)).not.toBe(base);
    expect(inspectKey("C:/d/a.fits", 1, 2, b)).not.toBe(base);
  });
});

describe("lineFitOverlayPolylines", () => {
  const px = (x: number) => axisValueToPixel(x, MAPPING);
  const py = (y: number) => 100 - (y / 10) * 100;

  it("maps fractional channels to display pixels by linear interpolation and keeps the last channel", () => {
    const curves = lineFitOverlayPolylines(spaxel(), MAPPING, FRAME);
    expect(curves.model.map((p) => p.x)).toEqual([px(1), px(1.5), px(2), px(2.5), px(3)]);
    expect(curves.model[1].x).toBeCloseTo((px(1) + px(2)) / 2, 12);
    expect(curves.model.map((p) => p.y)).toEqual([py(1), py(2), py(3), py(2), py(1)]);
    expect(curves.continuum).toHaveLength(5);
    expect(curves.continuum.every((p) => p.y === py(1))).toBe(true);
    expect(curves.components).toHaveLength(1);
    expect(curves.components[0].map((p) => p.y)).toEqual([py(1), py(2), py(3), py(2), py(1)]);
    expect(curves.model[4].x).toBe(axisValueToPixel(channelAxisValue(2, MAPPING), MAPPING));
    expect(curves.model[4].x).toBe(MAPPING.width - MAPPING.padRight);
  });

  it("drops points left of the plot pad, non-finite values and returns nothing without a model", () => {
    const shifted: PlotMapping = { ...MAPPING, xMin: 1.5 };
    const curves = lineFitOverlayPolylines(spaxel(), shifted, FRAME);
    expect(curves.model).toHaveLength(4);
    expect(curves.model[0].x).toBe(shifted.padLeft);
    expect(curves.model.every((p) => p.x >= shifted.padLeft && p.x <= shifted.width - shifted.padRight)).toBe(true);
    const withNull = spaxel({
      model: { channel: [0, 1, 2], continuum: [1, 1, 1], total: [1, null as unknown as number, 1], components: [] },
    });
    expect(lineFitOverlayPolylines(withNull, MAPPING, FRAME).model).toHaveLength(2);
    expect(lineFitOverlayPolylines(spaxel({ model: null }), MAPPING, FRAME)).toEqual({ continuum: [], model: [], components: [] });
  });

  it("returns one curve per component of a two-component spaxel", () => {
    const two = spaxel({
      ncomp: 2,
      model: {
        channel: [0, 1, 2],
        continuum: [1, 1, 1],
        total: [2, 3, 2],
        components: [
          [1, 1, 0],
          [0, 1, 1],
        ],
      },
    });
    const curves = lineFitOverlayPolylines(two, MAPPING, FRAME);
    expect(curves.components).toHaveLength(2);
    expect(curves.components[1].map((p) => p.y)).toEqual([py(1), py(2), py(2)]);
  });

  it("draws each component on top of the continuum and drops points where either array is null", () => {
    const sloped = spaxel({
      model: {
        channel: [0, 1, 2],
        continuum: [4, null as unknown as number, 6],
        total: [4.5, 6, 6.5],
        components: [[0.5, 1, null as unknown as number]],
      },
    });
    const curves = lineFitOverlayPolylines(sloped, MAPPING, FRAME);
    expect(curves.components[0]).toEqual([{ x: px(1), y: py(4.5) }]);
  });
});

describe("lineFitSpaxelRows", () => {
  const units = { flux: "MJy/sr km/s", velocity: "km/s", sigma: "km/s" };
  const labels = (s: LineFitSpaxel) => lineFitSpaxelRows(s, units).map((r) => r.label);
  const value = (s: LineFitSpaxel, label: string) => lineFitSpaxelRows(s, units).find((r) => r.label === label)?.value;

  it("lists V, σ obs, flux, amplitude, χ²_red, S/N, ncomp, ΔBIC and the dropped channels of a single fit", () => {
    const s = spaxel();
    expect(labels(s)).toEqual(["V", "σ obs", "Flux", "Amplitude", "χ²_red", "S/N", "ncomp", "ΔBIC", "Dropped", "Continuum", "Fit"]);
    expect(value(s, "V")).toBe("61.2 ± 1.6 km/s");
    expect(value(s, "σ obs")).toBe("96.0 ± 3.2 km/s");
    expect(value(s, "Flux")).toBe("601.5 ± 25.00 MJy/sr km/s");
    expect(value(s, "S/N")).toBe("24.1");
    expect(value(s, "ncomp")).toBe("1");
    expect(value(s, "ΔBIC")).toBe("n/a");
    expect(value(s, "Dropped")).toBe("DQ 2 ch, ERR 0 ch");
    expect(value(s, "Fit")).toContain("converged");
  });

  it("switches to σ corr when the spaxel carries an LSF-corrected sigma", () => {
    const s = spaxel({ single: component({ sigma_corr_kms: 80.25 }) });
    expect(labels(s)).toContain("σ corr");
    expect(labels(s)).not.toContain("σ obs");
    expect(value(s, "σ corr")).toContain("80.3 km/s");
    expect(value(s, "σ corr")).toContain("96.0 ± 3.2");
  });

  it("marks σ corr as unresolved when R is given and the spaxel has no corrected sigma", () => {
    const s = spaxel({ mask: 17 });
    expect(labels(s)).toContain("σ obs");
    const rows = lineFitSpaxelRows(s, units, 2700);
    expect(rows.map((r) => r.label)).toContain("σ corr");
    expect(rows.map((r) => r.label)).not.toContain("σ obs");
    expect(rows.find((r) => r.label === "σ corr")?.value).toBe("unresolved (σ ≤ σ_inst), obs 96.0 ± 3.2 km/s");
    const corrected = lineFitSpaxelRows(spaxel({ single: component({ sigma_corr_kms: 80.25 }) }), units, 2700);
    expect(corrected.find((r) => r.label === "σ corr")?.value).toContain("80.3 km/s");
    const two = spaxel({ ncomp: 2, components: [component(), component({ velocity_kms: 90 })] });
    const componentRows = lineFitSpaxelRows(two, units, 2700).filter((r) => r.label.startsWith("c"));
    expect(componentRows).toHaveLength(2);
    expect(componentRows.every((r) => r.value.includes("σ corr unresolved (σ ≤ σ_inst)"))).toBe(true);
    expect(lineFitSpaxelRows(two, units).filter((r) => r.label.startsWith("c")).some((r) => r.value.includes("unresolved"))).toBe(false);
  });

  it("adds one row per component when ncomp is 2", () => {
    const s = spaxel({
      ncomp: 2,
      delta_bic: 512.5,
      mask: 129,
      components: [component({ velocity_kms: -220.4 }), component({ velocity_kms: 35.5, v_err_kms: 2.25 })],
    });
    const rows = lineFitSpaxelRows(s, units);
    const componentRows = rows.filter((r) => r.label.startsWith("c1") || r.label.startsWith("c2"));
    expect(componentRows).toHaveLength(2);
    expect(componentRows[0].label.startsWith("c1")).toBe(true);
    expect(componentRows[0].value).toContain("V -220.4 ± 1.6 km/s");
    expect(componentRows[1].value).toContain("V 35.5 ± 2.3 km/s");
    expect(value(s, "ncomp")).toBe("2");
    expect(value(s, "ΔBIC")).toBe("512.5 raw, 470.6 / max(1, χ²_red)");
    expect(lineFitSpaxelRows(spaxel(), units).some((r) => r.label.startsWith("c1"))).toBe(false);
  });

  it("shows n/a values and a no-fit status when the spaxel was not fitted", () => {
    const s = spaxel({ single: null, converged: false, mask: 0, ncomp: 0, chi2: null, chi2_red: null, model: null });
    expect(value(s, "V")).toBe("n/a");
    expect(value(s, "Fit")).toBe("no fit attempted (fewer than 4 usable line samples or no usable continuum sample)");
    const noContinuum = spaxel({ single: null, converged: false, mask: 0, ncomp: 0, continuum: null, model: null, dropped_err: [0, 2] });
    expect(value(noContinuum, "Fit")).toContain("no fit attempted");
    const failed = spaxel({ single: null, converged: false, mask: 32, ncomp: 1 });
    expect(value(failed, "Fit")).toContain("did not converge");
  });

  it("reports a converged fit below the S/N threshold (mask 0, ncomp 1) as converged, not as no fit", () => {
    const s = spaxel({ mask: 0, ncomp: 1, converged: true, single: component({ snr: 2.1 }) });
    expect(value(s, "V")).toBe("61.2 ± 1.6 km/s");
    expect(value(s, "Fit")).toContain("converged, below S/N threshold");
    expect(value(s, "Fit")).not.toContain("no fit");
    const constantContinuum = spaxel({ mask: 2, ncomp: 1, converged: true, single: component({ snr: 2.1 }) });
    expect(value(constantContinuum, "Fit")).toContain("converged, below S/N threshold");
    expect(value(spaxel(), "Fit")).not.toContain("below S/N threshold");
    const twoModeBelow = spaxel({ mask: 64, ncomp: 1, converged: true, single: component({ snr: 2.1 }) });
    expect(value(twoModeBelow, "Fit")).toContain("converged, below S/N threshold");
  });

  it("shows the raw ΔBIC next to the value the auto rule compares, ΔBIC / max(1, χ²_red)", () => {
    const s = spaxel({ delta_bic: 512.5, chi2_red: 1.089, mask: 65 });
    expect(value(s, "ΔBIC")).toBe("512.5 raw, 470.6 / max(1, χ²_red)");
    const lowChi = spaxel({ delta_bic: 40, chi2_red: 0.5, mask: 65 });
    expect(value(lowChi, "ΔBIC")).toBe("40.00 raw, 40.00 / max(1, χ²_red)");
    const noChi = spaxel({ delta_bic: 40, chi2_red: null, mask: 65 });
    expect(value(noChi, "ΔBIC")).toBe("40.00 raw");
  });

  it("lineFitSpaxelNotes keeps the per-spaxel reasons and drops the notes the run already shows", () => {
    const runNotes = ["weights: 1/ERR^2 from HDU 2", "single Gaussian: inspect chi2_red"];
    const s = spaxel({
      mask: 65,
      notes: [
        "weights: 1/ERR^2 from HDU 2",
        "single Gaussian: inspect chi2_red",
        "two components rejected: component 1 amplitude below 3 sigma",
      ],
    });
    expect(lineFitSpaxelNotes(s, runNotes)).toEqual(["two components rejected: component 1 amplitude below 3 sigma"]);
    expect(lineFitSpaxelNotes(s, [])).toEqual(s.notes);
    expect(lineFitSpaxelNotes(spaxel({ notes: [] }), runNotes)).toEqual([]);
  });

  it("the default S/N threshold is 3", () => {
    expect(DEFAULT_LINE_FIT_SNR).toBe(3);
  });
});

describe("lineFitInspectPlot", () => {
  const AXIS = [4.0, 4.001, 4.002, 4.003, 4.004, 4.005, 4.0065, 4.0075, 4.0085, 4.0095, 4.0105, 4.0115, 4.0125];
  const MODEL_CHANNELS = Array.from({ length: 49 }, (_, k) => 100 + k / 4);
  const CONTINUUM = MODEL_CHANNELS.map(() => 1);
  const C1 = MODEL_CHANNELS.map((_, k) => (k === 22 ? 3 : 0.5));
  const C2 = MODEL_CHANNELS.map(() => 0.25);

  function inspectSpaxel(overrides: Partial<LineFitSpaxel> = {}): LineFitSpaxel {
    return spaxel({
      z0: 104,
      z1: 108,
      continuum_windows: [
        [100, 102],
        [110, 112],
      ],
      span: [100, 112],
      axis: AXIS,
      axis_unit: "um",
      flux: [1.0, 1.2, 0.9, 1.05, 2.0, 4.0, 1e6, 3.5, 1.8, 1.0, 1.1, null, 0.95],
      err: [0.1, null, 0.1, 0.1, 0.2, 0.2, 0.2, 0.2, 0.2, 0.1, 0.1, 0.1, 0.1],
      channels: [104, 105, 107, 108],
      dropped_dq: [106],
      dropped_err: [101],
      ncomp: 2,
      single: component({ centre: 4.0055, velocity_kms: 61.2 }),
      components: [component({ centre: 4.0052, velocity_kms: -220.34 }), component({ centre: 4.0081, velocity_kms: 15.2 })],
      model: {
        channel: MODEL_CHANNELS,
        continuum: CONTINUUM,
        total: CONTINUUM.map((c, k) => c + C1[k] + C2[k]),
        components: [C1, C2],
      },
      ...overrides,
    });
  }

  function singleSpaxel(): LineFitSpaxel {
    return inspectSpaxel({
      ncomp: 1,
      components: [],
      model: { channel: MODEL_CHANNELS, continuum: CONTINUUM, total: CONTINUUM.map((c, k) => c + C1[k]), components: [C1] },
    });
  }

  it("spanAxisValue maps a fractional channel of the span to the axis exactly as D19: axis[i] + (axis[i+1] - axis[i])·t, clamped at both ends", () => {
    const s = inspectSpaxel();
    expect(spanAxisValue(s, 100)).toBe(4.0);
    expect(spanAxisValue(s, 106)).toBe(4.0065);
    expect(spanAxisValue(s, 105.5)).toBeCloseTo(4.00575, 12);
    expect(spanAxisValue(s, 105.25)).toBeCloseTo(4.005 + 0.0015 * 0.25, 12);
    expect(spanAxisValue(s, 112)).toBe(4.0125);
    expect(spanAxisValue(s, 112.5)).toBe(4.0125);
    expect(spanAxisValue(s, 99.5)).toBe(4.0);
    expect(spanAxisValue({ axis: [], span: [0, 0] }, 0)).toBeNaN();
  });

  it("plots every finite sample of the span with its ERR bar and marks used, dropped and out-of-window channels", () => {
    const plot = lineFitInspectPlot(inspectSpaxel());
    expect(plot.samples.map((p) => p.channel)).toEqual([100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 112]);
    const state = Object.fromEntries(plot.samples.map((p) => [p.channel, p.state]));
    expect(state).toEqual({
      100: "used",
      101: "dropped",
      102: "used",
      103: "unused",
      104: "used",
      105: "used",
      106: "dropped",
      107: "used",
      108: "used",
      109: "unused",
      110: "used",
      112: "used",
    });
    const at = (channel: number) => plot.samples.find((p) => p.channel === channel);
    expect(at(106)).toEqual({ channel: 106, x: 4.0065, y: 1e6, err: 0.2, state: "dropped" });
    expect(at(101)?.err).toBeNull();
    expect(at(112)).toEqual({ channel: 112, x: 4.0125, y: 0.95, err: 0.1, state: "used" });
    expect(lineFitInspectPlot(inspectSpaxel({ err: null })).samples.every((p) => p.err === null)).toBe(true);
  });

  it("maps the model's fractional channels to x with the D19 rule and draws components on the continuum only for two components", () => {
    const plot = lineFitInspectPlot(inspectSpaxel());
    expect(plot.continuum).toHaveLength(49);
    expect(plot.total).toHaveLength(49);
    expect(plot.total[22].x).toBeCloseTo(4.00575, 12);
    expect(plot.total[22].y).toBeCloseTo(4.25, 12);
    expect(plot.total[48]).toEqual({ x: 4.0125, y: 1.75 });
    expect(plot.continuum[1].x).toBeCloseTo(4.00025, 12);
    expect(plot.components).toHaveLength(2);
    expect(plot.components[0][22].y).toBeCloseTo(4, 12);
    expect(plot.components[1][0]).toEqual({ x: 4.0, y: 1.25 });
    expect(lineFitInspectPlot(singleSpaxel()).components).toEqual([]);
    const gap = inspectSpaxel({
      model: { channel: MODEL_CHANNELS, continuum: CONTINUUM.map((c, k) => (k === 3 ? null : c)) as number[], total: CONTINUUM, components: [C1, C2] },
    });
    expect(lineFitInspectPlot(gap).continuum).toHaveLength(48);
    expect(lineFitInspectPlot(gap).components[0]).toHaveLength(48);
    const none = lineFitInspectPlot(inspectSpaxel({ model: null, ncomp: 0, single: null, components: [] }));
    expect(none.continuum).toEqual([]);
    expect(none.total).toEqual([]);
    expect(none.components).toEqual([]);
    expect(none.markers).toEqual([]);
    expect(none.samples).toHaveLength(12);
  });

  it("shades the line window and the continuum windows between half-channel boundaries, clamped to the span", () => {
    const plot = lineFitInspectPlot(inspectSpaxel());
    expect(plot.lineWindow[0]).toBeCloseTo(4.0035, 12);
    expect(plot.lineWindow[1]).toBeCloseTo(4.009, 12);
    expect(plot.continuumWindows[0][0]).toBe(4.0);
    expect(plot.continuumWindows[0][1]).toBeCloseTo(4.0025, 12);
    expect(plot.continuumWindows[1][0]).toBeCloseTo(4.01, 12);
    expect(plot.continuumWindows[1][1]).toBe(4.0125);
  });

  it("puts a centre marker per component labelled with the velocity of the response", () => {
    expect(lineFitInspectPlot(inspectSpaxel()).markers).toEqual([
      { x: 4.0052, label: "c1 -220.3 km/s", component: 0 },
      { x: 4.0081, label: "c2 15.2 km/s", component: 1 },
    ]);
    expect(lineFitInspectPlot(singleSpaxel()).markers).toEqual([{ x: 4.0055, label: "61.2 km/s", component: null }]);
    expect(lineFitInspectPlot(inspectSpaxel({ ncomp: 1, components: [], single: null, model: null })).markers).toEqual([]);
  });

  it("fits the y range to the used and out-of-window samples and the model, leaving dropped outliers out, with min/max ticks", () => {
    const plot = lineFitInspectPlot(inspectSpaxel());
    expect(plot.yTicks).toEqual([0.9, 4.25]);
    expect(plot.yDomain[0]).toBeLessThan(0.9);
    expect(plot.yDomain[1]).toBeGreaterThan(4.25);
    expect(plot.yDomain[1]).toBeLessThan(10);
    expect(plot.yTickLabels).toEqual(["0.90", "4.25"]);
    expect(lineFitInspectPlot(singleSpaxel()).yTicks).toEqual([0.9, 4]);
    const flat = lineFitInspectPlot(inspectSpaxel({ flux: AXIS.map(() => 2), model: null, ncomp: 0, single: null }));
    expect(flat.yTicks).toEqual([2, 2]);
    expect(flat.yDomain[0]).toBeLessThan(2);
    expect(flat.yDomain[1]).toBeGreaterThan(2);
  });

  it("gives 3 or 4 x ticks inside the axis range in the axis unit, for rising and falling axes", () => {
    const plot = lineFitInspectPlot(inspectSpaxel());
    expect(plot.unit).toBe("μm");
    expect(plot.xDomain).toEqual([4.0, 4.0125]);
    expect(plot.xTicks).toEqual([4.0, 4.005, 4.01]);
    expect(plot.xTickLabels).toEqual(["4.000", "4.005", "4.010"]);
    const ranges: [number, number, string][] = [
      [-310, 295, "km/s"],
      [230.552, 230.521, "GHz"],
      [1.87, 1.88, "um"],
      [0.05, 0.95, "um"],
      [2.6231, 2.6289, "um"],
    ];
    for (const [a, b, unit] of ranges) {
      const axis = Array.from({ length: 13 }, (_, k) => a + ((b - a) * k) / 12);
      const p = lineFitInspectPlot(inspectSpaxel({ axis, axis_unit: unit }));
      const [lo, hi] = p.xDomain;
      expect(lo).toBe(Math.min(...axis));
      expect(hi).toBe(Math.max(...axis));
      expect(p.xTicks.length).toBeGreaterThanOrEqual(3);
      expect(p.xTicks.length).toBeLessThanOrEqual(4);
      expect(p.xTicks.every((t, i) => t >= lo - 1e-12 && t <= hi + 1e-12 && (i === 0 || t > p.xTicks[i - 1]))).toBe(true);
      expect(new Set(p.xTickLabels).size).toBe(p.xTicks.length);
      expect(p.unit).toBe(unit === "um" ? "μm" : unit);
    }
  });

  it("summarises the plot as text for screen readers", () => {
    const two = inspectSpaxel();
    expect(lineFitInspectSummary(two, lineFitInspectPlot(two))).toBe(
      "Fitted spectrum of spaxel (18, 32), ch 100-112 (4.0000-4.0125 μm): 12 samples, 8 used, 2 dropped by DQ or ERR, 2 outside the windows; line window ch 104-108, continuum ch 100-102 and 110-112; continuum, total model and 2 components drawn; c1 -220.3 km/s, c2 15.2 km/s.",
    );
    const one = singleSpaxel();
    expect(lineFitInspectSummary(one, lineFitInspectPlot(one))).toContain("; continuum and total model drawn; centre 61.2 km/s.");
    const none = inspectSpaxel({ model: null, ncomp: 0, single: null, components: [] });
    expect(lineFitInspectSummary(none, lineFitInspectPlot(none))).toContain("; no model (no fit attempted or the fit did not converge).");
  });

  it("colours c1 blue and c2 red", () => {
    expect(COMPONENT_COLORS).toEqual(["rgba(96,165,250,0.95)", "rgba(248,113,113,0.95)"]);
  });
});
