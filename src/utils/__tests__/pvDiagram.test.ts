import { describe, it, expect } from "vitest";
import {
  DEFAULT_STEP_TEXT,
  DEFAULT_WIDTH_TEXT,
  PV_STEP_RANGE,
  PV_WIDTH_RANGE,
  isPvRecord,
  lineOf,
  offsetLabel,
  parseChannel,
  parseStep,
  parseWidth,
  pickLineRegion,
  pvCsvFileName,
  pvEffectiveMode,
  pvLogRecords,
  pvOnScreen,
  pvRecordLabel,
  pvRidgeCsv,
  pvRunBlocker,
  pvShiftKms,
  pvSpectralLabel,
  ridgeSeries,
} from "../pvDiagram";
import { CSV_LINE_END } from "../catalogCsv";
import type { Region } from "../../shared/types/regions";
import type { RadialVelocityCorrectionResult, SpectralAxisInfo } from "../../shared/types/spectral";
import type { PvDiagramResult, PvRun, PvRunParams } from "../../shared/types/pv";

const props = { color: null, width: null, text: null, dash: null, include: true };

function region(id: string, shape: Region["shape"]): Region {
  return { id, shape, props, backgroundId: null };
}

const circle = region("c1", { shape: "circle", x: 10, y: 10, r: 3 });
const lineA = region("l1", { shape: "line", x1: 2, y1: 16, x2: 29, y2: 16 });
const lineB = region("l2", { shape: "line", x1: 16, y1: 2, x2: 16, y2: 29 });

const waveAxis: SpectralAxisInfo = {
  kind: "wave",
  ctype: "WAVE",
  unit: "um",
  header_unit: "um",
  header_scale: 1,
  values: [1.0, 1.001, 1.002],
  crval: 1,
  cdelt: 0.001,
  crpix: 1,
  rest_wavelength_um: 1.02,
  rest_frequency_hz: null,
  specsys: "BARYCENT",
  velosys: null,
  notes: [],
};

const vradAxis: SpectralAxisInfo = { ...waveAxis, kind: "vrad", ctype: "VRAD", unit: "km/s", header_unit: "km/s" };

const unknownAxis: SpectralAxisInfo = { ...waveAxis, kind: "unknown", ctype: "FOO", unit: "", header_unit: "", rest_wavelength_um: null };

const correction: RadialVelocityCorrectionResult = {
  barycentric_kms: -7.25,
  heliocentric_kms: -7.2,
  jd_mid: 2459000.5,
  method: "test",
  accuracy_kms: 0.02,
  notes: [],
  ra_deg: 10,
  dec_deg: -20,
  coordinate_source: "header",
};

function result(overrides: Partial<PvDiagramResult> = {}): PvDiagramResult {
  return {
    fits_path: "C:/out/cube_pv_0-39_2-16_29-16.fits",
    png_path: "C:/out/cube_pv_0-39_2-16_29-16.png",
    dimensions: [3, 40],
    offsets: [-0.36, 0, 0.36],
    offset_unit: "arcsec",
    offset_step: 0.36,
    spectral_values: [-100, 0, 100],
    spectral_unit: "km/s",
    spectral_mode: "velocity",
    spectral_shift_kms: -7.25,
    spectral_rest_um: 1.02,
    spectral_convention: "optical",
    ridge: [-50, null, 50],
    ridge_channel: [19.5, null, 20.5],
    peak_channel: [19, null, 21],
    peak_value: [1.5, null, 1.4],
    peak_spectral: [-50, null, 50],
    xs: [2, 3, 4],
    ys: [16, 16, 16],
    line: { x0: 2, y0: 16, x1: 29, y1: 16 },
    step_px: 1,
    width_px: 1,
    z0: 0,
    z1: 39,
    n_across: 1,
    summary: {
      n_offsets: 3,
      n_channels: 40,
      n_across: 1,
      n_off_image: 0,
      offset_step: 0.36,
      offset_unit: "arcsec",
      slit_length_px: 27,
      slit_length: 9.72,
      slit_pa_deg: 90,
      pixel_scale_arcsec: 0.36,
      ridge_gradient: 138.9,
      ridge_gradient_unit: "km/s/arcsec",
      ridge_span: 100,
      n_ridge_valid: 2,
      peak_value: 1.5,
      peak_offset: -0.36,
      peak_spectral: -50,
      bunit: "Jy/beam",
    },
    notes: ["a", "b"],
    elapsed_ms: 12,
    ...overrides,
  };
}

function params(overrides: Partial<PvRunParams> = {}): PvRunParams {
  return {
    filePath: "C:/d/cube.fits",
    regionId: "l1",
    line: { x0: 2, y0: 16, x1: 29, y1: 16 },
    stepPx: 1,
    widthPx: 1,
    z0: 0,
    z1: 39,
    mode: "velocity",
    restUm: 1.02,
    convention: "optical",
    correction: "barycentric",
    velocityShiftKms: -7.25,
    ...overrides,
  };
}

function run(p: Partial<PvRunParams> = {}, r: Partial<PvDiagramResult> = {}): PvRun {
  return { params: params(p), result: result(r) };
}

describe("pickLineRegion", () => {
  it("prefers the selected line over an earlier line", () => {
    expect(pickLineRegion([lineA, lineB], "l2")).toBe(lineB);
  });

  it("falls back to the first line when the selection is a circle", () => {
    expect(pickLineRegion([circle, lineA, lineB], "c1")).toBe(lineA);
    expect(pickLineRegion([circle, lineA, lineB], null)).toBe(lineA);
  });

  it("returns null without a line region", () => {
    expect(pickLineRegion([circle], "c1")).toBeNull();
    expect(pickLineRegion([], null)).toBeNull();
  });
});

describe("lineOf", () => {
  it("maps x1,y1,x2,y2 to x0,y0,x1,y1", () => {
    expect(lineOf(lineA)).toEqual({ x0: 2, y0: 16, x1: 29, y1: 16 });
  });

  it("returns null for a non-line region", () => {
    expect(lineOf(circle)).toBeNull();
  });
});

describe("parsers", () => {
  it("accept the step bounds and reject outside or NaN", () => {
    expect(parseStep(String(PV_STEP_RANGE[0]))).toBe(0.05);
    expect(parseStep(String(PV_STEP_RANGE[1]))).toBe(1024);
    expect(parseStep(DEFAULT_STEP_TEXT)).toBe(1);
    expect(parseStep("0.04")).toBeNull();
    expect(parseStep("1025")).toBeNull();
    expect(parseStep("abc")).toBeNull();
    expect(parseStep("")).toBeNull();
  });

  it("accept the width bounds including zero", () => {
    expect(parseWidth(String(PV_WIDTH_RANGE[0]))).toBe(0);
    expect(parseWidth(String(PV_WIDTH_RANGE[1]))).toBe(4096);
    expect(parseWidth(DEFAULT_WIDTH_TEXT)).toBe(1);
    expect(parseWidth("-1")).toBeNull();
    expect(parseWidth("4097")).toBeNull();
    expect(parseWidth("NaN")).toBeNull();
  });

  it("accept integer channels inside the cube only", () => {
    expect(parseChannel("0", 40)).toBe(0);
    expect(parseChannel("39", 40)).toBe(39);
    expect(parseChannel("40", 40)).toBeNull();
    expect(parseChannel("-1", 40)).toBeNull();
    expect(parseChannel("2.5", 40)).toBeNull();
    expect(parseChannel("x", 40)).toBeNull();
    expect(parseChannel("0", 0)).toBeNull();
  });
});

describe("pvRunBlocker", () => {
  const valid = {
    line: { x0: 2, y0: 16, x1: 29, y1: 16 },
    step: 1,
    width: 1,
    z0: 0,
    z1: 39,
    mode: "velocity" as const,
    restUm: 1.02,
    axis: waveAxis,
  };

  it("returns null for a valid input", () => {
    expect(pvRunBlocker(valid)).toBeNull();
    expect(pvRunBlocker({ ...valid, mode: "wavelength_vac", restUm: null })).toBeNull();
    expect(pvRunBlocker({ ...valid, restUm: null, axis: vradAxis })).toBeNull();
    expect(pvRunBlocker({ ...valid, restUm: null, axis: null, mode: "wavelength_vac" })).toBeNull();
  });

  it("returns each reason in priority order", () => {
    expect(pvRunBlocker({ ...valid, line: null, step: null })).toBe("draw a line region on the cube");
    expect(pvRunBlocker({ ...valid, step: null, width: null })).toBe("step must be 0.05 to 1024 px");
    expect(pvRunBlocker({ ...valid, width: null, z0: null })).toBe("width must be 0 to 4096 px");
    expect(pvRunBlocker({ ...valid, z0: null })).toBe("channel range must be two channels of the cube in order");
    expect(pvRunBlocker({ ...valid, z1: null })).toBe("channel range must be two channels of the cube in order");
    expect(pvRunBlocker({ ...valid, z0: 10, z1: 10 })).toBe("channel range must be two channels of the cube in order");
    expect(pvRunBlocker({ ...valid, z0: 11, z1: 10 })).toBe("channel range must be two channels of the cube in order");
    expect(pvRunBlocker({ ...valid, restUm: null })).toBe("velocity mode needs a rest wavelength");
    expect(pvRunBlocker({ ...valid, line: { x0: 3, y0: 3, x1: 3, y1: 3 } })).toBe("line has zero length");
  });
});

describe("pvEffectiveMode", () => {
  it("falls back to the channel-index mode when the cube offers no spectral mode", () => {
    expect(pvEffectiveMode(null, "velocity")).toBe("wavelength_vac");
    expect(pvEffectiveMode(unknownAxis, "velocity")).toBe("wavelength_vac");
    expect(pvEffectiveMode(null, "frequency")).toBe("wavelength_vac");
    expect(pvEffectiveMode(unknownAxis, "wavelength_air")).toBe("wavelength_vac");
  });

  it("keeps an offered mode and forces the only mode of a velocity-kind axis", () => {
    expect(pvEffectiveMode(waveAxis, "velocity")).toBe("velocity");
    expect(pvEffectiveMode(waveAxis, "frequency")).toBe("frequency");
    expect(pvEffectiveMode(vradAxis, "velocity")).toBe("velocity");
    expect(pvEffectiveMode(vradAxis, "wavelength_vac")).toBe("velocity");
  });

  it("never sends velocity mode to a cube without a spectral axis", () => {
    for (const mode of ["wavelength_vac", "wavelength_air", "frequency", "velocity"] as const) {
      expect(pvEffectiveMode(null, mode)).not.toBe("velocity");
      expect(pvEffectiveMode(unknownAxis, mode)).not.toBe("velocity");
    }
  });
});

describe("pvShiftKms", () => {
  it("returns the frame value only in velocity mode", () => {
    expect(pvShiftKms("barycentric", correction, "velocity")).toBe(-7.25);
    expect(pvShiftKms("heliocentric", correction, "velocity")).toBe(-7.2);
    expect(pvShiftKms("none", correction, "velocity")).toBeNull();
    expect(pvShiftKms("barycentric", null, "velocity")).toBeNull();
    expect(pvShiftKms("barycentric", correction, "wavelength_vac")).toBeNull();
  });
});

describe("pvSpectralLabel", () => {
  it("labels the four modes", () => {
    expect(pvSpectralLabel(run({ mode: "wavelength_vac" }, { spectral_mode: "wavelength_vac", spectral_unit: "um", spectral_shift_kms: 0 }))).toBe(
      "Vacuum wavelength (μm)",
    );
    expect(pvSpectralLabel(run({ mode: "wavelength_air" }, { spectral_mode: "wavelength_air", spectral_unit: "um", spectral_shift_kms: 0 }))).toBe(
      "Air wavelength (μm)",
    );
    expect(pvSpectralLabel(run({ mode: "frequency" }, { spectral_mode: "frequency", spectral_unit: "GHz", spectral_shift_kms: 0 }))).toBe(
      "Frequency (GHz)",
    );
    expect(pvSpectralLabel(run({ correction: "none" }, { spectral_shift_kms: 0 }))).toBe("Velocity, optical (km/s)");
  });

  it("labels the channel fallback", () => {
    expect(pvSpectralLabel(run({ mode: "wavelength_vac" }, { spectral_mode: "wavelength_vac", spectral_unit: "ch" }))).toBe("Channel");
  });

  it("names the actual correction frame only with a non-zero shift", () => {
    expect(pvSpectralLabel(run({ correction: "barycentric" }, { spectral_shift_kms: -7.25 }))).toBe("Velocity, optical (km/s), barycentric");
    expect(
      pvSpectralLabel(run({ correction: "heliocentric", convention: "radio" }, { spectral_shift_kms: -7.2, spectral_convention: "radio" })),
    ).toBe("Velocity, radio (km/s), heliocentric");
    expect(pvSpectralLabel(run({ correction: "barycentric" }, { spectral_shift_kms: 0 }))).toBe("Velocity, optical (km/s)");
    expect(pvSpectralLabel(run({ correction: "none" }, { spectral_shift_kms: -7.25 }))).toBe("Velocity, optical (km/s)");
  });

  it("drops the convention on a velocity-kind axis", () => {
    expect(pvSpectralLabel(run({ correction: "none" }, { spectral_convention: null, spectral_shift_kms: 0 }))).toBe("Velocity (km/s)");
    expect(pvSpectralLabel(run({ correction: "barycentric" }, { spectral_convention: null, spectral_shift_kms: -7.25 }))).toBe(
      "Velocity (km/s), barycentric",
    );
  });
});

describe("offsetLabel and pvRecordLabel", () => {
  it("names the offset unit", () => {
    expect(offsetLabel("arcsec")).toBe("offset (arcsec)");
    expect(offsetLabel("pixel")).toBe("offset (px)");
  });

  it("labels the published record", () => {
    expect(pvRecordLabel(params({ stepPx: 0.5, widthPx: 3 }), result({ z0: 5, z1: 35 }))).toBe("PV · ch 5-35 · step 0.5 px · width 3 px");
  });
});

describe("ridgeSeries", () => {
  it("plots the ridge as a line and the peaks as points over the offsets", () => {
    const series = ridgeSeries(result());
    expect(series).toHaveLength(2);
    expect(series[0].x).toEqual([-0.36, 0, 0.36]);
    expect(series[0].y).toEqual([-50, null, 50]);
    expect(series[0].label).toBe("ridge");
    expect(series[0].mode).toBeUndefined();
    expect(series[1].x).toEqual([-0.36, 0, 0.36]);
    expect(series[1].y).toEqual([-50, null, 50]);
    expect(series[1].label).toBe("peak");
    expect(series[1].mode).toBe("points");
    expect(series[0].color).not.toBe(series[1].color);
  });
});

describe("pvRidgeCsv", () => {
  it("writes six provenance lines, the unit-tagged header and empty null cells", () => {
    const csv = pvRidgeCsv(run());
    const lines = csv.split(CSV_LINE_END);
    expect(lines.slice(0, 6).every((l) => l.startsWith("# "))).toBe(true);
    expect(lines[0]).toBe("# file C:/d/cube.fits");
    expect(lines[1]).toBe("# slit 2,16 -> 29,16 (px, 0-based)");
    expect(lines[2]).toBe("# step_px 1, width_px 1, n_across 1");
    expect(lines[3]).toBe("# channels 0-39");
    expect(lines[4]).toBe("# spectral axis: Velocity, optical (km/s), barycentric");
    expect(lines[5]).toBe("# fits: C:/out/cube_pv_0-39_2-16_29-16.fits");
    expect(lines[6]).toBe("offset_arcsec,ridge_km_s,ridge_channel,peak_channel,peak_km_s,peak_value");
    expect(lines[7]).toBe("-0.36,-50,19.5,19,-50,1.5");
    expect(lines[8]).toBe("0,,,,,");
    expect(lines[9]).toBe("0.36,50,20.5,21,50,1.4");
    expect(csv.endsWith(CSV_LINE_END)).toBe(true);
    expect(csv).not.toContain("\n\n");
  });

  it("tags pixel offsets and channel units", () => {
    const csv = pvRidgeCsv(run({ mode: "wavelength_vac" }, { offset_unit: "pixel", spectral_unit: "ch", spectral_mode: "wavelength_vac" }));
    expect(csv.split(CSV_LINE_END)[6]).toBe("offset_px,ridge_ch,ridge_channel,peak_channel,peak_ch,peak_value");
  });
});

describe("pvCsvFileName", () => {
  it("keeps the plane tag and falls back to output", () => {
    expect(pvCsvFileName("C:/d/cube.fits#hdu=1")).toBe("cube_hdu1_pv.csv");
    expect(pvCsvFileName("C:/d/cube.fits")).toBe("cube_pv.csv");
    expect(pvCsvFileName(null)).toBe("output_pv.csv");
  });
});

describe("pvOnScreen", () => {
  it("is true only when the displayed record is the run's FITS", () => {
    const r = run();
    expect(pvOnScreen({ fitsPath: r.result.fits_path }, r)).toBe(true);
    expect(pvOnScreen({ fitsPath: "C:/out/other.fits" }, r)).toBe(false);
    expect(pvOnScreen(null, r)).toBe(false);
    expect(pvOnScreen({ fitsPath: r.result.fits_path }, null)).toBe(false);
  });
});

describe("isPvRecord", () => {
  it("recognises the record the panel publishes from its label alone, without a run", () => {
    const r = run();
    expect(isPvRecord({ label: pvRecordLabel(r.params, r.result) })).toBe(true);
    expect(isPvRecord({ label: "PV · ch 0-39 · step 1 px · width 1 px" })).toBe(true);
    expect(isPvRecord({ label: pvRecordLabel(params({ stepPx: 0.5, widthPx: 3 }), result({ z0: 5, z1: 35 })) })).toBe(true);
  });

  it("is false for no record and for the other cube records", () => {
    expect(isPvRecord(null)).toBe(false);
    expect(isPvRecord({ label: "" })).toBe(false);
    expect(isPvRecord({ label: "Collapse mean · ch 0-39" })).toBe(false);
    expect(isPvRecord({ label: "Moment 1 · 40 ch" })).toBe(false);
    expect(isPvRecord({ label: "pv" })).toBe(false);
    expect(isPvRecord({ label: "Debayer" })).toBe(false);
  });
});

describe("pvLogRecords", () => {
  it("flattens the parameters and the summary numbers", () => {
    const records = pvLogRecords(run());
    expect(records.unit).toBe("Jy/beam");
    expect(records.params).toEqual({
      region_id: "l1",
      x0: 2,
      y0: 16,
      x1: 29,
      y1: 16,
      step_px: 1,
      width_px: 1,
      z0: 0,
      z1: 39,
      mode: "velocity",
      rest_um: 1.02,
      convention: "optical",
      correction: "barycentric",
      shift_kms: -7.25,
    });
    expect(records.values).toEqual({
      ridge_gradient: 138.9,
      ridge_gradient_unit: "km/s/arcsec",
      ridge_span: 100,
      n_ridge_valid: 2,
      peak_value: 1.5,
      peak_offset: -0.36,
      peak_spectral: -50,
      slit_length: 9.72,
      slit_pa_deg: 90,
      offset_step: 0.36,
    });
  });

  it("keeps nulls and a missing unit", () => {
    const records = pvLogRecords(
      run(
        { restUm: null, velocityShiftKms: null },
        { summary: { ...result().summary, ridge_gradient: null, slit_pa_deg: null, bunit: null } },
      ),
    );
    expect(records.unit).toBeNull();
    expect(records.params.rest_um).toBeNull();
    expect(records.params.shift_kms).toBeNull();
    expect(records.values.ridge_gradient).toBeNull();
    expect(records.values.slit_pa_deg).toBeNull();
  });
});
